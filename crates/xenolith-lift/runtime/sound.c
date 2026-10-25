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

/* The same for a desktop's sound server, which holds the device itself where
 * there is one, so that the device cannot be opened past it. */
struct xenolith_pulse_format {
    int format;
    uint32_t rate;
    uint8_t channels;
};
struct xenolith_pulse_buffering {
    uint32_t most;
    uint32_t aimed_at;
    uint32_t before_starting;
    uint32_t least_asked;
    uint32_t taken_at_once;
};
typedef void *(*xenolith_pulse_new)(const char *, const char *, int, const char *, const char *,
                                    const struct xenolith_pulse_format *, const void *,
                                    const struct xenolith_pulse_buffering *, int *);
typedef int (*xenolith_pulse_write)(void *, const void *, size_t, int *);
typedef void (*xenolith_pulse_free)(void *);

static struct {
    void *library;
    void *device;
    xenolith_pcm_open open;
    xenolith_pcm_set_params set_params;
    xenolith_pcm_writei writei;
    xenolith_pcm_recover recover;
    xenolith_pcm_close close;
    void *server_library;
    void *server;
    xenolith_pulse_new server_new;
    xenolith_pulse_write server_write;
    xenolith_pulse_free server_free;
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

/* Reaches the desktop's sound server, once, for the ways of making, writing to
 * and letting go of a stream. */
static int reach_the_sound_server(void) {
    if (output.server_library != NULL) {
        return output.server_new != NULL && output.server_write != NULL &&
               output.server_free != NULL;
    }
    output.server_library = dlopen("libpulse-simple.so.0", RTLD_NOW);
    if (output.server_library == NULL) {
        return 0;
    }
    output.server_new = (xenolith_pulse_new)dlsym(output.server_library, "pa_simple_new");
    output.server_write = (xenolith_pulse_write)dlsym(output.server_library, "pa_simple_write");
    output.server_free = (xenolith_pulse_free)dlsym(output.server_library, "pa_simple_free");
    return output.server_new != NULL && output.server_write != NULL &&
           output.server_free != NULL;
}

/* Makes a stream on the desktop's sound server, asked to keep about two frames'
 * worth ahead of what is heard, so that what a title mixes is heard soon after
 * it is mixed.
 *
 * A stream is made rather than kept, because one that has failed is a dead
 * connection and no amount of writing to it brings it back: a run mends its
 * sound by making another where it stood. */
static int make_the_sound_server(void) {
    static const int float_little_end_first = 5;
    static const int playback = 1;

    if (!reach_the_sound_server()) {
        return 0;
    }
    struct xenolith_pulse_format format = {float_little_end_first, 48000u,
                                           XENOLITH_DEVICE_CHANNELS};
    uint32_t a_frame = XENOLITH_SAMPLES_PER_FRAME * XENOLITH_DEVICE_CHANNELS * sizeof(float);
    struct xenolith_pulse_buffering buffering = {0xffffffffu, a_frame * 8u, 0xffffffffu,
                                                 0xffffffffu, 0xffffffffu};
    int why = 0;
    output.server = output.server_new(NULL, "xenolith", playback, NULL, "title", &format, NULL,
                                      &buffering, &why);
    return output.server != NULL;
}

/* Lets a stream go, so that another can be made where it stood. */
static void close_the_sound_server(void) {
    if (output.server != NULL && output.server_free != NULL) {
        output.server_free(output.server);
    }
    output.server = NULL;
}

/* Opens the host's sound, or says once why there is none.
 *
 * One line, naming the library or the device that was not there, is the whole
 * of the failure: a run continues without sound, and saying it more than once
 * would say nothing more. */
static int open_the_output(void) {
    static const int playback = 0;
    static const int interleaved = 3;
    static const int floats_of_the_host = 14;

    if (output.device != NULL || output.server != NULL) {
        return 1;
    }
    if (output.refused) {
        return 0;
    }
    if (make_the_sound_server()) {
        return 1;
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

/* Reads one of a title's samples out of the guest: a float in the console's
 * byte order, which the guest word accessor already turns into the host's. */
static float read_a_guest_sample(const uint8_t *base, uint32_t at) {
    uint32_t word = xenolith_load32(base, at);
    float given;
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
    if (output.device == NULL && output.server == NULL) {
        return;
    }

    pthread_mutex_lock(&output.handing);
    if (output.writing - output.reading >= XENOLITH_QUEUED_FRAMES) {
        output.dropped++;
        pthread_mutex_unlock(&output.handing);
        /* Said rather than counted in silence. A frame dropped here is sound a
         * title mixed that nobody hears, and a title that paces itself by the
         * count goes on mixing into a queue that is not draining. One line is
         * the difference between a run with a stutter in it and a run with no
         * explanation for the stutter, and saying it per frame would bury the
         * explanation under thousands of copies of itself. */
        static int said;
        if (!said) {
            said = 1;
            say_a_report("a frame of sound was dropped: the queue holds %u and the "
                         "device is not taking them",
                         XENOLITH_QUEUED_FRAMES);
        }
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

/* Feeds the device one frame at a time for as long as the run lasts.
 *
 * A stream that fails is mended rather than mourned: the server's is let go
 * and another made, the device's is recovered or closed and opened again, and
 * a frame that cannot be written anywhere is waited out rather than spun on. */
static void *keep_the_device_fed(void *given) {
    (void)given;
    for (;;) {
        float frame[XENOLITH_SAMPLES_PER_FRAME * XENOLITH_DEVICE_CHANNELS];
        int quiet = 0;

        take_a_frame_out(frame, &quiet);

        long written = -1;
        if (output.server != NULL) {
            int why = 0;
            written = output.server_write(output.server, frame, sizeof frame, &why) == 0
                          ? (long)XENOLITH_SAMPLES_PER_FRAME
                          : -1;
            if (written < 0) {
                close_the_sound_server();
                if (make_the_sound_server()) {
                    why = 0;
                    written =
                        output.server_write(output.server, frame, sizeof frame, &why) == 0
                            ? (long)XENOLITH_SAMPLES_PER_FRAME
                            : -1;
                }
            }
        } else if (output.device != NULL) {
            written = output.writei(output.device, frame, XENOLITH_SAMPLES_PER_FRAME);
            if (written < 0 && output.recover(output.device, (int)written, 1) == 0) {
                written = output.writei(output.device, frame, XENOLITH_SAMPLES_PER_FRAME);
            }
            if (written < 0) {
                output.close(output.device);
                output.device = NULL;
                open_the_output();
                if (output.device != NULL) {
                    written = output.writei(output.device, frame, XENOLITH_SAMPLES_PER_FRAME);
                }
            }
        }
        if (written < 0) {
            struct timespec a_frame = {0, 5333333L};
            nanosleep(&a_frame, NULL);
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


/* Compressed sound, decoded the way the console's decoder would.
 *
 * A title fills in a context for each compressed sound it plays: two input
 * buffers of packets two kilobytes long, a ring of blocks for the decoded sound,
 * and offsets into each that the decoder and the title move between them. The
 * decoder reads packets from whichever input buffer is current, puts sixteen bit
 * samples into the ring for as long as there is room, marks a buffer it has
 * finished as no longer valid, and moves on to the other.
 *
 * The title starts a context by writing to the decoder's registers, which are
 * ordinary memory here, so a write to them cannot be seen. What a context says
 * is what is decoded instead: one with somewhere to put sound and a buffer to
 * read is one the title wants decoded.
 *
 * The decoding is the host's own libavcodec, reached at run time for the same
 * reason the device is. Its structures are read at the places its own headers
 * give them, which have stayed put across the versions reached for. */
typedef void *(*xenolith_av_find)(const char *);
typedef void *(*xenolith_av_alloc_context)(const void *);
typedef void *(*xenolith_av_alloc)(void);
typedef void (*xenolith_av_free)(void **);
typedef int (*xenolith_av_to_context)(void *, const void *);
typedef int (*xenolith_av_open)(void *, const void *, void **);
typedef int (*xenolith_av_send)(void *, const void *);
typedef int (*xenolith_av_receive)(void *, void *);
typedef void (*xenolith_av_flush)(void *);

#define XENOLITH_PARAMETERS_KIND 0u
#define XENOLITH_PARAMETERS_EXTRA 16u
#define XENOLITH_PARAMETERS_EXTRA_SIZE 24u
#define XENOLITH_PARAMETERS_CHANNELS 112u
#define XENOLITH_PARAMETERS_RATE 116u
#define XENOLITH_PARAMETERS_BLOCK 120u
#define XENOLITH_PACKET_DATA 24u
#define XENOLITH_PACKET_SIZE 32u
#define XENOLITH_FRAME_SAMPLES 112u
#define XENOLITH_FRAME_FORMAT 116u
#define XENOLITH_AUDIO 1
#define XENOLITH_FLOATS_APART 8

/* What a packet is, and how much room a decoder asks to be left after one. */
#define XENOLITH_PACKET_BYTES 2048u
#define XENOLITH_PACKET_HEADER_BITS 32u
#define XENOLITH_PACKET_ROOM 64u

/* How many samples a frame decodes to, and the parts a loop may begin or end
 * part way through one at. */
#define XENOLITH_FRAME_SAMPLES_OUT 512u
#define XENOLITH_SUBFRAME_SAMPLES 128u

/* What the ring is counted in: blocks of two hundred and fifty six bytes. */
#define XENOLITH_BLOCK_BYTES 256u

/* Decoded sound waiting for room in a ring: a few packets' worth. */
#define XENOLITH_WAITING_BYTES 65536u

static struct {
    void *library;
    xenolith_av_find find;
    xenolith_av_alloc_context alloc_context;
    xenolith_av_free free_context;
    xenolith_av_alloc parameters_alloc;
    xenolith_av_free parameters_free;
    xenolith_av_to_context to_context;
    xenolith_av_open open;
    xenolith_av_alloc packet_alloc;
    xenolith_av_free packet_free;
    xenolith_av_alloc frame_alloc;
    xenolith_av_free frame_free;
    xenolith_av_send send;
    xenolith_av_receive receive;
    xenolith_av_flush flush;
    const void *decoder;
    void *packet;
    void *frame;
    int refused;
} decoding;

/* What is kept about one context between one look at it and the next. */
struct compressed_sound {
    void *decoder;
    int unopenable;
    uint32_t channels;
    uint32_t rate;
    uint8_t waiting[XENOLITH_WAITING_BYTES];
    uint32_t waiting_bytes;
};

static struct compressed_sound *compressed_sounds[XENOLITH_SOUND_CONTEXTS];
static pthread_mutex_t compressed_sounds_lock = PTHREAD_MUTEX_INITIALIZER;

/* Reaches the host's decoder, or says once why there is none. */
static int reach_the_decoder(void) {
    if (decoding.decoder != NULL) {
        return 1;
    }
    if (decoding.refused) {
        return 0;
    }
    decoding.refused = 1;
    static const char *const names[] = {"libavcodec.so.60", "libavcodec.so.58",
                                        "libavcodec.so"};
    for (size_t at = 0; at < sizeof names / sizeof *names && decoding.library == NULL; at++) {
        decoding.library = dlopen(names[at], RTLD_NOW);
    }
    if (decoding.library == NULL) {
        say_a_report("compressed sound is not decoded: libavcodec is not on this host");
        return 0;
    }
    decoding.find = (xenolith_av_find)dlsym(decoding.library, "avcodec_find_decoder_by_name");
    decoding.alloc_context =
        (xenolith_av_alloc_context)dlsym(decoding.library, "avcodec_alloc_context3");
    decoding.free_context = (xenolith_av_free)dlsym(decoding.library, "avcodec_free_context");
    decoding.parameters_alloc =
        (xenolith_av_alloc)dlsym(decoding.library, "avcodec_parameters_alloc");
    decoding.parameters_free =
        (xenolith_av_free)dlsym(decoding.library, "avcodec_parameters_free");
    decoding.to_context =
        (xenolith_av_to_context)dlsym(decoding.library, "avcodec_parameters_to_context");
    decoding.open = (xenolith_av_open)dlsym(decoding.library, "avcodec_open2");
    decoding.packet_alloc = (xenolith_av_alloc)dlsym(decoding.library, "av_packet_alloc");
    decoding.packet_free = (xenolith_av_free)dlsym(decoding.library, "av_packet_free");
    decoding.frame_alloc = (xenolith_av_alloc)dlsym(decoding.library, "av_frame_alloc");
    decoding.frame_free = (xenolith_av_free)dlsym(decoding.library, "av_frame_free");
    decoding.send = (xenolith_av_send)dlsym(decoding.library, "avcodec_send_packet");
    decoding.receive = (xenolith_av_receive)dlsym(decoding.library, "avcodec_receive_frame");
    decoding.flush = (xenolith_av_flush)dlsym(decoding.library, "avcodec_flush_buffers");
    if (decoding.find == NULL || decoding.alloc_context == NULL ||
        decoding.free_context == NULL || decoding.parameters_alloc == NULL ||
        decoding.parameters_free == NULL || decoding.to_context == NULL ||
        decoding.open == NULL || decoding.packet_alloc == NULL || decoding.packet_free == NULL ||
        decoding.frame_alloc == NULL || decoding.frame_free == NULL || decoding.send == NULL ||
        decoding.receive == NULL || decoding.flush == NULL) {
        say_a_report("compressed sound is not decoded: libavcodec is missing a way to "
                     "decode");
        return 0;
    }
    decoding.decoder = decoding.find("xma2");
    decoding.packet = decoding.packet_alloc();
    decoding.frame = decoding.frame_alloc();
    if (decoding.decoder == NULL || decoding.packet == NULL || decoding.frame == NULL) {
        decoding.decoder = NULL;
        say_a_report("compressed sound is not decoded: libavcodec has no decoder for it");
        return 0;
    }
    decoding.refused = 0;
    return 1;
}

static void put_an_int(void *into, uint32_t at, int value) {
    memcpy((uint8_t *)into + at, &value, sizeof value);
}

/* Opens a decoder for a stream of so many channels at so many samples a second.
 *
 * What the decoder is told of the stream is the console's own description of
 * one, after the part every sound format shares: one stream, which channels it
 * carries, and how big a block is. */
static void *open_a_decoder(uint32_t channels, uint32_t rate) {
    uint8_t *described = calloc(34u + XENOLITH_PACKET_ROOM, 1);
    void *parameters = decoding.parameters_alloc();
    if (described == NULL || parameters == NULL) {
        free(described);
        decoding.parameters_free(&parameters);
        return NULL;
    }
    uint32_t which_channels = channels == 2u ? 3u : 4u;
    described[0] = 1;
    memcpy(described + 2, &which_channels, 4);
    uint32_t block = XENOLITH_PACKET_BYTES;
    memcpy(described + 10, &block, 4);

    put_an_int(parameters, XENOLITH_PARAMETERS_KIND, XENOLITH_AUDIO);
    memcpy((uint8_t *)parameters + XENOLITH_PARAMETERS_EXTRA, &described, sizeof described);
    put_an_int(parameters, XENOLITH_PARAMETERS_EXTRA_SIZE, 34);
    put_an_int(parameters, XENOLITH_PARAMETERS_CHANNELS, (int)channels);
    put_an_int(parameters, XENOLITH_PARAMETERS_RATE, (int)rate);
    put_an_int(parameters, XENOLITH_PARAMETERS_BLOCK, (int)XENOLITH_PACKET_BYTES);

    void *decoder = decoding.alloc_context(decoding.decoder);
    int made = decoder != NULL && decoding.to_context(decoder, parameters) >= 0 &&
               decoding.open(decoder, decoding.decoder, NULL) >= 0;

    void *nothing = NULL;
    memcpy((uint8_t *)parameters + XENOLITH_PARAMETERS_EXTRA, &nothing, sizeof nothing);
    decoding.parameters_free(&parameters);
    free(described);
    if (!made) {
        decoding.free_context(&decoder);
        return NULL;
    }
    return decoder;
}

/* Forgets what was decoded for a context, which a title has made again or let
 * go, so that a sound played in it next starts from nothing. */
static void forget_a_compressed_sound(uint32_t context) {
    pthread_mutex_lock(&compressed_sounds_lock);
    if (sound_contexts != 0 && context >= sound_contexts) {
        uint32_t at = (context - sound_contexts) / XENOLITH_SOUND_CONTEXT;
        if (at < XENOLITH_SOUND_CONTEXTS && compressed_sounds[at] != NULL) {
            struct compressed_sound *sound = compressed_sounds[at];
            if (sound->decoder != NULL) {
                decoding.free_context(&sound->decoder);
            }
            sound->unopenable = 0;
            sound->waiting_bytes = 0;
        }
    }
    pthread_mutex_unlock(&compressed_sounds_lock);
}

/* Changes some bits of one of a context's words, leaving the rest as whoever
 * else is writing them left them. */
static void change_a_context_word(uint8_t *base, uint32_t at, uint32_t keep, uint32_t put) {
    uint32_t *word = (uint32_t *)(void *)(base + at);
    uint32_t was = __atomic_load_n(word, __ATOMIC_RELAXED);
    uint32_t now;
    do {
        now = __builtin_bswap32((__builtin_bswap32(was) & keep) | put);
    } while (!__atomic_compare_exchange_n(word, &was, now, 1, __ATOMIC_ACQ_REL,
                                          __ATOMIC_RELAXED));
}

static int16_t a_sixteen_bit_sample(float given) {
    float in_range = given > 1.0f ? 1.0f : (given < -1.0f ? -1.0f : given);
    return (int16_t)(in_range * 32767.0f);
}

/* Decodes one packet, putting what comes out after what is already waiting as
 * sixteen bit samples, the channels taken in turn, in the title's byte order. */
static void decode_a_packet(struct compressed_sound *sound, const uint8_t *packet) {
    uint8_t padded[XENOLITH_PACKET_BYTES + XENOLITH_PACKET_ROOM] = {0};
    memcpy(padded, packet, XENOLITH_PACKET_BYTES);
    uint8_t *data = padded;
    memcpy((uint8_t *)decoding.packet + XENOLITH_PACKET_DATA, &data, sizeof data);
    put_an_int(decoding.packet, XENOLITH_PACKET_SIZE, (int)XENOLITH_PACKET_BYTES);
    if (decoding.send(sound->decoder, decoding.packet) < 0) {
        return;
    }
    while (decoding.receive(sound->decoder, decoding.frame) >= 0) {
        int samples = 0;
        int format = 0;
        memcpy(&samples, (uint8_t *)decoding.frame + XENOLITH_FRAME_SAMPLES, sizeof samples);
        memcpy(&format, (uint8_t *)decoding.frame + XENOLITH_FRAME_FORMAT, sizeof format);
        float *planes[2] = {NULL, NULL};
        memcpy(planes, decoding.frame, sizeof planes);
        if (format != XENOLITH_FLOATS_APART || planes[0] == NULL ||
            (sound->channels == 2u && planes[1] == NULL)) {
            continue;
        }
        for (int at = 0; at < samples; at++) {
            for (uint32_t channel = 0; channel < sound->channels; channel++) {
                if (sound->waiting_bytes + 2u > XENOLITH_WAITING_BYTES) {
                    return;
                }
                uint16_t sample = (uint16_t)a_sixteen_bit_sample(planes[channel][at]);
                sound->waiting[sound->waiting_bytes++] = (uint8_t)(sample >> 8);
                sound->waiting[sound->waiting_bytes++] = (uint8_t)sample;
            }
        }
    }
}

/* Reads so many bits of a packet, the first of them the highest, from so many
 * bits into it. */
static uint32_t bits_of_a_packet(const uint8_t *packet, uint32_t at, uint32_t how_many) {
    uint32_t value = 0;
    for (uint32_t bit = at; bit < at + how_many; bit++) {
        value = (value << 1) | ((packet[bit / 8u] >> (7u - bit % 8u)) & 1u);
    }
    return value;
}

/* How many of the frames that end in a packet start before a place, and how
 * many start at it or after, the place counted in bits from the front of the
 * packet.
 *
 * A packet says where the first frame to start in it begins, and each frame
 * begins with how long it is, so the frames are walked one after another. One
 * that runs on into the next packet ends there rather than here. */
static void frames_ending_in_a_packet(const uint8_t *packet, uint32_t place, uint32_t *before,
                                      uint32_t *after) {
    static const uint32_t no_frame_starts = 0x7fffu;
    static const uint32_t packet_bits = XENOLITH_PACKET_BYTES * 8u;
    *before = 0;
    *after = 0;
    uint32_t first = bits_of_a_packet(packet, 6u, 15u);
    if (first == no_frame_starts) {
        return;
    }
    uint32_t at = first + XENOLITH_PACKET_HEADER_BITS;
    while (at + 15u <= packet_bits) {
        uint32_t length = bits_of_a_packet(packet, at, 15u);
        if (length == 0 || length == no_frame_starts || at + length > packet_bits) {
            return;
        }
        if (at < place) {
            (*before)++;
        } else {
            (*after)++;
        }
        at += length;
    }
}

/* Where the decoder has reached in a context's input, and how far it may go. */
struct reading_a_context {
    uint32_t current;
    uint32_t buffer;
    uint32_t packets;
    uint32_t offset;
};

/* Finds the input buffer to read next, moving to the other where the current
 * one is not valid. Nought where neither is. */
static int find_the_input(uint8_t *base, uint32_t context, struct reading_a_context *reading) {
    uint32_t first = xenolith_load32(base, context);
    uint32_t valid[2] = {(first >> 20) & 1u, (first >> 21) & 1u};
    uint32_t current = xenolith_load32(base, context + 16u) >> 31;
    if (!valid[current]) {
        if (!valid[current ^ 1u]) {
            return 0;
        }
        current ^= 1u;
        change_a_context_word(base, context + 16u, 0x7fffffffu, current << 31);
        change_a_context_word(base, context + 8u, 0xfc000000u, XENOLITH_PACKET_HEADER_BITS);
    }
    reading->current = current;
    reading->buffer = xenolith_load32(base, context + 20u + current * 4u);
    reading->packets = current == 0 ? first & 0xfffu : xenolith_load32(base, context + 4u) & 0xfffu;
    reading->offset = xenolith_load32(base, context + 8u) & 0x3ffffffu;
    return 1;
}

/* Goes back to where a loop begins, where the packet just decoded holds where
 * it ends and the context says to go round it again. Gives the packet to read
 * next, or nought where there is no going back.
 *
 * Where a loop begins and ends is said as the frames it begins and ends at,
 * counted in bits through the buffer, and as how many parts of a frame to leave
 * off or take on at either end. What was decoded past the end is taken off what
 * waits for the ring, the decoder starts again at the packet holding the
 * beginning, and what that decodes before the beginning is taken off too. */
static uint32_t go_round_a_loop(uint8_t *base, uint32_t context, struct compressed_sound *sound,
                                const struct reading_a_context *input, uint32_t packet,
                                uint32_t before) {
    static const uint32_t packet_bits = XENOLITH_PACKET_BYTES * 8u;
    static const uint32_t for_ever = 255u;
    uint32_t first = xenolith_load32(base, context);
    uint32_t times = (first >> 12) & 0xffu;
    uint32_t second = xenolith_load32(base, context + 4u);
    uint32_t begins = xenolith_load32(base, context + 12u) & 0x3ffffffu;
    uint32_t ends = xenolith_load32(base, context + 16u) & 0x3ffffffu;
    if (times == 0 || ends <= begins || ends / packet_bits != packet ||
        begins / packet_bits >= input->packets) {
        return 0;
    }
    uint32_t frame_bytes = XENOLITH_FRAME_SAMPLES_OUT * sound->channels * 2u;
    uint32_t part_bytes = XENOLITH_SUBFRAME_SAMPLES * sound->channels * 2u;
    uint32_t buffer = input->buffer & (XENOLITH_PHYSICAL - 1u);

    uint32_t kept_before = 0;
    uint32_t past_the_end = 0;
    frames_ending_in_a_packet(base + buffer + packet * XENOLITH_PACKET_BYTES, ends % packet_bits,
                              &kept_before, &past_the_end);
    uint32_t taken_off = past_the_end * frame_bytes;
    uint32_t parts_on = (second >> 14) & 7u;
    if (past_the_end != 0 && taken_off >= parts_on * part_bytes) {
        taken_off -= parts_on * part_bytes;
    }
    uint32_t decoded = sound->waiting_bytes - before;
    sound->waiting_bytes -= taken_off < decoded ? taken_off : decoded;

    uint32_t start = begins / packet_bits;
    const uint8_t *beginning = base + buffer + start * XENOLITH_PACKET_BYTES;
    decoding.flush(sound->decoder);
    uint32_t from = sound->waiting_bytes;
    decode_a_packet(sound, beginning);
    uint32_t ahead = 0;
    uint32_t in_the_loop = 0;
    frames_ending_in_a_packet(beginning, begins % packet_bits, &ahead, &in_the_loop);
    uint32_t left_off = ahead * frame_bytes + ((second >> 12) & 3u) * part_bytes;
    uint32_t got = sound->waiting_bytes - from;
    if (left_off > got) {
        left_off = got;
    }
    memmove(sound->waiting + from, sound->waiting + from + left_off, got - left_off);
    sound->waiting_bytes -= left_off;

    if (times != for_ever) {
        change_a_context_word(base, context, ~(0xffu << 12), (times - 1u) << 12);
    }
    return start + 1u + beginning[3];
}

/* Moves sound from one context's input into its ring for as long as there is
 * input to read and room to put what comes of it. */
static void decode_a_context(uint8_t *base, uint32_t context, struct compressed_sound *sound) {
    /* A context that says no ring, or a write past its end, is one the title
     * has not armed yet; there is nothing to do for it. */
    uint32_t shape = xenolith_load32(base, context);
    uint32_t ring_blocks = (shape >> 22) & 31u;
    uint32_t write_at = (shape >> 27) & 31u;
    if (ring_blocks == 0 || write_at >= ring_blocks) {
        return;
    }
    uint32_t second = xenolith_load32(base, context + 4u);
    if ((second & 0x80000000u) == 0) {
        return;
    }
    static const uint32_t rates[4] = {24000u, 32000u, 44100u, 48000u};
    uint32_t channels = (second >> 29) & 1u ? 2u : 1u;
    uint32_t rate = rates[(second >> 27) & 3u];
    if (sound->channels != channels || sound->rate != rate) {
        if (sound->decoder != NULL) {
            decoding.free_context(&sound->decoder);
        }
        sound->unopenable = 0;
        sound->channels = channels;
        sound->rate = rate;
        sound->waiting_bytes = 0;
    }
    if (sound->decoder == NULL) {
        if (sound->unopenable) {
            return;
        }
        sound->decoder = open_a_decoder(channels, rate);
        if (sound->decoder == NULL) {
            sound->unopenable = 1;
            say_a_report("compressed sound of %u channels at %u is not decoded: libavcodec "
                         "would not open a decoder for it",
                         channels, rate);
            return;
        }
    }

    uint32_t reading = xenolith_load32(base, context + 36u) & 31u;
    if (reading >= ring_blocks) {
        return;
    }
    uint32_t room = (reading + ring_blocks - write_at) % ring_blocks;
    if (room == 0) {
        room = ring_blocks;
    }
    for (;;) {
        uint32_t first = xenolith_load32(base, context);
        uint32_t blocks = (first >> 22) & 31u;
        uint32_t writing = (first >> 27) & 31u;
        uint32_t ring = xenolith_load32(base, context + 28u) & (XENOLITH_PHYSICAL - 1u);
        if (blocks == 0 || writing >= blocks) {
            return;
        }
        /* Output-valid distinguishes a full ring from an empty one when
         * the offsets meet. Filling the offered space hands it back to the
         * title, which advances the reader and rearms the output. */
        if (sound->waiting_bytes >= XENOLITH_BLOCK_BYTES) {
            memcpy(base + ring + writing * XENOLITH_BLOCK_BYTES, sound->waiting,
                   XENOLITH_BLOCK_BYTES);
            sound->waiting_bytes -= XENOLITH_BLOCK_BYTES;
            memmove(sound->waiting, sound->waiting + XENOLITH_BLOCK_BYTES, sound->waiting_bytes);
            change_a_context_word(base, context, 0x07ffffffu,
                                  ((writing + 1u) % blocks) << 27);
            if (--room == 0) {
                change_a_context_word(base, context + 4u, 0x7fffffffu, 0);
                return;
            }
            continue;
        }

        struct reading_a_context input;
        if (!find_the_input(base, context, &input)) {
            return;
        }
        uint32_t packet = input.offset / (XENOLITH_PACKET_BYTES * 8u);
        if (packet >= input.packets) {
            /* What is left of a skip past the end of one buffer is taken in
             * the other, which carries on where this one stops. */
            uint32_t past = packet - input.packets;
            change_a_context_word(base, context, ~(1u << (20u + input.current)), 0);
            change_a_context_word(base, context + 16u, 0x7fffffffu, (input.current ^ 1u) << 31);
            change_a_context_word(base, context + 8u, 0xfc000000u,
                                  past * XENOLITH_PACKET_BYTES * 8u +
                                      XENOLITH_PACKET_HEADER_BITS);
            continue;
        }
        uint32_t at = (input.buffer & (XENOLITH_PHYSICAL - 1u)) + packet * XENOLITH_PACKET_BYTES;
        uint32_t before = sound->waiting_bytes;
        decode_a_packet(sound, base + at);
        /* A sound of more than two channels is several streams whose packets
         * take turns in one buffer, a context reading one of them, and each
         * packet ends by saying how many after it belong to the others. */
        uint32_t next = packet + 1u + xenolith_load8(base, at + 3u);
        uint32_t looped = go_round_a_loop(base, context, sound, &input, packet, before);
        change_a_context_word(base, context + 8u, 0xfc000000u,
                              (looped != 0 ? looped : next) * XENOLITH_PACKET_BYTES * 8u +
                                  XENOLITH_PACKET_HEADER_BITS);
    }
}

/* Looks at every context a title holds, often enough that a ring a title is
 * playing from never runs dry. */
static void *keep_compressed_sound_decoded(void *given) {
    uint8_t *base = given;
    for (;;) {
        for (uint32_t at = 0; at < XENOLITH_SOUND_CONTEXTS; at++) {
            pthread_mutex_lock(&sound_contexts_lock);
            int taken = sound_context_taken[at];
            pthread_mutex_unlock(&sound_contexts_lock);
            if (!taken) {
                continue;
            }
            pthread_mutex_lock(&compressed_sounds_lock);
            if (compressed_sounds[at] == NULL) {
                compressed_sounds[at] = calloc(1, sizeof *compressed_sounds[at]);
            }
            if (compressed_sounds[at] != NULL) {
                decode_a_context(base, sound_contexts + at * XENOLITH_SOUND_CONTEXT,
                                 compressed_sounds[at]);
            }
            pthread_mutex_unlock(&compressed_sounds_lock);
        }
        struct timespec a_while = {0, 2000000};
        nanosleep(&a_while, NULL);
    }
    return NULL;
}

/* Starts decoding the first time a title makes a context, or says once why
 * nothing will be. */
static void start_decoding_compressed_sound(uint8_t *base) {
    static int started;
    if (started) {
        return;
    }
    started = 1;
    if (!reach_the_decoder()) {
        return;
    }
    pthread_t decoder;
    if (pthread_create(&decoder, NULL, keep_compressed_sound_decoded, base) == 0) {
        pthread_detach(decoder);
    }
}
