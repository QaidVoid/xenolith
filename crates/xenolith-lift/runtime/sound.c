/* The sound a title makes, and what this does with it.
 *
 * A title does not open a device. It mixes into a ring of frames and hands them
 * over, and a title that streams a compressed sound has the console's own
 * decoder do the work. Neither of those is a device: what a title sees is a
 * queue it writes into and a count it paces itself by.
 *
 * So this keeps the queue and the count, and reaches a host device where there
 * is one. Where there is not, it says so once and carries on without sound,
 * because a run with no sound in it is worth more than a run that stops at the
 * first frame a title hands over.
 *
 * This file is included by xenolith.c rather than compiled on its own: it needs
 * the guest memory accessors and the report lock, and those live there.
 */

/* One frame is two hundred and fifty six samples across six channels, and what
 * goes to a device is two channels interleaved, folded down the way a player
 * with two speakers hears six: the front pair as they are, the centre in both
 * lessened, the rears in their own side lessened more, and the low effects
 * nowhere because a player with two speakers has nowhere to put them. */
#define XENOLITH_SAMPLES_PER_FRAME 256u
#define XENOLITH_FRAME_CHANNELS 6u
#define XENOLITH_DEVICE_CHANNELS 2u

/* How many frames may wait to be played. Enough for a title to run a little
 * ahead of the device without the device running dry, and no more, because a
 * frame dropped for a full queue is sound a title mixed that is never heard and
 * the fewer of those the better. */
#define XENOLITH_QUEUED_FRAMES 16u

/* What the host's sound library is asked for, by the numbers its own headers
 * give them. They are written here rather than included, because reaching for
 * the library at run time keeps this clear of depending on it at build time. */
typedef int (*xenolith_pcm_open)(void **, const char *, int, int);
typedef int (*xenolith_pcm_set_params)(void *, int, int, unsigned, unsigned, int, unsigned);
typedef long (*xenolith_pcm_writei)(void *, const void *, unsigned long);
typedef int (*xenolith_pcm_recover)(void *, int, int);
typedef int (*xenolith_pcm_close)(void *);

static struct {
    void *library;
    void *device;
    xenolith_pcm_open open;
    xenolith_pcm_set_params set_params;
    xenolith_pcm_writei writei;
    xenolith_pcm_recover recover;
    xenolith_pcm_close close;
    pthread_mutex_t handing;
    float queued[XENOLITH_QUEUED_FRAMES][XENOLITH_SAMPLES_PER_FRAME * XENOLITH_DEVICE_CHANNELS];
    unsigned reading;
    unsigned writing;
    unsigned long played;
    unsigned long silent;
    unsigned long dropped;
    int refused;
} output = {.handing = PTHREAD_MUTEX_INITIALIZER};

static void start_the_output(void);

/* Opens the host's sound, or says once why there is none.
 *
 * One line, naming the library or the device that was not there, is the whole
 * of the failure: a run continues without sound, and saying it more than once
 * would say nothing more. */
static int open_the_output(void) {
    static const int playback = 0;
    static const int interleaved = 3;
    static const int floats_of_the_host = 14;

    if (output.device != NULL) {
        return 1;
    }
    if (output.refused) {
        return 0;
    }
    if (output.library == NULL) {
        output.library = dlopen("libasound.so.2", RTLD_NOW);
        if (output.library == NULL) {
            output.library = dlopen("libasound.so", RTLD_NOW);
        }
        if (output.library == NULL) {
            say_a_report("no sound will be heard: libasound is not on this host");
            output.refused = 1;
            return 0;
        }
        output.open = (xenolith_pcm_open)dlsym(output.library, "snd_pcm_open");
        output.set_params = (xenolith_pcm_set_params)dlsym(output.library, "snd_pcm_set_params");
        output.writei = (xenolith_pcm_writei)dlsym(output.library, "snd_pcm_writei");
        output.recover = (xenolith_pcm_recover)dlsym(output.library, "snd_pcm_recover");
        output.close = (xenolith_pcm_close)dlsym(output.library, "snd_pcm_close");
        if (output.open == NULL || output.set_params == NULL || output.writei == NULL ||
            output.recover == NULL || output.close == NULL) {
            say_a_report("no sound will be heard: libasound is missing a way to write "
                         "to a device");
            output.refused = 1;
            return 0;
        }
    }
    if (output.open(&output.device, "default", playback, 0) != 0) {
        say_a_report("no sound will be heard: the host has no device to play on");
        output.refused = 1;
        return 0;
    }
    if (output.set_params(output.device, floats_of_the_host, interleaved,
                          XENOLITH_DEVICE_CHANNELS, 48000u, 1, 40000u) != 0) {
        say_a_report("no sound will be heard: the host's device would not take two "
                     "channels at eight and forty thousand");
        output.close(output.device);
        output.device = NULL;
        output.refused = 1;
        return 0;
    }
    return 1;
}

/* Says how far playback has got, when asked to trace it. */
static void say_how_playback_is_going(void) {
    static int telling = -1;
    static unsigned long said;

    if (telling < 0) {
        telling = getenv("XENOLITH_TRACE_SOUND") != NULL;
    }
    if (!telling) {
        return;
    }
    pthread_mutex_lock(&output.handing);
    unsigned long frames = output.played;
    pthread_mutex_unlock(&output.handing);
    if (frames / 512u != said / 512u) {
        said = frames;
        say_a_report("the device has played %lu frames, %lu of them silence made up, "
                     "%lu dropped",
                     frames, output.silent, output.dropped);
    }
}

/* Brings one sample of a title's into the range a device takes.
 *
 * A sample that is not a number, or out of either end, is not sound, and the
 * nearest thing to it that is: nothing, or the end it went past. */
static float bring_a_sample_in(float given) {
    if (given != given) {
        return 0.0f;
    }
    if (given > 1.0f) {
        return 1.0f;
    }
    if (given < -1.0f) {
        return -1.0f;
    }
    return given;
}

/* Reads one of a title's samples out of the guest.
 *
 * A sample is a float kept the other way round from the host's, because the
 * console keeps everything that way. Read as it stands it is a number no device
 * can play, which is silence at best and a crackle at worst. */
static float read_a_guest_sample(const uint8_t *base, uint32_t at) {
    uint32_t word = xenolith_load32(base, at);
    float given;
    word = __builtin_bswap32(word);
    memcpy(&given, &word, sizeof given);
    return bring_a_sample_in(given);
}

/* Puts a frame a title mixed where the device will find it.
 *
 * The frame is folded from six channels to two as it is read, and dropped whole
 * if the queue is full, which is sound arriving too late to be worth hearing
 * rather than sound worth delaying everything else for. */
static void hand_a_frame_over(const uint8_t *base, uint32_t at) {
    static const float centre = 0.70710678f;
    static const float surround = 0.5f;
    static const float everywhere = 1.0f / (1.0f + 0.70710678f + 0.5f);

    start_the_output();
    if (output.device == NULL) {
        return;
    }

    pthread_mutex_lock(&output.handing);
    if (output.writing - output.reading >= XENOLITH_QUEUED_FRAMES) {
        output.dropped++;
        pthread_mutex_unlock(&output.handing);
        return;
    }
    float *into = output.queued[output.writing % XENOLITH_QUEUED_FRAMES];
    for (unsigned sample = 0; sample < XENOLITH_SAMPLES_PER_FRAME; sample++) {
        float front_left = read_a_guest_sample(
            base, at + (0u * XENOLITH_SAMPLES_PER_FRAME + sample) * 4u);
        float front_right = read_a_guest_sample(
            base, at + (1u * XENOLITH_SAMPLES_PER_FRAME + sample) * 4u);
        float middle = read_a_guest_sample(
            base, at + (2u * XENOLITH_SAMPLES_PER_FRAME + sample) * 4u);
        float back_left = read_a_guest_sample(
            base, at + (4u * XENOLITH_SAMPLES_PER_FRAME + sample) * 4u);
        float back_right = read_a_guest_sample(
            base, at + (5u * XENOLITH_SAMPLES_PER_FRAME + sample) * 4u);
        float left = (front_left + middle * centre + back_left * surround) * everywhere;
        float right = (front_right + middle * centre + back_right * surround) * everywhere;

        /* A frame that folds past the device's end is eased back under rather
         * than clipped at the edge, which a loud passage would crackle at every
         * frame the peak crossed. */
        float peak = left >= 0.0f ? left : -left;
        float right_peak = right >= 0.0f ? right : -right;
        if (right_peak > peak) {
            peak = right_peak;
        }
        if (peak > 0.92f) {
            float scale = 0.92f / peak;
            left *= scale;
            right *= scale;
        }
        into[sample * 2u] = left;
        into[sample * 2u + 1u] = right;
    }
    output.writing++;
    pthread_mutex_unlock(&output.handing);
}

/* Takes one frame out of the queue, or makes up silence where there is none.
 *
 * Silence is counted like any other frame, because the device consumed it and a
 * title pacing by the count should not stand still for a stretch it filled with
 * nothing. */
static void take_a_frame_out(float *into, int *quiet) {
    pthread_mutex_lock(&output.handing);
    if (output.reading == output.writing) {
        *quiet = 1;
        memset(into, 0,
               sizeof(float) * XENOLITH_SAMPLES_PER_FRAME * XENOLITH_DEVICE_CHANNELS);
    } else {
        *quiet = 0;
        memcpy(into, output.queued[output.reading % XENOLITH_QUEUED_FRAMES],
               sizeof(float) * XENOLITH_SAMPLES_PER_FRAME * XENOLITH_DEVICE_CHANNELS);
        output.reading++;
    }
    pthread_mutex_unlock(&output.handing);
}

/* How far playback has got, in samples the device has taken.
 *
 * A title reads this to pace what it shows against what is heard, and the
 * console counted samples the hardware consumed, silence the hardware made up
 * included. So this is the count of frames played, silence and all. */
static uint32_t how_far_playback_has_got(void) {
    pthread_mutex_lock(&output.handing);
    unsigned long samples = output.played;
    pthread_mutex_unlock(&output.handing);
    if (samples > 0xfffffffful) {
        return 0xffffffffu;
    }
    return (uint32_t)samples;
}

/* Feeds the device one frame at a time for as long as the run lasts. */
static void *keep_the_device_fed(void *given) {
    (void)given;
    for (;;) {
        float frame[XENOLITH_SAMPLES_PER_FRAME * XENOLITH_DEVICE_CHANNELS];
        int quiet = 0;

        take_a_frame_out(frame, &quiet);

        long written = output.writei(output.device, frame, XENOLITH_SAMPLES_PER_FRAME);
        if (written == -32) {
            /* The device ran dry, which the library calls EPIPE. */
            output.recover(output.device, -32, 1);
            written = output.writei(output.device, frame, XENOLITH_SAMPLES_PER_FRAME);
        }
        if (written > 0) {
            pthread_mutex_lock(&output.handing);
            output.played += (unsigned long)written;
            if (quiet) {
                output.silent += (unsigned long)written;
            }
            pthread_mutex_unlock(&output.handing);
            say_how_playback_is_going();
        }
    }
    return NULL;
}

/* Starts the device the first time a title hands a frame over.
 *
 * Nothing before that moment can be played, so a run that never mixes opens no
 * device and prints nothing, which is a run with no sound in it rather than a
 * run that failed to find any. */
static void start_the_output(void) {
    static int started;

    if (started || !open_the_output()) {
        return;
    }
    started = 1;
    pthread_t feeding;
    if (pthread_create(&feeding, NULL, keep_the_device_fed, NULL) == 0) {
        pthread_detach(feeding);
    }
}

