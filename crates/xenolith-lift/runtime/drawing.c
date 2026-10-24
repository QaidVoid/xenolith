/* Drawing what a title asked for.
 *
 * A title does not draw. It hands over a pair of shaders, fills in the fetch
 * registers and the constants, and then writes a draw command saying how many
 * vertices and what shape. Everything that decides what appears is in those
 * registers, so this is where the command stream's reading turns into something
 * the renderer can act on.
 *
 * This file is included by xenolith.c rather than compiled on its own, because
 * it needs the register accessors and the reporting lock, and those live there.
 */


static uint32_t drawn_blending, drawn_colour_control, drawn_written, drawn_how_far;
static uint32_t drawn_cut_off, drawn_cut_off_at;
static unsigned long long in_mode[16][8];
static float moved_depth_by, moved_depth_to;

/* The last shader of each kind a title handed over.
 *
 * A draw uses whichever pair was last set up, because that is how a title says
 * which: it hands them over and then draws, rather than naming them in the
 * draw. */
static struct {
    uint32_t sum;
    uint32_t words;
    const uint32_t *said;
    uint32_t length;
    /* Where a shader fetches from and which pictures it reads, which come from
     * the same place the shader did rather than being asked after separately:
     * one said while a title runs is in no table to ask. */
    const uint32_t *reads;
    uint32_t streams;
    const uint32_t *looks;
    uint32_t pictures;
} in_hand[2];

/* What the last draw was clipped to, how it asked for a position to be moved,
 * and what it drew into. A census reads these, and saying where a position
 * moved decides whether how far away a thing is lands inside the range a host
 * keeps. */
static uint32_t kept_left, kept_top, kept_right, kept_bottom;
static uint32_t moved_how;
static float moved_scale[4], moved_offset[4];
static uint32_t drawn_onto, drawn_laid_out, drawn_depth_onto;
static uint32_t drawn_how, drawn_from_a_packet, drawn_offset_by;
static uint32_t drawn_looks_at, drawn_looks_at_size;
static uint32_t drawn_reads_from, drawn_reads_slot;
static uint32_t drawn_in_mode;

static void say_what_a_shader_was(const uint8_t *base, uint32_t sum, uint32_t where,
                                  uint32_t words) {
    static uint32_t told[64];
    static uint32_t telling;

    const char *into = getenv("XENOLITH_KEEP_SHADERS");
    if (into == NULL || words == 0) {
        return;
    }
    for (uint32_t at = 0; at < telling; at++) {
        if (told[at] == sum) {
            return;
        }
    }
    if (telling >= 64u) {
        return;
    }
    told[telling++] = sum;

    char name[512];
    if (snprintf(name, sizeof name, "%s/%08x.ucode", into, sum) >= (int)sizeof name) {
        return;
    }
    FILE *out = fopen(name, "wb");
    if (out == NULL) {
        return;
    }
    for (uint32_t k = 0; k < words; k++) {
        uint32_t word = xenolith_load32(base, where + k * 4);
        fwrite(&word, 4, 1, out);
    }
    fclose(out);
}

#ifdef XENOLITH_CAN_SAY_A_SHADER
static void say_what_a_shader_became(uint32_t sum, const uint32_t *said, uint32_t length) {
    static uint32_t told[64];
    static uint32_t telling;

    const char *into = getenv("XENOLITH_KEEP_SHADERS");
    if (into == NULL || said == NULL || length == 0) {
        return;
    }
    for (uint32_t at = 0; at < telling; at++) {
        if (told[at] == sum) {
            return;
        }
    }
    if (telling >= 64u) {
        return;
    }
    told[telling++] = sum;

    char where[512];
    if (snprintf(where, sizeof where, "%s/%08x.spv", into, sum) >= (int)sizeof where) {
        return;
    }
    FILE *out = fopen(where, "wb");
    if (out == NULL) {
        return;
    }
    fwrite(said, 4, length, out);
    fclose(out);
}
#endif

#ifdef XENOLITH_CAN_SAY_A_SHADER
static void said_one_while_running(uint32_t which_kind, uint32_t words) {
    static int said;

    if (said) {
        return;
    }
    said = 1;
    fprintf(stderr,
            "xenolith: a shader of %u words was said while running, of the kind that runs for "
            "%s, because a title loaded it from the data beside its executable\n",
            words, which_kind == 0 ? "each vertex" : "each pixel");
}
#endif

static void said_nothing_of_a_shader(uint32_t sum, uint32_t words) {
    static uint32_t said[64];
    static unsigned count;

    for (unsigned at = 0; at < count; at++) {
        if (said[at] == sum) {
            return;
        }
    }
    if (count < sizeof said / sizeof *said) {
        said[count++] = sum;
    }

    fprintf(stderr,
            "xenolith: a shader of %u words was handed over that nothing here was made of\n",
            words);
}

#define XENOLITH_COUNT_REGISTER 0x4908u
#define XENOLITH_TRUTH_REGISTER 0x4900u
#define XENOLITH_WINDOW_EDGE 0x7fffu
#define XENOLITH_WINDOW_STARTS 0x2081u
#define XENOLITH_WINDOW_STOPS 0x2082u

/* Where a title says the part of a frame being drawn sits in the memory beside
 * the hardware, as how far to move it across and down.
 *
 * A title whose frame does not fit in that memory draws it a part at a time,
 * each time moving what it draws so that the part it means lands at the top
 * left. Whether a shape's corners are moved is a bit of its own, and whether
 * what a draw is kept to is moved is a bit of what it is kept to. */
#define XENOLITH_WINDOW_OFFSET 0x2080u
#define XENOLITH_HOW_SHAPES_ARE_FILLED 0x2205u
#define XENOLITH_CORNERS_ARE_MOVED (1u << 16)
#define XENOLITH_KEPT_WHERE_IT_IS (1u << 31)

/* One of the two halves of a place's move, which are fifteen bits each and
 * may be less than nought. */
static int32_t moved_by(uint32_t said, unsigned int shift) {
    return (int32_t)(((said >> shift) & XENOLITH_WINDOW_EDGE) << 17) >> 17;
}

/* How far a shape's corners are moved across and down, which is nothing where
 * a title has not asked for them to be. */
static void how_far_corners_move(const uint8_t *base, int32_t *across, int32_t *down) {
    uint32_t said = xenolith_load32(base, XENOLITH_REGISTERS + XENOLITH_WINDOW_OFFSET * 4u);
    uint32_t filled =
        xenolith_load32(base, XENOLITH_REGISTERS + XENOLITH_HOW_SHAPES_ARE_FILLED * 4u);
    int moved = (filled & XENOLITH_CORNERS_ARE_MOVED) != 0;
    *across = moved ? moved_by(said, 0) : 0;
    *down = moved ? moved_by(said, 16) : 0;
}

/* What a draw is kept to, in the place drawn into. */
static void where_a_draw_is_kept(const uint8_t *base, int32_t *left, int32_t *top,
                                 int32_t *right, int32_t *bottom) {
    uint32_t from = xenolith_load32(base, XENOLITH_REGISTERS + XENOLITH_WINDOW_STARTS * 4u);
    uint32_t to = xenolith_load32(base, XENOLITH_REGISTERS + XENOLITH_WINDOW_STOPS * 4u);
    uint32_t said = xenolith_load32(base, XENOLITH_REGISTERS + XENOLITH_WINDOW_OFFSET * 4u);
    int32_t across = (from & XENOLITH_KEPT_WHERE_IT_IS) != 0 ? 0 : moved_by(said, 0);
    int32_t down = (from & XENOLITH_KEPT_WHERE_IT_IS) != 0 ? 0 : moved_by(said, 16);

    *left = (int32_t)(from & XENOLITH_WINDOW_EDGE) + across;
    *top = (int32_t)((from >> 16) & XENOLITH_WINDOW_EDGE) + down;
    *right = (int32_t)(to & XENOLITH_WINDOW_EDGE) + across;
    *bottom = (int32_t)((to >> 16) & XENOLITH_WINDOW_EDGE) + down;
}

static uint32_t not_below_nought(int32_t at) { return at < 0 ? 0u : (uint32_t)at; }
#define XENOLITH_PACKET_DRAW 0x36u
#define XENOLITH_PACKET_DRAW_INDEXED 0x22u

static void say_whether_a_draw_was_asked_about(uint32_t header) {
    static int telling = -1;
    static unsigned long long asked, plain;

    if (telling < 0) {
        telling = getenv("XENOLITH_TRACE_DRAWS") != NULL;
    }
    if (!telling) {
        return;
    }
    if ((header & 1u) != 0) {
        asked++;
    } else {
        plain++;
    }
    if ((asked + plain) % 20000ull == 0) {
        fprintf(stderr, "xenolith: %llu draws asked about first and %llu not\n", asked, plain);
    }
}

/* Whether a frame has been started but not yet shown. */
static int drawing_a_frame;

/* Whether a frame is being drawn is reached from two threads: the one reading a
 * title's commands, which begins one and draws into it, and whichever guest
 * thread says to show what was drawn. Both begin and end frames, so which of
 * them is the one doing it has to be settled rather than raced over. */
static pthread_mutex_t drawing_a_frame_lock = PTHREAD_MUTEX_INITIALIZER;

/* Ends the frame being drawn, where one is, and says whether there was. */
static int finish_the_frame(void) {
    pthread_mutex_lock(&drawing_a_frame_lock);
    int was = drawing_a_frame;
    if (was) {
        xenolith_end_a_frame();
        drawing_a_frame = 0;
    }
    pthread_mutex_unlock(&drawing_a_frame_lock);
    return was;
}

/* Begins one where none is being drawn. */
static void start_the_frame(void) {
    pthread_mutex_lock(&drawing_a_frame_lock);
    if (!drawing_a_frame) {
        drawing_a_frame = xenolith_begin_a_frame();
    }
    pthread_mutex_unlock(&drawing_a_frame_lock);
}

/* How many pictures a shader can be handed, which is how many the hardware
 * keeps a place for. */
#define XENOLITH_PICTURES_SHOWN 24u

static void said_nothing_of_a_picture(uint32_t format) {
    static uint32_t said[8];
    static unsigned count;

    for (unsigned at = 0; at < count; at++) {
        if (said[at] == format) {
            return;
        }
    }
    if (count < sizeof said / sizeof *said) {
        said[count++] = format;
    }

    fprintf(stderr,
            "xenolith: a picture was handed over kept as %u, which is not a way this reads one, "
            "so nothing was made of it\n",
            format);
}

/* Which vertex of a fetch a draw's first one is.
 *
 * The hardware counts a draw's vertices from nought and adds this, so a title
 * can keep many shapes in one buffer and say which of them to draw without
 * moving the fetch. A draw that counts its own vertices rather than fetching
 * them still goes through it, which is what makes it the only thing saying
 * where such a draw begins. */
#define XENOLITH_FIRST_VERTEX 0x2102u

#define XENOLITH_MOVING_A_POSITION 0x2206u

#define XENOLITH_VIEWPORT_SCALE_X 0x210fu

/* Which of the six parts is applied, one bit each, and then whether a shader
 * has already divided its position through by the fourth number. */
#define XENOLITH_MOVING_ANY 0x3fu

#define XENOLITH_ALREADY_DIVIDED 0x100u

/* How far across and down a draw in pixels is looked at, which is more than any
 * grid of samples and no more than every host takes. */
#define XENOLITH_PIXELS_STRETCH 4096.0f

/* How many samples a place keeps for each of its pixels, in the two bits above
 * its row width: one, two or four. */
#define XENOLITH_SAMPLES_SHIFT 16u

/* How many times wider and taller a place's samples are than its pixels.
 *
 * The hardware keeps a place drawn with four samples a pixel as a grid twice as
 * wide and twice as tall, and one with two samples as a grid twice as tall. A
 * title reads that grid back as a picture of its own size: one draws at 640 by
 * 360 with four samples and copies the result out as 1280 by 720. Drawn here at
 * the size the title names, the whole frame lands in a quarter of what is
 * copied, so everything is drawn onto the grid instead. */
static uint32_t samples_across = 1;
static uint32_t samples_down = 1;

/* Where a title says which parts of a colour reach the surface. */
#define XENOLITH_WHAT_IS_WRITTEN 0x2104u

/* Where a title says whether to look at how far away a place already is before
 * drawing over it, whether to say how far away what it draws now is, and which
 * way round the two are compared. */
#define XENOLITH_HOW_FAR_IS_READ 0x2200u

/* The bit of what a title lays a colour down with that says a place is to be
 * thrown away where how solid it is does not answer the test beside it. */
#define XENOLITH_A_TEST_IS_SET 8u

/* Counts what a title draws and says so now and again.
 *
 * What is on screen follows from what was asked for, so the shapes, how many
 * corners they carry, which pair of a title's own shaders drew them and whether
 * anything was drawn at all is the first thing to know when a picture is
 * missing. Kept as a small tally rather than a line per draw, because a title
 * draws thousands a second.
 */
#define XENOLITH_SHAPES_COUNTED 16u

/* Where a title says each of its fetches reads from. */
#define XENOLITH_FETCH_REGISTER 0x4800u

#define XENOLITH_FETCH_SLOTS 96u

/* What a fetch constant says it is: three means a run of vertices. */
#define XENOLITH_FETCH_IS_VERTICES 3u

/* Where a title says which pictures its shaders read, and what says the one
 * format this reads them in.
 *
 * The places sit beside the ones a vertex is fetched through, six registers to
 * each, and what a place holds says whether it is a picture at all. Only the
 * format this title was seen to use is handed over; anything else is reported,
 * because a picture read as the wrong format is wrong quietly. */
#define XENOLITH_PICTURE_REGISTER 0x4800u

#define XENOLITH_PICTURE_PLACES 32u

/* How many registers one picture is described by, how much room a title leaves
 * the system to say where the finished one is, and the command that means
 * nothing at all.
 *
 * The room is what a title reserves in its own command buffer before asking for
 * a swap. The number is the console's, taken from what a title leaves untouched
 * either side of the call. */
#define XENOLITH_PICTURE_WORDS 6u

#define XENOLITH_PLACE_HOLDS_A_PICTURE 2u

#define XENOLITH_PICTURE_IN_SQUARES 20u

#define XENOLITH_PICTURE_IN_SQUARES_PLAIN 19u

#define XENOLITH_PICTURE_IN_SQUARES_SMALL 18u

#define XENOLITH_PICTURE_IN_FLOATS 36u

#define XENOLITH_PICTURE_IN_PARTS 2u

#define XENOLITH_PICTURE_IN_FOURS 6u

#define XENOLITH_PICTURE_HOW_FAR 22u

#define XENOLITH_PICTURE_HOW_FAR_AS_A_FLOAT 23u

/* Says which slot a shape's corners were read from, where in a title's own
 * memory that is, and how much of what was there was anything at all.
 *
 * A draw whose corners are all nought is either one a title never filled in or
 * one this runtime read from the wrong place, and the address is what tells the
 * two apart.
 */
#define XENOLITH_SLOTS_SHOWN 12u

/* Where a title says whether the hardware puts where a pixel is in one of a
 * pixel shader's registers, and which. */
#define XENOLITH_PROGRAM_CONTROL 0x2180u
#define XENOLITH_PIXEL_IS_GIVEN (1u << 18)
#define XENOLITH_SHADER_MISCELLANY 0x2181u

static void say_where_the_pixel_goes(const uint8_t *base) {
    uint32_t control = xenolith_load32(base, XENOLITH_REGISTERS + XENOLITH_PROGRAM_CONTROL * 4u);
    uint32_t which =
        (xenolith_load32(base, XENOLITH_REGISTERS + XENOLITH_SHADER_MISCELLANY * 4u) >> 8) & 0xffu;
    xenolith_find_the_pixel_in((control & XENOLITH_PIXEL_IS_GIVEN) != 0 ? which : 0xffffffffu);
}

/* Says which of a title's places the next draw goes into, and starts a frame
 * in it, ending the one before where that was drawing into another.
 *
 * Which place is how wide a row of it is and how many samples to a pixel,
 * beside what each of them holds. It is settled before the frame starts,
 * because starting one is what brings the place a title turned to back onto
 * the surface. Settled after, each frame took up the place the draw before it
 * went into: a pass drawn at half the size of the frame after one drawn at the
 * whole size went into the whole one, a quarter of the way across it. */
static void start_the_frame_in_its_place(const uint8_t *base) {
    static uint32_t shaped_as;

    drawn_laid_out = xenolith_load32(base, XENOLITH_REGISTERS + 0x2000u * 4u);
    drawn_onto = xenolith_load32(base, XENOLITH_REGISTERS + 0x2001u * 4u);
    uint32_t samples = (drawn_laid_out >> XENOLITH_SAMPLES_SHIFT) & 3u;
    samples_across = samples == 2u ? 2u : 1u;
    samples_down = samples != 0u ? 2u : 1u;

    uint32_t shaped = (drawn_laid_out & 0x3ffffu) | ((drawn_onto & 0xf0000u) << 2);
    if (shaped != shaped_as) {
        finish_the_frame();
        shaped_as = shaped;
    }
    /* The place is how many samples wide a row is rather than how many pixels,
     * because that is what the memory beside the hardware holds. A pass of a
     * quarter the size with four samples to a pixel is the same samples as one
     * of the whole size with one, and a title draws into the one and copies
     * out of the other: it lays a scene down at the whole size, draws its
     * smoke and glows over it at the quarter, and copies the whole size out.
     * Told apart, the glows went over whatever the quarter last held, and the
     * copy was of that. */
    xenolith_drawing_into(((drawn_laid_out & 0x3fffu) * samples_across)
                              | ((drawn_onto & 0xf0000u) << 2),
                          samples_across, samples_down);
    start_the_frame();
}

static void say_how_a_position_moves(const uint8_t *base) {
    uint32_t how = xenolith_load32(base, XENOLITH_REGISTERS + XENOLITH_MOVING_A_POSITION * 4u);

    moved_how = how;
    drawn_in_mode = xenolith_load32(base, XENOLITH_REGISTERS + 0x2208u * 4u) & 7u;
    drawn_depth_onto = xenolith_load32(base, XENOLITH_REGISTERS + 0x2002u * 4u);
    drawn_offset_by = xenolith_load32(base, XENOLITH_REGISTERS + 0x2080u * 4u);
    drawn_blending = xenolith_load32(base, XENOLITH_REGISTERS + 0x2201u * 4u);
    drawn_colour_control = xenolith_load32(base, XENOLITH_REGISTERS + 0x2202u * 4u);

    /* How solid a place has to be to be drawn at all. A title that asks for no
     * test is the same as one asking for the test that draws everything, so the
     * two are not told apart past here. */
    drawn_cut_off = (drawn_colour_control & XENOLITH_A_TEST_IS_SET) != 0
                        ? drawn_colour_control & 7u
                        : XENOLITH_DRAWN_HOWEVER_SOLID;
    drawn_cut_off_at = xenolith_load32(base, XENOLITH_REGISTERS + 0x210eu * 4u);

    /* Whether a title wants how far away a thing is looked at before it is
     * drawn over what is already there, and which way round. */
    drawn_how_far = xenolith_load32(base, XENOLITH_REGISTERS + XENOLITH_HOW_FAR_IS_READ * 4u);

    /* Which parts of a colour reach the surface. A title turns them all off for
     * a pass whose work is not to be seen. */
    drawn_written = xenolith_load32(base, XENOLITH_REGISTERS + XENOLITH_WHAT_IS_WRITTEN * 4u)
                    & 0xfu;
    {
        uint32_t held = xenolith_load32(base, XENOLITH_REGISTERS + 0x2113u * 4u);
        memcpy(&moved_depth_by, &held, sizeof moved_depth_by);
        held = xenolith_load32(base, XENOLITH_REGISTERS + 0x2114u * 4u);
        memcpy(&moved_depth_to, &held, sizeof moved_depth_to);
    }

    float scale[4] = {1.0f, 1.0f, 1.0f, 1.0f};
    float offset[4] = {0.0f, 0.0f, 0.0f, 0.0f};
    int32_t corners_across, corners_down;
    how_far_corners_move(base, &corners_across, &corners_down);

    if ((how & XENOLITH_MOVING_ANY) != 0) {
        /* The hardware is to do it, and what it does is exactly what a host
         * does of its own accord to everything it draws: a position between one
         * and minus one is put somewhere on what is drawn into. So the title's
         * numbers say where that is rather than being applied to the position,
         * and what the shader puts out is left alone.
         *
         * Doing it in the shader instead applies it twice, because a host still
         * does its own afterwards, and what a title meant to fill the screen
         * lands in a corner of it. */
        for (unsigned int at = 0; at < 3u; at++) {
            uint32_t held = xenolith_load32(
                base, XENOLITH_REGISTERS + (XENOLITH_VIEWPORT_SCALE_X + at * 2u) * 4u);
            memcpy(&scale[at], &held, sizeof scale[at]);
            held = xenolith_load32(
                base, XENOLITH_REGISTERS + (XENOLITH_VIEWPORT_SCALE_X + at * 2u + 1u) * 4u);
            memcpy(&offset[at], &held, sizeof offset[at]);
        }
        offset[0] += (float)corners_across;
        offset[1] += (float)corners_down;
        scale[0] *= (float)samples_across;
        offset[0] *= (float)samples_across;
        scale[1] *= (float)samples_down;
        offset[1] *= (float)samples_down;
        memcpy(moved_scale, scale, sizeof moved_scale);
        memcpy(moved_offset, offset, sizeof moved_offset);
        xenolith_look_at(scale, offset);
        xenolith_move_a_position_by(NULL, NULL);
        return;
    }

    /* Everything below is a position the hardware is told to do nothing to,
     * which makes where the shader put it a pixel: the parts the hardware would
     * have applied are what turn a position between one and minus one into one.
     * A host does turn it into one, and keeps nothing outside the stretch it
     * was told to draw onto, so a stretch larger than any grid of samples is
     * looked at and each pixel is brought to where the host will put it back.
     * The console puts a pixel's middle on the whole number, where a host puts
     * it half way between two, so half a pixel is added; and the samples grid
     * is what is drawn onto.
     *
     * Looking at a stretch the size of one pixel instead, and leaving the
     * position as it was, put the pixels in the right places and kept only the
     * few of them inside that stretch: a title wiping the screen with a
     * rectangle in pixels wiped a corner, and a film drawn over it after was
     * thrown away as further off than the wipe that never reached it. */
    {
        const float half = XENOLITH_PIXELS_STRETCH / 2.0f;
        const float stretch[4] = {half, half, 1.0f, 1.0f};
        const float middle[4] = {half, half, 0.0f, 0.0f};
        xenolith_look_at(stretch, middle);

        float onto_pixels[4] = {(float)samples_across / half, (float)samples_down / half, 1.0f,
                                1.0f};
        float half_a_pixel[4] = {(0.5f + (float)corners_across) * (float)samples_across / half -
                                     1.0f,
                                 (0.5f + (float)corners_down) * (float)samples_down / half - 1.0f,
                                 0.0f, 0.0f};
        if ((how & XENOLITH_ALREADY_DIVIDED) != 0) {
            /* The shader has divided its position through already, so what it
             * must not meet is a host dividing it a second time: the fourth is
             * made one. */
            onto_pixels[3] = 0.0f;
            half_a_pixel[3] = 1.0f;
        }
        xenolith_move_a_position_by(onto_pixels, half_a_pixel);
    }
}

static void say_what_was_drawn(uint32_t shape, uint32_t vertices, uint32_t vertex_shader,
                               uint32_t pixel_shader, int drawn) {
    static int telling = -1;
    static unsigned long long counted[XENOLITH_SHAPES_COUNTED];
    static unsigned long long corners[XENOLITH_SHAPES_COUNTED];
    static unsigned long long cornered[XENOLITH_SHAPES_COUNTED];
    static uint32_t by_shape[XENOLITH_SHAPES_COUNTED][4][2];
    static uint32_t blends[XENOLITH_SHAPES_COUNTED][4];
    static unsigned long long refused;
    static unsigned long long all;
    static uint32_t pairs[8][2];
    static uint32_t pairing;
    static uint32_t turned_down[8][2];
    static unsigned long long turned_down_often[8];
    static uint32_t turnings;

    if (telling < 0) {
        const char *how = getenv("XENOLITH_TRACE_DRAWS");
        telling = how == NULL ? 0 : (strcmp(how, "each") == 0 ? 2 : 1);
    }
    if (!telling) {
        return;
    }

    /* Each draw on its own line rather than a count of them all, which is what
     * says how one frame differs from another. A count over a whole run cannot:
     * a run that starts right and later goes wrong reads the same either way. */
    if (telling == 2) {
        fprintf(stderr,
                "xenolith: frame %llu drew shape %u of %u corners by %08x/%08x, blend %08x, moved "
                "%04x by %g %g and %g %g, kept to %u,%u..%u,%u%s\n",
                (unsigned long long)graphics.frames, shape, vertices, vertex_shader, pixel_shader,
                drawn_blending, moved_how, (double)moved_scale[0], (double)moved_offset[0],
                (double)moved_scale[1], (double)moved_offset[1], kept_left, kept_top, kept_right,
                kept_bottom, drawn ? "" : ", turned down");
        fprintf(stderr,
                "xenolith:   onto %08x, laid out %08x, depth onto %08x, offset by %08x, packet "
                "%#04x said %08x, taking its order from %u, looking at %08x of %ux%u\n",
                drawn_onto, drawn_laid_out, drawn_depth_onto, drawn_offset_by, drawn_from_a_packet,
                drawn_how, (drawn_how >> 6) & 3u, drawn_looks_at, drawn_looks_at_size >> 16,
                drawn_looks_at_size & 0xffffu);
        fprintf(stderr, "xenolith:   corners out of %08x, through fetch %u\n", drawn_reads_from,
                drawn_reads_slot);
    }

    if (shape < XENOLITH_SHAPES_COUNTED) {
        counted[shape]++;
        corners[shape] += vertices;
        if (kept_right - kept_left < 64u || kept_bottom - kept_top < 64u) {
            cornered[shape]++;
        }
        if (drawn_in_mode < 8u) {
            in_mode[shape][drawn_in_mode]++;
        }
        uint32_t slot = 0;
        while (slot < 4u && blends[shape][slot] != 0 && blends[shape][slot] != drawn_blending) {
            slot++;
        }
        if (slot < 4u) {
            blends[shape][slot] = drawn_blending;
        }
        uint32_t seat = 0;
        while (seat < 4u && by_shape[shape][seat][0] != 0
               && (by_shape[shape][seat][0] != vertex_shader
                   || by_shape[shape][seat][1] != pixel_shader)) {
            seat++;
        }
        if (seat < 4u) {
            by_shape[shape][seat][0] = vertex_shader;
            by_shape[shape][seat][1] = pixel_shader;
        }
    }
    if (!drawn) {
        refused++;
        uint32_t which = 0;
        while (which < turnings
               && (turned_down[which][0] != vertex_shader
                   || turned_down[which][1] != pixel_shader)) {
            which++;
        }
        if (which == turnings && turnings < 8u) {
            turned_down[turnings][0] = vertex_shader;
            turned_down[turnings][1] = pixel_shader;
            turnings++;
        }
        if (which < 8u) {
            turned_down_often[which]++;
        }
    }
    uint32_t one = 0;
    while (one < pairing && (pairs[one][0] != vertex_shader || pairs[one][1] != pixel_shader)) {
        one++;
    }
    if (one == pairing && pairing < 8u) {
        pairs[pairing][0] = vertex_shader;
        pairs[pairing][1] = pixel_shader;
        pairing++;
    }

    if (++all % 20000u != 0) {
        return;
    }
    fprintf(stderr, "xenolith: %llu draws, %llu of them refused, shapes", all, refused);
    for (uint32_t which = 0; which < XENOLITH_SHAPES_COUNTED; which++) {
        if (counted[which] != 0) {
            fprintf(stderr, " %u:%llux(%llu corners, %llu kept to a corner, modes", which,
                    counted[which], corners[which], cornered[which]);
            for (uint32_t mode = 0; mode < 8u; mode++) {
                if (in_mode[which][mode] != 0) {
                    fprintf(stderr, " %u:%llu", mode, in_mode[which][mode]);
                }
            }
            fprintf(stderr, ", blends");
            for (uint32_t slot = 0; slot < 4u && blends[which][slot] != 0; slot++) {
                fprintf(stderr, " %08x", blends[which][slot]);
            }
            fprintf(stderr, ", by");
            for (uint32_t seat = 0; seat < 4u && by_shape[which][seat][0] != 0; seat++) {
                fprintf(stderr, " %08x/%08x", by_shape[which][seat][0], by_shape[which][seat][1]);
            }
            fprintf(stderr, ")");
        }
    }
    fprintf(stderr, ", %u shader pairs:", pairing);
    for (uint32_t which = 0; which < pairing; which++) {
        fprintf(stderr, " %08x/%08x", pairs[which][0], pairs[which][1]);
    }
    fprintf(stderr, ", last kept to %u,%u..%u,%u, moved by %04x, depth by %g to %g", kept_left,
            kept_top, kept_right, kept_bottom, moved_how, (double)moved_depth_by,
            (double)moved_depth_to);
    if (turnings != 0) {
        fprintf(stderr, "xenolith: turned down:");
        for (uint32_t which = 0; which < turnings; which++) {
            fprintf(stderr, " %08x/%08x x%llu", turned_down[which][0], turned_down[which][1],
                    turned_down_often[which]);
        }
        fprintf(stderr, "\n");
    }
    fprintf(stderr, "\n");
}

static uint32_t complete_the_rectangle(uint32_t *window, uint32_t room, const uint32_t *reads,
                                       uint32_t streams) {
    uint32_t reached = 0;
    for (uint32_t one = 0; one < streams; one++) {
        uint32_t slot = reads[one * 4];
        uint32_t stride = reads[one * 4 + 1];
        uint32_t offset = reads[one * 4 + 2];
        uint32_t words = reads[one * 4 + 3];
        if (slot >= XENOLITH_FETCH_SLOTS || stride == 0 || words == 0) {
            continue;
        }

        uint32_t at = window[slot];
        if (at == 0 || at + 5u * stride + offset + words > room) {
            continue;
        }

        for (uint32_t word = 0; word < words; word++) {
            uint32_t first = window[at + offset + word];
            uint32_t second = window[at + stride + offset + word];
            uint32_t third = window[at + 2u * stride + offset + word];
            float near, middle, far;
            memcpy(&near, &first, sizeof near);
            memcpy(&middle, &second, sizeof middle);
            memcpy(&far, &third, sizeof far);

            float across = near - middle + far;
            uint32_t fourth;
            memcpy(&fourth, &across, sizeof fourth);

            window[at + 3u * stride + offset + word] = first;
            window[at + 4u * stride + offset + word] = third;
            window[at + 5u * stride + offset + word] = fourth;
            if (at + 5u * stride + offset + word + 1u > reached) {
                reached = at + 5u * stride + offset + word + 1u;
            }
        }
    }
    return reached;
}

static uint32_t expand_the_quads(uint32_t *window, uint32_t room, uint32_t filled,
                                 const uint32_t *reads, uint32_t streams, uint32_t quads) {
    static const uint32_t order[6] = {0u, 1u, 2u, 0u, 2u, 3u};
    uint32_t done[XENOLITH_FETCH_SLOTS];
    uint32_t doing = 0;

    for (uint32_t one = 0; one < streams; one++) {
        uint32_t slot = reads[one * 4];
        uint32_t stride = reads[one * 4 + 1];
        if (slot >= XENOLITH_FETCH_SLOTS || stride == 0) {
            continue;
        }

        uint32_t already = 0;
        for (uint32_t at = 0; at < doing; at++) {
            already = already || done[at] == slot;
        }
        if (already) {
            continue;
        }
        done[doing++] = slot;

        uint32_t from = window[slot];
        if (from == 0) {
            continue;
        }
        uint32_t needs = quads * 6u * stride;
        if (needs == 0 || needs > room - filled) {
            continue;
        }

        for (uint32_t quad = 0; quad < quads; quad++) {
            for (uint32_t corner = 0; corner < 6u; corner++) {
                uint32_t took = from + (quad * 4u + order[corner]) * stride;
                uint32_t puts = filled + (quad * 6u + corner) * stride;

                /* Whole, where it is inside what there is and the words it
                 * comes from are not the words it goes to. A shader that says
                 * it reads fewer words than there are corners leaves the two
                 * overlapping, and then it has to go word by word. */
                if (took + stride <= room && puts >= took + stride) {
                    memcpy(&window[puts], &window[took], (size_t)stride * sizeof *window);
                    continue;
                }
                for (uint32_t word = 0; word < stride; word++) {
                    window[puts + word] = took + word < room ? window[took + word] : 0u;
                }
            }
        }
        window[slot] = filled;
        filled += needs;
    }
    return filled;
}

static void one_corner_over(uint32_t *window, uint32_t room, uint32_t from, uint32_t filled,
                            uint32_t stride, uint32_t put_at, uint32_t which) {
    uint32_t took = from + which * stride;
    uint32_t puts = filled + put_at * stride;

    /* Where the corner is inside what there is and the words it is taken from
     * are not the words it is put in, it goes whole.
     *
     * Which corner a title names is its own number, so it can name one that has
     * not been read, and where that lands among the words being written the
     * copy has to stay word by word: taking it whole would give the words as
     * they were rather than as they are being made, which is not what a corner
     * put over itself a word at a time comes to. */
    if (took + stride <= room && puts >= took + stride) {
        memcpy(&window[puts], &window[took], (size_t)stride * sizeof *window);
        return;
    }
    for (uint32_t word = 0; word < stride; word++) {
        window[puts + word] = (took + word < room && puts + word < room) ? window[took + word] : 0u;
    }
}

static uint32_t a_corner_named(const uint8_t *base, uint32_t listed_at, int wide, uint32_t which) {
    if (wide) {
        return xenolith_load32(base, listed_at + which * 4u);
    }
    uint32_t pair = xenolith_load32(base, listed_at + (which / 2u) * 4u);
    return (which % 2u) == 0u ? (pair >> 16) : (pair & 0xffffu);
}

static uint32_t breaks_a_run(int wide) {
    return wide ? 0xffffffffu : 0xffffu;
}

static void look_over_the_list(const uint8_t *base, uint32_t listed_at, int wide,
                               uint32_t listed_count, uint32_t *comes_to, uint32_t *furthest) {
    uint32_t breaks = breaks_a_run(wide);
    uint32_t most = 0;
    uint32_t held = 0;
    uint32_t made = 0;

    for (uint32_t at = 0; at < listed_count; at++) {
        uint32_t which = a_corner_named(base, listed_at, wide, at);
        if (which == breaks) {
            held = 0;
            continue;
        }
        if (which > most) {
            most = which;
        }
        if (held < 2u) {
            held++;
            continue;
        }
        made += 3u;
        held++;
    }

    if (comes_to != NULL) {
        *comes_to = made;
    }
    if (furthest != NULL) {
        *furthest = most;
    }
}

static uint32_t order_the_corners(const uint8_t *base, uint32_t *window, uint32_t room,
                                  uint32_t filled, const uint32_t *reads, uint32_t streams,
                                  uint32_t corners, uint32_t listed_at, uint32_t listed_count,
                                  int wide, int a_run, uint32_t adds) {
    uint32_t done[XENOLITH_FETCH_SLOTS];
    uint32_t doing = 0;
    uint32_t breaks = breaks_a_run(wide);

    for (uint32_t one = 0; one < streams; one++) {
        uint32_t slot = reads[one * 4];
        uint32_t stride = reads[one * 4 + 1];
        if (slot >= XENOLITH_FETCH_SLOTS || stride == 0) {
            continue;
        }

        uint32_t already = 0;
        for (uint32_t at = 0; at < doing; at++) {
            already = already || done[at] == slot;
        }
        if (already) {
            continue;
        }
        done[doing++] = slot;

        uint32_t from = window[slot];
        if (from == 0) {
            continue;
        }
        uint32_t wants = corners;
        uint32_t needs = wants * stride;
        if (needs == 0 || needs > room - filled) {
            continue;
        }

        uint32_t put_at = 0;
        uint32_t held[3] = {0, 0, 0};
        uint32_t holding = 0;
        uint32_t turned_over = 0;

        for (uint32_t corner = 0; corner < listed_count; corner++) {
            uint32_t which = a_corner_named(base, listed_at, wide, corner);

            if (put_at >= wants) {
                break;
            }
            if (!a_run) {
                one_corner_over(window, room, from, filled, stride, put_at++, which + adds);
                continue;
            }

            if (which == breaks) {
                holding = 0;
                turned_over = 0;
                continue;
            }
            if (holding < 2u) {
                held[holding++] = which;
                continue;
            }
            held[2] = which;

            /* Every other triangle in a run is wound the other way, and a
             * runtime that puts them all out the same way has half of them
             * facing backwards, which is half a surface missing wherever a
             * title turns its backs away. */
            uint32_t first = turned_over ? held[1] : held[0];
            uint32_t second = turned_over ? held[0] : held[1];
            one_corner_over(window, room, from, filled, stride, put_at++, first + adds);
            one_corner_over(window, room, from, filled, stride, put_at++, second + adds);
            one_corner_over(window, room, from, filled, stride, put_at++, held[2] + adds);

            held[0] = held[1];
            held[1] = held[2];
            turned_over = !turned_over;
        }
        window[slot] = filled;
        filled += needs;
    }
    return filled;
}

static void say_what_a_picture_holds(uint32_t place, const uint8_t *base, uint32_t where,
                                     uint32_t across, uint32_t down, uint32_t format,
                                     uint32_t held_across, uint32_t held_down,
                                     uint32_t read_how) {
    static int telling = -1;
    static uint32_t said[XENOLITH_PICTURES_SHOWN];
    static uint32_t shown;

    if (telling < 0) {
        telling = getenv("XENOLITH_TRACE_DRAWS") != NULL;
    }
    if (!telling || shown >= XENOLITH_PICTURES_SHOWN) {
        return;
    }
    /* Once for each place read from rather than the first few handed over. A
     * title hands the same picture over again every frame, so counting them
     * says nothing about how many different ones there are. */
    for (uint32_t at = 0; at < shown; at++) {
        if (said[at] == where) {
            return;
        }
    }
    said[shown++] = where;

    uint32_t looked = across * down / 4u;
    if (looked > 4096u) {
        looked = 4096u;
    }
    uint32_t anything = 0;
    for (uint32_t step = 0; step < looked; step++) {
        if (xenolith_load32(base, where + step * 4u) != 0) {
            anything++;
        }
    }
    fprintf(stderr,
            "xenolith: picture in place %u at %08x, %ux%u, kind %u, held to %u and %u, read %u "
            "larger and %u smaller, %u of %u words hold anything\n",
            place, where, across, down, format, held_across, held_down, read_how & 3u,
            (read_how >> 2) & 3u, anything, looked);

    /* What one holds, where a run says to put it, so that a picture that came
     * out wrong can be looked at rather than reasoned about. Only the ones kept
     * a byte to a part are written, because those are the ones a run has had to
     * ask about. */
    const char *into = getenv("XENOLITH_KEEP_PICTURES");
    if (into == NULL || format != XENOLITH_PICTURE_IN_PARTS) {
        return;
    }
    char name[512];
    if (snprintf(name, sizeof name, "%s/%08x.pgm", into, where) >= (int)sizeof name) {
        return;
    }
    FILE *out = fopen(name, "wb");
    if (out == NULL) {
        return;
    }
    fprintf(out, "P5\n%u %u\n255\n", across, down);
    for (uint32_t step = 0; step < across * down; step++) {
        fputc(xenolith_load8(base, where + step), out);
    }
    fclose(out);
}

static int only_this_shader_draws(uint32_t sum) {
    static int settled = -1;
    static uint32_t wanted;

    if (settled < 0) {
        const char *asked = getenv("XENOLITH_ONLY_SHADER");
        wanted = asked != NULL ? (uint32_t)strtoul(asked, NULL, 16) : 0;
        settled = 1;
    }
    return wanted != 0 && sum != wanted;
}

static void say_a_draw_had_no_shader(uint32_t vertex_shader, uint32_t pixel_shader, int no_vertex,
                                     int no_pixel) {
    static uint32_t said[8][2];
    static unsigned saying;
    static unsigned long long often;

    often++;
    for (unsigned at = 0; at < saying; at++) {
        if (said[at][0] == vertex_shader && said[at][1] == pixel_shader) {
            return;
        }
    }
    if (saying >= 8u) {
        return;
    }
    said[saying][0] = vertex_shader;
    said[saying][1] = pixel_shader;
    saying++;

    fprintf(stderr,
            "xenolith: a draw names %08x/%08x and is passed over, because nothing was made of "
            "the one that runs for each %s%s%s. %llu draws so far\n",
            vertex_shader, pixel_shader, no_vertex ? "vertex" : "",
            no_vertex && no_pixel ? " and each " : "", no_pixel ? "pixel" : "", often);
}

static void hand_over_the_pictures(const uint8_t *base, const uint32_t *looks,
                                   uint32_t pictures) {
    for (uint32_t one = 0; one < pictures; one++) {
        uint32_t place = looks[one];
        if (place >= XENOLITH_PICTURE_PLACES) {
            continue;
        }
        uint32_t at = XENOLITH_REGISTERS + (XENOLITH_PICTURE_REGISTER + place * 6u) * 4u;
        uint32_t first = xenolith_load32(base, at);
        if ((first & 3u) != XENOLITH_PLACE_HOLDS_A_PICTURE) {
            continue;
        }

        uint32_t second = xenolith_load32(base, at + 4u);
        uint32_t format = second & 0x3fu;
        if (format != XENOLITH_PICTURE_IN_SQUARES && format != XENOLITH_PICTURE_IN_SQUARES_PLAIN
            && format != XENOLITH_PICTURE_IN_SQUARES_SMALL && format != XENOLITH_PICTURE_IN_PARTS
            && format != XENOLITH_PICTURE_IN_FOURS && format != XENOLITH_PICTURE_IN_FLOATS
            && format != XENOLITH_PICTURE_HOW_FAR
            && format != XENOLITH_PICTURE_HOW_FAR_AS_A_FLOAT) {
            /* Left holding nothing rather than left as it was. A place a title
             * has filled with something this cannot read still gets read by the
             * shader about to draw, and what was in the place before is another
             * of the title's own pictures, so leaving it there draws the wrong
             * picture with nothing anywhere saying so. */
            said_nothing_of_a_picture(format);
            xenolith_look_at_nothing(place);
            continue;
        }

        uint32_t third = xenolith_load32(base, at + 8u);
        /* How a title asks for a picture to be read: by the part nearest what
         * is being drawn, or by mixing the parts around it. Two bits each for
         * making one larger and for making one smaller. */
        uint32_t fourth = xenolith_load32(base, at + 12u);
        uint32_t read_how = ((fourth >> 19) & 3u) | (((fourth >> 21) & 3u) << 2);
        if (place == 0) {
            drawn_looks_at = (second >> 12) << 12;
            drawn_looks_at_size = ((third & 0x1fffu) + 1u) << 16 | (((third >> 13) & 0x1fffu) + 1u);
        }
        say_what_a_picture_holds(place, base, (second >> 12) << 12, (third & 0x1fffu) + 1u,
                                 ((third >> 13) & 0x1fffu) + 1u, format, (first >> 10) & 7u,
                                 (first >> 13) & 7u, read_how);
        /* Which part of the picture each of the four a shader reads is, three
         * bits each above the bit saying how a number in it is kept. */
        uint32_t parts = (fourth >> 1) & 0xfffu;
        xenolith_look_at_a_picture(place, base, (second >> 12) << 12, (third & 0x1fffu) + 1u,
                                   ((third >> 13) & 0x1fffu) + 1u, ((first >> 22) & 0x1ffu) * 32u,
                                   (int)((first >> 31) & 1u), (second >> 6) & 3u, format,
                                   ((first >> 10) & 7u) | (((first >> 13) & 7u) << 4)
                                       | (read_how << 8) | (parts << 12));
    }
}

static void say_where_a_shape_reads_from(uint32_t slot, uint32_t where, uint32_t words,
                                         const uint32_t *read, uint32_t shape) {
    static int telling = -1;
    static uint32_t shown;

    if (telling < 0) {
        telling = getenv("XENOLITH_TRACE_DRAWS") != NULL;
    }
    if (!telling || shown >= XENOLITH_SLOTS_SHOWN || shape != 13u) {
        return;
    }
    shown++;

    /* How many of the words are anything at all, counted here rather than while
     * they are read. A draw reads as many words as it has corners and this is
     * wanted for a handful of draws in a whole run, so counting them as they go
     * past puts the cost of a report on every draw that never makes one. */
    uint32_t anything = 0;
    for (uint32_t step = 0; step < words; step++) {
        if (read[step] != 0) {
            anything++;
        }
    }
    fprintf(stderr, "xenolith: shape %u reads slot %u from %08x, %u words, %u of them anything\n",
            shape, slot, where, words, anything);
}

static void say_what_is_being_drawn_from(const uint32_t *window, uint32_t filled, uint32_t room,
                                         uint32_t vertices, uint32_t shape, const uint32_t *reads,
                                         uint32_t streams, uint32_t by) {
    static int telling = -1;
    static uint32_t said[8];
    static uint32_t saying;

    if (telling < 0) {
        telling = getenv("XENOLITH_TRACE_DRAWS") != NULL;
    }
    /* The three a scene is built out of: the quads and rectangles a title
     * composites with, and the runs of triangles its world is made of. */
    if (!telling || (shape != 13u && shape != 8u && shape != 6u)) {
        return;
    }
    /* Either once for each shader, or every draw of one frame where a run names
     * the frame. The first says what a shader draws; the second says whether
     * what it draws changes from one draw to the next, which is what separates
     * a shape read wrongly from a shape that is the same every time. */
    static long which_frame = -2;
    if (which_frame == -2) {
        const char *asked = getenv("XENOLITH_CORNERS_AT");
        which_frame = asked != NULL ? (long)strtol(asked, NULL, 0) : -1;
    }
    if (which_frame >= 0) {
        if ((long)graphics.frames != which_frame) {
            return;
        }
    } else {
        for (uint32_t at = 0; at < saying; at++) {
            if (said[at] == by) {
                return;
            }
        }
        if (saying >= 8u) {
            return;
        }
        said[saying++] = by;
    }
    fprintf(stderr, "xenolith: drawn by %08x, ", by);

    uint32_t nothing = 0;
    for (uint32_t at = XENOLITH_FETCH_SLOTS; at < filled; at++) {
        if (window[at] == 0) {
            nothing++;
        }
    }
    fprintf(stderr, "xenolith: shape %u of %u corners, %u words of %u, %u nought, %u streams",
            shape, vertices, filled - XENOLITH_FETCH_SLOTS, room, nothing, streams);
    for (uint32_t one = 0; one < streams && reads != NULL; one++) {
        uint32_t slot = reads[one * 4];
        uint32_t stride = reads[one * 4 + 1];
        uint32_t offset = reads[one * 4 + 2];
        uint32_t wide = reads[one * 4 + 3];
        fprintf(stderr, "\n  stream %u: slot %u at %u stride %u offset %u wide %u, corners:", one,
                slot, slot < XENOLITH_FETCH_SLOTS ? window[slot] : 0u, stride, offset, wide);
        /* A batch carries many shapes and the first of them is the one least
         * worth seeing: a shape read from the wrong place still gives the first
         * shape right and every one after it the same. Where a run names a
         * frame it is asking to be shown that, so it is shown enough corners to
         * tell. */
        uint32_t shown = which_frame >= 0 ? 96u : 8u;
        for (uint32_t corner = 0; corner < vertices && corner < shown; corner++) {
            fprintf(stderr, " (");
            for (uint32_t part = 0; part < wide && part < 4u; part++) {
                uint32_t where = window[slot] + offset + corner * stride + part;
                if (slot >= XENOLITH_FETCH_SLOTS || window[slot] == 0 || where >= room) {
                    fprintf(stderr, "?");
                    continue;
                }
                float value;
                uint32_t word = window[where];
                memcpy(&value, &word, sizeof value);
                fprintf(stderr, "%s%g", part != 0 ? "," : "", (double)value);
            }
            fprintf(stderr, ")");
        }
    }
    fprintf(stderr, "\n");
}

static uint32_t what_a_shader_reads_of(const uint32_t *reads, uint32_t streams, uint32_t slot,
                                       uint32_t vertices, int *reads_it, uint32_t *apart) {
    uint32_t most = 0;

    *reads_it = 0;
    *apart = 0;
    for (uint32_t one = 0; one < streams; one++) {
        if (reads[one * 4] != slot) {
            continue;
        }
        *reads_it = 1;
        if (reads[one * 4 + 1] > *apart) {
            *apart = reads[one * 4 + 1];
        }
        uint32_t wanted =
            vertices * reads[one * 4 + 1] + reads[one * 4 + 2] + reads[one * 4 + 3] + 1u;
        if (wanted > most) {
            most = wanted;
        }
    }
    return most;
}

static void hand_over_the_vertices(const uint8_t *base, uint32_t vertices, const uint32_t *reads,
                                   uint32_t streams, uint32_t shape, uint32_t laid_out_as,
                                   const uint32_t *declared, uint32_t declared_streams, uint32_t by,
                                   uint32_t listed_at, uint32_t listed_count, int listed_wide,
                                   int a_run, uint32_t furthest) {
    uint32_t room = 0;
    uint32_t *window = xenolith_room_for_vertices(&room);
    if (window == NULL || room <= XENOLITH_FETCH_SLOTS) {
        return;
    }

    /* Which vertex of the fetch this draw's first one is.
     *
     * A title that draws its interface out of one buffer a frame writes every
     * shape into it one after another and then draws each in turn, saying here
     * where the one it means begins. The fetch itself does not move, so a
     * runtime that reads this as nothing gives every draw the first shape's
     * corners: every quad comes out with the first quad's picture across the
     * whole of it, however right everything else is. */
    uint32_t first_vertex = register_says(base, XENOLITH_FIRST_VERTEX);


    uint32_t filled = XENOLITH_FETCH_SLOTS;
    for (uint32_t slot = 0; slot < XENOLITH_FETCH_SLOTS; slot++) {
        window[slot] = 0;

        uint32_t low = xenolith_load32(
            base, XENOLITH_REGISTERS + (XENOLITH_FETCH_REGISTER + slot * 2u) * 4u);
        uint32_t high = xenolith_load32(
            base, XENOLITH_REGISTERS + (XENOLITH_FETCH_REGISTER + slot * 2u + 1u) * 4u);
        if ((low & 3u) != XENOLITH_FETCH_IS_VERTICES) {
            continue;
        }

        uint32_t where = low & ~3u;
        uint32_t words = (high >> 2) & 0xffffffu;

        /* A fetch that says it holds vertices but names nowhere is one a title
         * set the kind of and never the place of. Reading from the bottom of
         * memory gives whatever happens to be there and hands it to a shader as
         * though it were a shape. */
        if (where == 0) {
            continue;
        }

        /* As much of this fetch as the shader reads, where the shader says, and
         * otherwise the widest a vertex could be. The second is what a fetch's
         * stride is counted in and how far apart two of them can sit. */
        uint32_t reach = vertices * 255u + 256u;
        uint32_t apart = 0;
        if (declared != NULL && declared_streams != 0) {
            int reads_it = 0;
            uint32_t wanted = what_a_shader_reads_of(declared, declared_streams, slot, vertices,
                                                     &reads_it, &apart);
            if (!reads_it) {
                continue;
            }
            reach = wanted;
            if (listed_at != 0 && apart != 0) {
                uint32_t past = (furthest + first_vertex + 1u) * apart;
                if (past > reach) {
                    reach = past;
                }
            }
        }

        /* Past the shapes drawn before this one. How far that is in words is
         * how far apart two vertices are, which the shader says and the fetch
         * does not, so a fetch no shader reads is left where it is. */
        /* Where the corners are named, the numbers are counted from the start
         * of the fetch, so nothing is skipped here and where a draw begins is
         * added to each number instead. */
        uint32_t past = listed_at != 0 ? 0u : first_vertex * apart;
        if (past >= words) {
            continue;
        }
        where += past * 4u;
        words -= past;

        if (words > reach) {
            words = reach;
        }
        if (words > room - filled) {
            words = room - filled;
        }
        if (words == 0) {
            continue;
        }

        window[slot] = filled;
        for (uint32_t step = 0; step < words; step++) {
            window[filled + step] = xenolith_load32(base, where + step * 4u);
        }
        say_where_a_shape_reads_from(slot, where, words, window + filled, shape);
        drawn_reads_from = where;
        drawn_reads_slot = slot;
        filled += words;
    }

    if (reads != NULL && shape == XENOLITH_SHAPE_RECTANGLES) {
        uint32_t reached = complete_the_rectangle(window, room, reads, streams);
        if (reached > filled) {
            filled = reached;
        }
    }
    if (declared != NULL && declared_streams != 0 && listed_at != 0) {
        filled = order_the_corners(base, window, room, filled, declared, declared_streams, vertices,
                                   listed_at, listed_count, listed_wide, a_run, first_vertex);
    }
    if (reads != NULL && shape == XENOLITH_SHAPE_QUADS
        && laid_out_as == XENOLITH_SHAPE_TRIANGLES) {
        filled = expand_the_quads(window, room, filled, reads, streams, vertices / 6u);
    }

    say_what_is_being_drawn_from(window, filled, room, vertices, shape, declared, declared_streams,
                                 by);
    xenolith_done_with_vertices(filled);
}

static int say_it_now(const uint8_t *base, uint32_t which_kind, uint32_t where,
                      uint32_t how_many, uint32_t sum) {
#ifndef XENOLITH_CAN_SAY_A_SHADER
    (void)base;
    (void)which_kind;
    (void)where;
    (void)how_many;
    (void)sum;
    return 0;
#else
    /* One that could not be said is kept too, with no answer, so that it is
     * not tried again every time a title hands it over. */
    static struct {
        uint32_t kind;
        uint32_t sum;
        uint32_t words;
        uint32_t *answer;
    } kept[XENOLITH_SAID_WHILE_RUNNING];
    static unsigned how_many_kept;

    uint32_t *answer = NULL;
    uint32_t *handed;
    uint32_t wanted;
    uint32_t length;

    for (unsigned at = 0; at < how_many_kept; at++) {
        if (kept[at].kind == which_kind && kept[at].sum == sum && kept[at].words == how_many) {
            if (kept[at].answer == NULL) {
                return 0;
            }
            answer = kept[at].answer;
            break;
        }
    }

    if (answer == NULL) {
        /* The words as the guest holds them, which are the other way round from
         * the way this machine reads one. */
        handed = malloc((size_t)how_many * 4u);
        if (handed == NULL) {
            return 0;
        }
        for (uint32_t k = 0; k < how_many; k++) {
            handed[k] = xenolith_load32(base, where + k * 4);
        }

        say_what_a_shader_was(base, sum, where, how_many);
        wanted = xenolith_say_a_shader(which_kind, handed, how_many, NULL, 0);
        if (wanted == 0) {
            if (getenv("XENOLITH_DUMP_FAILED_SHADER") != NULL) {
                fprintf(stderr, "XSHADER kind=%u words=%u:", which_kind, how_many);
                for (uint32_t k = 0; k < how_many; k++) {
                    fprintf(stderr, " %08x", handed[k]);
                }
                fprintf(stderr, "\n");
            }
            free(handed);
            if (how_many_kept < XENOLITH_SAID_WHILE_RUNNING) {
                kept[how_many_kept].kind = which_kind;
                kept[how_many_kept].sum = sum;
                kept[how_many_kept].words = how_many;
                kept[how_many_kept].answer = NULL;
                how_many_kept++;
            }
            return 0;
        }

        answer = malloc((size_t)wanted * 4u);
        if (answer == NULL || xenolith_say_a_shader(which_kind, handed, how_many, answer, wanted)
                                  != wanted) {
            free(handed);
            free(answer);
            return 0;
        }
        free(handed);

        if (how_many_kept < XENOLITH_SAID_WHILE_RUNNING) {
            kept[how_many_kept].kind = which_kind;
            kept[how_many_kept].sum = sum;
            kept[how_many_kept].words = how_many;
            kept[how_many_kept].answer = answer;
            how_many_kept++;
            said_one_while_running(which_kind, how_many);
        }
    }

    length = answer[0];
#ifdef XENOLITH_CAN_SAY_A_SHADER
    say_what_a_shader_became(sum, answer + XENOLITH_COUNTS, length);
#endif
    in_hand[which_kind].sum = sum;
    in_hand[which_kind].words = how_many;
    in_hand[which_kind].said = answer + XENOLITH_COUNTS;
    in_hand[which_kind].length = length;
    in_hand[which_kind].streams = answer[1];
    in_hand[which_kind].reads = answer[1] != 0 ? answer + XENOLITH_COUNTS + length : NULL;
    in_hand[which_kind].pictures = answer[2];
    in_hand[which_kind].looks =
        answer[2] != 0 ? answer + XENOLITH_COUNTS + length + answer[1] * XENOLITH_WORDS_A_STREAM
                       : NULL;
    return 1;
#endif
}

static void take_the_shader_in_hand(uint8_t *base, uint32_t which_kind, uint32_t where,
                                    uint32_t how_many) {
    uint32_t sum = 0x811c9dc5u;

    if (which_kind > 1) {
        return;
    }

    for (uint32_t k = 0; k < how_many; k++) {
        uint32_t word = xenolith_load32(base, where + k * 4);
        uint32_t length = 0;
        const uint32_t *said;
        for (int byte = 3; byte >= 0; byte--) {
            sum ^= (word >> (byte * 8)) & 0xffu;
            sum *= 0x01000193u;
        }
        said = xenolith_said(which_kind, sum, k + 1, &length);
        if (said != NULL) {
            say_what_a_shader_was(base, sum, where, k + 1);
            in_hand[which_kind].sum = sum;
            in_hand[which_kind].words = k + 1;
            in_hand[which_kind].said = said;
            in_hand[which_kind].length = length;
            in_hand[which_kind].reads =
                xenolith_reads(which_kind, sum, k + 1, &in_hand[which_kind].streams);
            in_hand[which_kind].looks =
                xenolith_looks_at(which_kind, sum, k + 1, &in_hand[which_kind].pictures);
            return;
        }
    }

    if (say_it_now(base, which_kind, where, how_many, sum)) {
        return;
    }

    /* Nothing in hand rather than the one before it. A draw goes ahead with
     * whatever is in hand, and the one before is another of the title's
     * shaders, which draws something wrong in place of nothing. */
    in_hand[which_kind].sum = sum;
    in_hand[which_kind].words = how_many;
    in_hand[which_kind].said = NULL;
    said_nothing_of_a_shader(sum, how_many);
}

/* Draws what a title asked to draw, with the shaders it last handed over.
 *
 * A title does not name its shaders in the draw. It hands them over and then
 * draws, so what is used is whichever pair was last set up.
 *
 * Kept apart from the walk because it is the one command whose effect is a
 * whole subsystem's worth of work, and reading it inline buries the walk.
 */
/* The registers a copy out of the hardware is described by, by the console's
 * numbering: the mode that says a draw is a copy, what to copy and whether to
 * wipe it, where to, how wide and tall, what shape the copy takes, and what
 * the wipes start again as. */
#define XENOLITH_EDRAM_MODE 0x2208u
#define XENOLITH_EDRAM_COPIES 6u
#define XENOLITH_COPY_CONTROL 0x2318u
#define XENOLITH_COPY_DEST_BASE 0x2319u
#define XENOLITH_COPY_DEST_PITCH 0x231au
#define XENOLITH_COPY_DEST_INFO 0x231bu
#define XENOLITH_COPY_SWAPS_RED_AND_BLUE (1u << 24)
#define XENOLITH_DEPTH_CLEAR 0x231du

/* How the place a title keeps how far away things are holds it, and the bit of
 * that saying it holds the console's small float rather than a fraction of the
 * whole. A copy hands it back as it is held, whatever kind of picture the copy
 * goes into, so the title reads it back the way it was drawn. */
#define XENOLITH_HOW_FAR_IS_KEPT 0x2002u
#define XENOLITH_KEPT_AS_A_FLOAT (1u << 16)
#define XENOLITH_COLOUR_CLEAR 0x231eu

/* Which of what was drawn a copy takes when it takes how far away things are
 * rather than a colour. */
#define XENOLITH_COPY_FROM_HOW_FAR 4u

static uint32_t a_register(const uint8_t *base, uint32_t which) {
    return xenolith_load32(base, XENOLITH_REGISTERS + which * 4u);
}

/* Where a copy's rectangle is said: the first fetch, which a title points at
 * three of its corners, two floats each, before drawing it. */
#define XENOLITH_COPY_CORNERS 0x4800u

static int32_t a_corner_rounded(float at) { return (int32_t)floorf(at + 0.5f); }

/* The rectangle a copy takes, in the place drawn into, kept to what the copy
 * is kept to. A rectangle reaching nowhere comes out empty. */
static void what_a_copy_takes(const uint8_t *base, struct xenolith_copy *copy) {
    uint32_t corners = a_register(base, XENOLITH_COPY_CORNERS) & ~3u;
    float at[6] = {0};
    if (corners != 0) {
        for (unsigned int one = 0; one < 6u; one++) {
            uint32_t word = xenolith_load32(base, corners + one * 4u);
            memcpy(&at[one], &word, sizeof at[one]);
        }
    }
    float least_across = fminf(fminf(at[0], at[2]), at[4]);
    float most_across = fmaxf(fmaxf(at[0], at[2]), at[4]);
    float least_down = fminf(fminf(at[1], at[3]), at[5]);
    float most_down = fmaxf(fmaxf(at[1], at[3]), at[5]);

    int32_t across, down;
    how_far_corners_move(base, &across, &down);
    int32_t left = a_corner_rounded(least_across) + across;
    int32_t right = a_corner_rounded(most_across) + across;
    int32_t top = a_corner_rounded(least_down) + down;
    int32_t bottom = a_corner_rounded(most_down) + down;

    int32_t kept_from_left, kept_from_top, kept_to_right, kept_to_bottom;
    where_a_draw_is_kept(base, &kept_from_left, &kept_from_top, &kept_to_right, &kept_to_bottom);
    left = left > kept_from_left ? left : kept_from_left;
    top = top > kept_from_top ? top : kept_from_top;
    right = right < kept_to_right ? right : kept_to_right;
    bottom = bottom < kept_to_bottom ? bottom : kept_to_bottom;

    copy->left = not_below_nought(left);
    copy->top = not_below_nought(top);
    copy->right = not_below_nought(right);
    copy->bottom = not_below_nought(bottom);
}

/* Carries out a copy out of the hardware, which a title asks for as a draw.
 *
 * The draw is a rectangle over what is to be copied, made while the mode says
 * copy rather than draw, and drawing it as a rectangle puts a flat shape over
 * the frame where the copy should have gone. So it is not drawn. What it asks
 * for is done instead: what was drawn is finished and handed back to the
 * address the title names, in the shape it names, and wiped afterwards if the
 * title said to, and any picture already made of that memory is forgotten so
 * the next draw that reads it sees the copy.
 */
static void copy_what_was_drawn(uint8_t *base) {
    uint32_t control = a_register(base, XENOLITH_COPY_CONTROL);
    uint32_t size = a_register(base, XENOLITH_COPY_DEST_PITCH);
    uint32_t format = (a_register(base, XENOLITH_COPY_DEST_INFO) >> 7) & 0x3fu;
    uint32_t samples = (a_register(base, 0x2000u) >> XENOLITH_SAMPLES_SHIFT) & 3u;

    struct xenolith_copy copy = {0};
    copy.into = a_register(base, XENOLITH_COPY_DEST_BASE);
    copy.pitch = size & 0x3fffu;
    copy.height = (size >> 16) & 0x3fffu;
    copy.samples_across = samples == 2u ? 2u : 1u;
    copy.samples_down = samples != 0u ? 2u : 1u;
    copy.as_one_float = format == XENOLITH_PICTURE_IN_FLOATS;
    copy.red_and_blue_swapped =
        (a_register(base, XENOLITH_COPY_DEST_INFO) & XENOLITH_COPY_SWAPS_RED_AND_BLUE) != 0;
    copy.from_how_far = (control & 7u) == XENOLITH_COPY_FROM_HOW_FAR;
    copy.how_far_as_a_float =
        (a_register(base, XENOLITH_HOW_FAR_IS_KEPT) & XENOLITH_KEPT_AS_A_FLOAT) != 0;
    what_a_copy_takes(base, &copy);

    /* Copied out of the place the registers name, with its samples laid out
     * the way they say, which is not always how the last draw left it: a title
     * draws its glows with four samples to a pixel and copies the same memory
     * out with one. */
    start_the_frame_in_its_place(base);
    finish_the_frame();
    if (copy.into != 0 && copy.pitch != 0 && copy.height != 0) {
        xenolith_hand_back_what_was_drawn(base, &copy);
        xenolith_forget_pictures_at(copy.into, ((copy.pitch + 31u) & ~31u)
                                                   * ((copy.bottom + 31u) & ~31u) * 4u);
    }

    uint32_t depth = a_register(base, XENOLITH_DEPTH_CLEAR);
    xenolith_wipe_the_colours_after_a_copy((control >> 8) & 1u,
                                           a_register(base, XENOLITH_COLOUR_CLEAR));
    xenolith_wipe_how_far_after_a_copy((control >> 9) & 1u,
                                       (float)(depth >> 8) / 16777215.0f);
}

static void draw_what_was_asked_for(uint8_t *base, uint32_t stream, uint32_t at,
                                    uint32_t follows, uint32_t opcode, uint32_t header) {
        if ((opcode == XENOLITH_PACKET_DRAW || opcode == XENOLITH_PACKET_DRAW_INDEXED)
            && (a_register(base, XENOLITH_EDRAM_MODE) & 7u) == XENOLITH_EDRAM_COPIES) {
            copy_what_was_drawn(base);
            return;
        }

        /* A draw, with whatever pair of shaders was last handed over.
         *
         * That is how a title says which to use: it hands them over and
         * then draws, rather than naming them in the draw itself.
         *
         * What came of it is said once. A title draws tens of times a frame
         * and a line for each would say nothing the first one did not. */
        if ((opcode == XENOLITH_PACKET_DRAW || opcode == XENOLITH_PACKET_DRAW_INDEXED)
            && follows >= 1 && (in_hand[0].said == NULL || in_hand[1].said == NULL)) {
            say_a_draw_had_no_shader(in_hand[0].sum, in_hand[1].sum, in_hand[0].said == NULL,
                                     in_hand[1].said == NULL);
        }

        if ((opcode == XENOLITH_PACKET_DRAW || opcode == XENOLITH_PACKET_DRAW_INDEXED)
            && in_hand[0].said != NULL && in_hand[1].said != NULL && follows >= 1) {
            /* How many vertices, which the two draws do not keep in the
             * same place. The one that fetches its own indices puts a word
             * about counting what was drawn in front of it, so the word
             * this is read out of is the second rather than the first, and
             * reading the first gives a count taken from a word that is not
             * a count at all. */
            say_whether_a_draw_was_asked_about(header);
            uint32_t which_word = opcode == XENOLITH_PACKET_DRAW_INDEXED ? 2u : 1u;
            uint32_t how = xenolith_load32(base, stream + (at + which_word) * 4);

            /* Where a title's own list of which corners each shape is made
             * of sits, for the draws that name one. The two bits saying
             * where a draw takes its order from say nought for a list kept
             * in memory, and the bit above the count says whether each
             * number is four bytes rather than two. */
            uint32_t listed_at = 0;
            uint32_t listed_count = 0;
            uint32_t comes_to = 0;
            uint32_t furthest = 0;
            int listed_wide = 0;
            if (opcode == XENOLITH_PACKET_DRAW_INDEXED && ((how >> 6) & 3u) == 0u
                && follows >= 4) {
                listed_at = xenolith_load32(base, stream + (at + 3) * 4) & ~3u;
                listed_wide = (int)((how >> 11) & 1u);
                listed_count = how >> 16;
                look_over_the_list(base, listed_at, listed_wide, listed_count, &comes_to,
                                   &furthest);
            }
            uint32_t vertices = how >> 16;
            uint32_t shape = how & 0x3fu;
            drawn_how = how;
            drawn_from_a_packet = opcode;

            /* A rectangle arrives as three of its corners, the hardware
             * having worked out the fourth. A host draws it as the two
             * triangles that shape comes to, so twice as many vertices are
             * drawn and the shader is asked where it reads them from. */
            const uint32_t *reads = NULL;
            uint32_t streams = 0;
            uint32_t laid_out_as = shape;
            if (shape == XENOLITH_SHAPE_RECTANGLES) {
                reads = in_hand[0].reads;
                streams = in_hand[0].streams;
                if (reads != NULL) {
                    vertices *= 2u;
                }
            }

            /* A run of quads is one shape for every four corners, and the
             * nearest a host has is a fan, which is one shape however many
             * corners it is given. So each quad's two triangles are written
             * out and what is drawn is triangles.
             *
             * Corners left over past the last whole four are dropped, which
             * is what the hardware does with them: a quad needs four and
             * there are not four left. Asking for a count that divides by
             * four instead, and leaving the rest alone where it does not,
             * left nearly every quad a scene draws going out as a fan over
             * its raw corners, which is a different shape entirely. This
             * title asks for six corners far more often than any other
             * number, so that was almost all of them. */
            /* A run of triangles whose corners are named is written out as
             * separate triangles, so that is what is drawn. How many there
             * are is worked out where they are written. */
            if (shape == XENOLITH_SHAPE_TRIANGLE_RUN && listed_at != 0
                && in_hand[0].reads != NULL) {
                if (comes_to != 0) {
                    vertices = comes_to;
                    laid_out_as = XENOLITH_SHAPE_TRIANGLES;
                }
            }

            if (shape == XENOLITH_SHAPE_QUADS) {
                reads = in_hand[0].reads;
                streams = in_hand[0].streams;
                if (reads != NULL && vertices >= 4u) {
                    vertices = vertices / 4u * 6u;
                    laid_out_as = XENOLITH_SHAPE_TRIANGLES;
                }
            }

            /* Before the frame rather than inside it. Setting a picture up
             * is work of its own that has to finish before anything reads
             * it, and waiting for it in the middle of a frame stops the
             * frame that is already being written. */
            uint32_t pictures = in_hand[1].pictures;
            const uint32_t *looks = in_hand[1].looks;
            if (looks != NULL) {
                hand_over_the_pictures(base, looks, pictures);
            }
            /* And the ones the vertex shader reads. A title reads how bright
             * the last frame came out there, once for every corner rather than
             * for every pixel, and hands what it makes of that on to the pixel
             * shader; reading nothing there makes every frame black. */
            if (in_hand[0].looks != NULL) {
                hand_over_the_pictures(base, in_hand[0].looks, in_hand[0].pictures);
            }

            start_the_frame_in_its_place(base);

            /* What a title said to keep, which it changes between draws.
             * Some of what it draws belongs in a corner of the screen
             * rather than anywhere on it, and without this those went
             * wherever their vertices put them. */
            {
                int32_t left, top, right, bottom;
                where_a_draw_is_kept(base, &left, &top, &right, &bottom);
                kept_left = not_below_nought(left);
                kept_top = not_below_nought(top);
                kept_right = not_below_nought(right);
                kept_bottom = not_below_nought(bottom);
            }

            /* What a shader reads from outside, taken from where a title
             * put it. Handed over before each draw because a title writes
             * these between draws as readily as before them. */
            say_how_a_position_moves(base);
            say_where_the_pixel_goes(base);
            xenolith_clip_to(kept_left * samples_across, kept_top * samples_down,
                             kept_right * samples_across, kept_bottom * samples_down);
            xenolith_hand_over_constants(
                base + XENOLITH_REGISTERS + XENOLITH_CONSTANT_REGISTER * 4,
                base + XENOLITH_REGISTERS + XENOLITH_TRUTH_REGISTER * 4,
                base + XENOLITH_REGISTERS + XENOLITH_COUNT_REGISTER * 4);
            hand_over_the_vertices(base, vertices, reads, streams, shape, laid_out_as,
                                   in_hand[0].reads, in_hand[0].streams, in_hand[0].sum,
                                   listed_at, listed_count, listed_wide,
                                   shape == XENOLITH_SHAPE_TRIANGLE_RUN, furthest);

            if (only_this_shader_draws(in_hand[0].sum)) {
                return;
            }

            int drawn = xenolith_draw_now(drawn_blending, drawn_written, drawn_how_far,
                                          drawn_cut_off, drawn_cut_off_at, in_hand[0].sum,
                                          in_hand[0].said, in_hand[0].length, in_hand[1].sum,
                                          in_hand[1].said, in_hand[1].length, vertices,
                                          laid_out_as);

            say_what_was_drawn(shape, vertices, in_hand[0].sum, in_hand[1].sum, drawn);

            static int said_how_it_went;
            if (!said_how_it_went && drawing_a_frame) {
                said_how_it_went = 1;
                fprintf(stderr, "xenolith: %s\n",
                        drawn ? "a title's own shaders are what a device draws with"
                              : "a title's shaders would not build into something to draw with");
            }
        }


}
