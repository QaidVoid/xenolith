/* Reading the commands a title writes for the graphics hardware.
 *
 * A title does not call the hardware. It writes packets into a ring buffer and
 * tells the hardware where it has got to, and the hardware reads them and
 * writes back how far it has read. Everything a title draws is in that buffer
 * and nothing else, so a runtime that does not read it draws nothing and has no
 * way of knowing what was asked for.
 *
 * This reads them. What it can do with them depends on what else is here: with
 * no renderer, a packet that would draw is accounted for and passed over, and
 * the run says so rather than pretending a frame was drawn. A packet whose
 * effect a title can see, which is a write to an address it named, is carried
 * out, because a title waits on that and never carries on without it.
 *
 * This file is included by xenolith.c rather than compiled on its own, because
 * everything it needs is a register write or a report, and those live there.
 */

/* What the top two bits of a header say a packet is.
 *
 * Read out of what a title actually wrote rather than taken from a description
 * of the hardware. The two bits say which of four kinds a packet is and the
 * next fourteen say how many words follow, which is checkable against a real
 * buffer: the lengths those fields give land exactly on the next header, four
 * packets running, with nothing left over. */
#define XENOLITH_PACKET_KIND_REGISTERS 0
#define XENOLITH_PACKET_KIND_TWO_REGISTERS 1
#define XENOLITH_PACKET_KIND_NOTHING 2
#define XENOLITH_PACKET_KIND_COMMAND 3

/* The opcodes a command packet carries that this reads. */
#define XENOLITH_PACKET_BUFFER 0x3fu
#define XENOLITH_PACKET_SHADER 0x2bu
#define XENOLITH_PACKET_CHANGE_PART 0x21u
#define XENOLITH_PACKET_TELL_THE_TITLE 0x54u
#define XENOLITH_PACKET_LOAD 0x2fu
#define XENOLITH_PACKET_REPORT 0x58u
#define XENOLITH_PACKET_DRAW 0x36u
#define XENOLITH_PACKET_DRAW_INDEXED 0x22u

/* How deep a run of commands kept somewhere else may nest.
 *
 * A buffer packet names another run, and that run may name another. Nothing
 * says how deep a title goes, and a buffer that named itself would be read for
 * ever, so the depth is bounded and a run past it is reported. */
#define XENOLITH_BUFFER_DEPTH 8u

/* The bit above the register index that says whether the words after a packet
 * all go to that one register or go up by one each. */
#define XENOLITH_ALL_TO_ONE 0x8000u
#define XENOLITH_REGISTER_INDEX 0x7fffu

/* What has been read since the run began, so that a run can say what it found
 * rather than only what it did not. */
struct read_so_far {
    unsigned long packets;
    unsigned long registers;
    unsigned long drew;
    unsigned long passed_over;
    unsigned long not_read;
    unsigned long buffers;
    unsigned long shaders;
};

static struct read_so_far read_so_far;

/* Says a packet was one this has no reading of, once per opcode.
 *
 * Once per opcode rather than once per packet: a title writes thousands of them
 * a frame and the interesting fact is which kinds are not read, not how many
 * times each appeared. */
static void say_an_opcode_was_not_read(uint32_t opcode, uint32_t at) {
    static uint32_t said[32];
    static unsigned count;

    for (unsigned one = 0; one < count; one++) {
        if (said[one] == opcode) {
            return;
        }
    }
    if (count < sizeof said / sizeof *said) {
        said[count++] = opcode;
    }
    say_a_report("packet %#04x at %#010x is one this does not read", opcode, at);
}

/* Says what was read, once, at the end of a run. */
static void say_what_was_read(void) {
    say_a_report("read %lu packets: %lu set registers, %lu drew, %lu passed over, "
                 "%lu not read, %lu buffers, %lu shaders",
                 read_so_far.packets, read_so_far.registers, read_so_far.drew,
                 read_so_far.passed_over, read_so_far.not_read, read_so_far.buffers,
                 read_so_far.shaders);
}

/* Reads one run of commands, which may name others.
 *
 * Returns how many words it read, which is every word it was given: a packet
 * that runs past the end is the rest of it not having been written yet, and the
 * walk stops there rather than reading whatever follows. */
static uint32_t walk_the_commands(uint8_t *base, uint32_t stream, uint32_t words,
                                  unsigned depth) {
    uint32_t at = 0;

    if (depth > XENOLITH_BUFFER_DEPTH) {
        say_a_report("commands kept somewhere else nest more than %u deep at %#010x",
                     XENOLITH_BUFFER_DEPTH, stream);
        return 0;
    }

    while (at < words) {
        uint32_t header = xenolith_load32(base, stream + at * 4);
        uint32_t kind = header >> 30;
        uint32_t follows;

        switch (kind) {
        case XENOLITH_PACKET_KIND_NOTHING:
            /* A filler word, carrying nothing. */
            follows = 0;
            break;
        case XENOLITH_PACKET_KIND_TWO_REGISTERS:
            follows = 2;
            break;
        default:
            /* The field is how many words the packet takes altogether, its own
             * header counted. Adding the header again here counts it twice,
             * which makes every packet one word too long: the walk then starts
             * the next one a word early and reads its header out of the middle
             * of the one before. */
            follows = (header >> 16) & 0x3fff;
            break;
        }

        if (at + 1 + follows > words) {
            /* The rest of this packet has not been written yet. */
            break;
        }

        read_so_far.packets++;

        switch (kind) {
        case XENOLITH_PACKET_KIND_REGISTERS: {
            /* A run of registers set from one packet.
             *
             * Which register it starts at is fifteen bits, and the bit above
             * them says whether the words after it go up by one each or all go
             * to that one register. Reading that bit as part of the index puts
             * every write of the second kind eight thousand registers past
             * where it belongs, which is off the end of everything that means
             * anything. */
            uint32_t first = header & XENOLITH_REGISTER_INDEX;
            int all_to_one = (header & XENOLITH_ALL_TO_ONE) != 0;
            for (uint32_t k = 0; k < follows; k++) {
                set_register(base, all_to_one ? first : first + k,
                             xenolith_load32(base, stream + (at + 1 + k) * 4));
                read_so_far.registers++;
            }
            break;
        }
        case XENOLITH_PACKET_KIND_TWO_REGISTERS:
            /* Two registers, each named in the header rather than counted. */
            set_register(base, header & 0x7ff,
                         xenolith_load32(base, stream + (at + 1) * 4));
            set_register(base, (header >> 11) & 0x7ff,
                         xenolith_load32(base, stream + (at + 2) * 4));
            read_so_far.registers += 2;
            break;

        case XENOLITH_PACKET_KIND_COMMAND: {
            uint32_t opcode = (header >> 8) & 0xff;

            /* Commands kept somewhere else are followed, because that is where
             * a title puts nearly all of them. */
            if (opcode == XENOLITH_PACKET_BUFFER && follows >= 2) {
                uint32_t where = xenolith_load32(base, stream + (at + 1) * 4);
                uint32_t count = xenolith_load32(base, stream + (at + 2) * 4) & 0xfffffu;
                read_so_far.buffers++;
                if (where != 0 && count != 0) {
                    (void)walk_the_commands(base, where, count, depth + 1);
                }
                break;
            }

            /* A shader, handed over as the kind it is, how many words follow,
             * and the words. Nothing here compiles one, so what is counted is
             * that one was handed over. */
            if (opcode == XENOLITH_PACKET_SHADER && follows >= 2) {
                uint32_t which_kind = xenolith_load32(base, stream + (at + 1) * 4);
                uint32_t how_many = xenolith_load32(base, stream + (at + 2) * 4) & 0xffffu;
                read_so_far.shaders++;
                if (how_many == 0 || how_many + 2 > follows) {
                    say_a_report("a shader packet at %#010x says %u words and carries %u",
                                 stream + at * 4, how_many, follows - 2);
                    break;
                }
                take_the_shader_in_hand(base, which_kind, stream + (at + 3) * 4, how_many);
                break;
            }

            /* Telling the title the hardware has reached this point.
             *
             * A title queues work to be run when its commands are reached and
             * writes this where it wants to be told. Nothing it queues
             * afterwards happens until that shows up. */
            if (opcode == XENOLITH_PACKET_TELL_THE_TITLE) {
                raise_the_graphics_interrupt(base, XENOLITH_INTERRUPT_REACHED);
                break;
            }

            /* Changing part of a register rather than all of it: what it holds
             * is kept where a mask says, and bits are put in where another
             * says. */
            if (opcode == XENOLITH_PACKET_CHANGE_PART && follows >= 3) {
                uint32_t how = xenolith_load32(base, stream + (at + 1) * 4);
                uint32_t keeping = xenolith_load32(base, stream + (at + 2) * 4);
                uint32_t putting = xenolith_load32(base, stream + (at + 3) * 4);
                uint32_t which = how & 0x7ffu;
                uint32_t value = register_says(base, which);
                value &= keeping;
                value |= putting;
                set_register(base, which, value);
                read_so_far.registers++;
                break;
            }

            /* Filling in registers from memory, which is where a title keeps
             * what its shaders read. */
            if (opcode == XENOLITH_PACKET_LOAD && follows >= 3) {
                uint32_t where = xenolith_load32(base, stream + (at + 1) * 4) & 0x3fffffffu;
                uint32_t what = xenolith_load32(base, stream + (at + 2) * 4);
                uint32_t word_count = xenolith_load32(base, stream + (at + 3) * 4) & 0xfffu;
                for (uint32_t word = 0; word < word_count; word++) {
                    set_register(base, what + word, xenolith_load32(base, where + word * 4));
                    read_so_far.registers++;
                }
                break;
            }

            /* Saying that what was handed over is finished.
             *
             * Everything written here is finished the moment it is read, so the
             * value goes where it was asked for as soon as this is reached.
             * Nothing waits, because there is nothing to wait for.
             *
             * A byte order this does not know is reported rather than written
             * in, because writing a word the wrong way round gives a title a
             * number it compares against the one it asked for and never
             * matches, which is the same as not writing it at all except that
             * it looks like it worked. */
            if (opcode == XENOLITH_PACKET_REPORT && follows >= 3) {
                uint32_t where = xenolith_load32(base, stream + (at + 2) * 4);
                uint32_t what = xenolith_load32(base, stream + (at + 3) * 4);
                uint32_t order = where & 3u;

                if (order == 2u) {
                    xenolith_store32(base, where & ~3u, what);
                } else {
                    static int said_about_order;
                    if (!said_about_order) {
                        said_about_order = 1;
                        say_a_report("a report at %#010x asks for byte order %u, and "
                                     "only 2 is written",
                                     where & ~3u, order);
                    }
                }
                break;
            }

            if (opcode == XENOLITH_PACKET_DRAW || opcode == XENOLITH_PACKET_DRAW_INDEXED) {
                read_so_far.drew++;
                read_so_far.passed_over++;
                draw_what_was_asked_for(base, stream, at, follows, opcode, header);
                break;
            }

            /* Anything else is read for its length and passed over. That is not
             * the same as executing it, and a frame reported finished here had
             * nothing drawn in it. */
            say_an_opcode_was_not_read(opcode, stream + at * 4);
            read_so_far.not_read++;
            break;
        }
        }

        at += 1 + follows;
    }

    return at;
}
