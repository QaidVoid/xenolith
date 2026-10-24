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
#define XENOLITH_PACKET_SHADER_FROM_MEMORY 0x27u
#define XENOLITH_PACKET_WAIT 0x3cu
#define XENOLITH_PACKET_WRITE_IF 0x45u

/* The register saying whether the processor's view of memory has caught up
 * with the hardware's, whose top bit a title sets to ask and waits to see
 * clear. */
#define XENOLITH_COHERENT_STATUS 0x0a31u
#define XENOLITH_STILL_CATCHING_UP 0x80000000u

/* What a wait or a conditional write compares: a register, or a word of
 * memory in the byte order the low two bits of its address say. */
static uint32_t the_value_looked_at(uint8_t *base, uint32_t how, uint32_t where) {
    if (how & 0x10u) {
        return as_the_processor_reads(xenolith_load32(base, where & ~3u), where);
    }
    if (where == XENOLITH_COHERENT_STATUS) {
        /* Memory here is one view, so it has always caught up the moment it is
         * asked. */
        set_register(base, where, register_says(base, where) & ~XENOLITH_STILL_CATCHING_UP);
    }
    return register_says(base, where);
}

/* Whether a value, masked, stands as the low three bits of `how` ask against
 * a reference: never, below, at most, equal, not equal, at least, above, or
 * always. */
static int it_holds(uint32_t how, uint32_t value, uint32_t against, uint32_t mask) {
    uint32_t masked = value & mask;
    switch (how & 7u) {
    case 0:
        return 0;
    case 1:
        return masked < against;
    case 2:
        return masked <= against;
    case 3:
        return masked == against;
    case 4:
        return masked != against;
    case 5:
        return masked >= against;
    case 6:
        return masked > against;
    default:
        return 1;
    }
}
#define XENOLITH_PACKET_CHANGE_PART 0x21u
#define XENOLITH_PACKET_TELL_THE_TITLE 0x54u
#define XENOLITH_PACKET_LOAD 0x2fu
#define XENOLITH_PACKET_SET_CONSTANT 0x2du
#define XENOLITH_PACKET_BIN_MASK 0x50u
#define XENOLITH_PACKET_BIN_SELECT 0x51u
#define XENOLITH_PACKET_BIN_MASK_LOW 0x60u
#define XENOLITH_PACKET_BIN_MASK_HIGH 0x61u
#define XENOLITH_PACKET_BIN_SELECT_LOW 0x62u
#define XENOLITH_PACKET_BIN_SELECT_HIGH 0x63u

/* Which parts of the screen the commands are for, and which part is being
 * drawn now.
 *
 * A title drawing the screen a part at a time hands every part the same
 * commands, and marks the ones that belong to only some of the parts. A marked
 * command is carried out only where the two have a bit in common. Among the
 * marked ones is asking for something to be run on the processor when the
 * commands get there, and carried out for every part rather than the one it
 * belongs to, it runs once a part, and what it runs hands the hardware the
 * parts over again. Both start with every bit set, so a title that never draws
 * a part at a time has every command carried out. */
static uint64_t parts_marked = ~0ull;
static uint64_t part_drawn = ~0ull;
#define XENOLITH_PACKET_REPORT 0x58u
#define XENOLITH_PACKET_WHERE_IT_REACHED 0x5au
#define XENOLITH_PACKET_DRAW 0x36u
#define XENOLITH_PACKET_DRAW_INDEXED 0x22u
#define XENOLITH_PACKET_COMPARE 0x45u

/* Which block of registers a load fills in, and where each begins. The number
 * a load names is counted from the front of the block rather than from the front
 * of the register file. */
#define XENOLITH_LOAD_INTO_CONSTANTS 0u
#define XENOLITH_LOAD_INTO_FETCHES 1u
#define XENOLITH_LOAD_INTO_TRUTHS 2u
#define XENOLITH_LOAD_INTO_COUNTS 3u
#define XENOLITH_LOAD_INTO_REGISTERS 4u
#define XENOLITH_FIRST_ORDINARY_REGISTER 0x2000u

/* Where in the register file the block a packet names begins, and whether it
 * names one this reads.
 *
 * The number a packet gives is counted from the front of the block rather than
 * from the front of the register file, so one that names constant nought fills
 * the first constant a shader reads rather than the first register anything
 * reads. Written without the block's own start, every constant a title sends
 * lands eight thousand registers short of where the shader looks for it, and
 * the shader computes from nought. A block this has no reading of is refused:
 * filling the register file from it would put a title's numbers where
 * something else reads them. */
static int where_a_block_begins(uint32_t named, uint32_t *start) {
    switch ((named >> 16) & 0xffu) {
    case XENOLITH_LOAD_INTO_CONSTANTS:
        *start = XENOLITH_CONSTANT_REGISTER;
        return 1;
    case XENOLITH_LOAD_INTO_FETCHES:
        *start = XENOLITH_FETCH_REGISTER;
        return 1;
    case XENOLITH_LOAD_INTO_TRUTHS:
        *start = XENOLITH_TRUTH_REGISTER;
        return 1;
    case XENOLITH_LOAD_INTO_COUNTS:
        *start = XENOLITH_COUNT_REGISTER;
        return 1;
    case XENOLITH_LOAD_INTO_REGISTERS:
        *start = XENOLITH_FIRST_ORDINARY_REGISTER;
        return 1;
    default:
        return 0;
    }
}

/* A comparison that is always true, which is how a title writes a register
 * through the compare packet rather than through a register packet. */
#define XENOLITH_COMPARE_ALWAYS 7u

/* Set where the compare packet writes to memory rather than to a register. */
#define XENOLITH_WRITES_MEMORY 0x20u

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
/* Writes the words of a packet this does not read, where a run asks for them.
 *
 * Reading a packet kind by hand is the only way to tell whether a field was
 * taken from the right bits, and the words are not kept anywhere a run can be
 * asked for them afterwards. */
static void say_what_a_packet_held(uint8_t *base, uint32_t at) {
    static int telling = -1;
    if (telling < 0) {
        telling = getenv("XENOLITH_KEEP_PACKETS") != NULL;
    }
    if (!telling) {
        return;
    }
    fprintf(stderr, "XPACKET %#010x:", at);
    for (uint32_t k = 0; k < 4u; k++) {
        fprintf(stderr, " %08x", xenolith_load32(base, at + k * 4u));
    }
    fprintf(stderr, "\n");
}

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

/* Says what was read so far.
 *
 * The packets counted here are words walked, not distinct commands. A title
 * that rewinds its write pointer has everything from there on read again, and
 * one that spins on a short run of commands winds this count up without asking
 * for anything new. So the count says how much walking was done, which is why
 * it is worth having at the end of a run: a title reading quietly and a title
 * spinning are the same thing to everything else here. */
static void say_what_was_read(void) {
    say_a_report("read %lu packets: %lu set registers, %lu drew, %lu passed over, "
                 "%lu not read, %lu buffers, %lu shaders",
                 read_so_far.packets, read_so_far.registers, read_so_far.drew,
                 read_so_far.passed_over, read_so_far.not_read, read_so_far.buffers,
                 read_so_far.shaders);
}

/* Says what was read over the whole run, at the point the run ends.
 *
 * The same counts the snapshot above reports, said once more so that a run which
 * ended between two snapshots still leaves a record of what it walked. */
static void say_what_was_read_in_total(void) {
    say_a_report("in total, read %lu packets: %lu set registers, %lu drew, %lu "
                 "passed over, %lu not read, %lu buffers, %lu shaders",
                 read_so_far.packets, read_so_far.registers, read_so_far.drew,
                 read_so_far.passed_over, read_so_far.not_read, read_so_far.buffers,
                 read_so_far.shaders);
}

/* How many words a packet takes, its header included. */
static uint32_t packet_words(uint32_t header) {
    switch (header >> 30) {
    case XENOLITH_PACKET_KIND_NOTHING:
        /* A filler word, carrying nothing. */
        return 1;
    case XENOLITH_PACKET_KIND_TWO_REGISTERS:
        return 3;
    default:
        /* The field is one less than how many words follow the header. A
         * buffer packet a title wrote reads `c0013f00 163d0040 0000000b`: the
         * field is one and there are two words after the header, so the field
         * plus one is the count. Read as the count itself, every packet is one
         * word short and the walk starts the next one inside the one before. */
        return ((header >> 16) & 0x3fff) + 2;
    }
}

/* Reads one run of commands, which may name others.
 * Returns how many words it read, which is every word it was given: a packet
 * that runs past the end is the rest of it not having been written yet, and the
 * walk stops there rather than reading whatever follows. */
/* Where in the ring the commands being walked began, or nothing where they are
 * not the ring's own.
 *
 * The hardware says how far through the ring it has read as it reads, and a
 * title waits on that before it writes over what was read. Said only once a
 * whole run of commands had been walked, which can be the whole of a frame, a
 * title spent most of its time waiting to be told what had long been read. */
#define XENOLITH_NOT_THE_RING UINT32_MAX
static uint32_t ring_read_from = XENOLITH_NOT_THE_RING;

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
        uint32_t follows = packet_words(header) - 1;

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

            if ((header & 1u) != 0 && (parts_marked & part_drawn) == 0) {
                read_so_far.passed_over++;
                break;
            }

            if (opcode >= XENOLITH_PACKET_BIN_MASK_LOW && opcode <= XENOLITH_PACKET_BIN_SELECT_HIGH
                && follows >= 1) {
                uint64_t value = xenolith_load32(base, stream + (at + 1) * 4);
                int high = opcode == XENOLITH_PACKET_BIN_MASK_HIGH
                           || opcode == XENOLITH_PACKET_BIN_SELECT_HIGH;
                uint64_t *which = opcode <= XENOLITH_PACKET_BIN_MASK_HIGH ? &parts_marked
                                                                          : &part_drawn;
                *which = high ? (*which & 0xffffffffull) | (value << 32)
                              : (*which & ~0xffffffffull) | value;
                break;
            }
            if ((opcode == XENOLITH_PACKET_BIN_MASK || opcode == XENOLITH_PACKET_BIN_SELECT)
                && follows >= 2) {
                uint64_t value = ((uint64_t)xenolith_load32(base, stream + (at + 1) * 4) << 32)
                                 | xenolith_load32(base, stream + (at + 2) * 4);
                if (opcode == XENOLITH_PACKET_BIN_MASK) {
                    parts_marked = value;
                } else {
                    part_drawn = value;
                }
                break;
            }

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

            /* A shader kept in memory rather than carried in the packet: the
             * kind it is in the low two bits of the address, and how many words
             * it runs to in the low half of the word after.
             *
             * Passed over, the shaders a title hands over this way are never
             * the ones drawn with: every draw after it uses whatever was last
             * handed over in a packet, and one title hands over nearly all of
             * its shaders this way, so its whole frame was drawn with the one
             * pair it carried in packets. */
            if (opcode == XENOLITH_PACKET_SHADER_FROM_MEMORY && follows >= 2) {
                uint32_t named = xenolith_load32(base, stream + (at + 1) * 4);
                uint32_t how_many = xenolith_load32(base, stream + (at + 2) * 4) & 0xffffu;
                read_so_far.shaders++;
                if (how_many != 0 && (named & ~3u) != 0) {
                    take_the_shader_in_hand(base, named & 3u, named & ~3u, how_many);
                }
                break;
            }

            /* Waiting for a register or a word of memory to stand as asked.
             *
             * The hardware stops reading here until it does, and a title
             * relies on that: it hands over a run of commands whose end it has
             * not written yet, with a wait in front of the end for a word it
             * changes once it has, and goes on writing. Read straight through,
             * what lies past the wait is whatever that memory held before, and
             * every command after it is read out of step. So this waits too,
             * as long as it takes, and says so once a wait has gone on long
             * enough to look like one that will not end. */
            if (opcode == XENOLITH_PACKET_WAIT && follows >= 5) {
                uint32_t how = xenolith_load32(base, stream + (at + 1) * 4);
                uint32_t where = xenolith_load32(base, stream + (at + 2) * 4);
                uint32_t against = xenolith_load32(base, stream + (at + 3) * 4);
                uint32_t mask = xenolith_load32(base, stream + (at + 4) * 4);
                unsigned long waited = 0;
                unsigned long looked = 0;
                while (!it_holds(how, the_value_looked_at(base, how, where), against, mask)) {
                    if (a_moment_passes(&looked) && ++waited == 20000u) {
                        say_a_report("the commands at %#010x have waited two seconds for %s "
                                     "%#x to stand against %#x, and wait on",
                                     stream + at * 4, (how & 0x10u) ? "memory at" : "register",
                                     where & ~3u, against);
                    }
                }
                break;
            }

            /* Writing a register or a word of memory if another stands as
             * asked, which a title uses to write a table one entry at a time
             * only while the hardware is where it expects. */
            if (opcode == XENOLITH_PACKET_WRITE_IF && follows >= 6) {
                uint32_t how = xenolith_load32(base, stream + (at + 1) * 4);
                uint32_t where = xenolith_load32(base, stream + (at + 2) * 4);
                uint32_t against = xenolith_load32(base, stream + (at + 3) * 4);
                uint32_t mask = xenolith_load32(base, stream + (at + 4) * 4);
                uint32_t written_where = xenolith_load32(base, stream + (at + 5) * 4);
                uint32_t written = xenolith_load32(base, stream + (at + 6) * 4);
                if (it_holds(how, the_value_looked_at(base, how, where), against, mask)) {
                    if (how & 0x100u) {
                        xenolith_store32(base, written_where & ~3u,
                                         as_the_processor_reads(written, written_where));
                    } else {
                        set_register(base, written_where, written);
                        read_so_far.registers++;
                    }
                }
                break;
            }

            /* Telling the title the hardware has reached this point.
             *
             * A title queues work to be run when its commands are reached and
             * writes this where it wants to be told. Nothing it queues
             * afterwards happens until that shows up. The word after the
             * header names the processors to tell, a bit apiece, and each is
             * told in turn. */
            if (opcode == XENOLITH_PACKET_TELL_THE_TITLE) {
                uint32_t processors = follows >= 1 ? xenolith_load32(base, stream + (at + 1) * 4)
                                                   : 1u;
                for (uint32_t processor = 0; processor < XENOLITH_PROCESSORS; processor++) {
                    if ((processors >> processor) & 1u) {
                        raise_the_graphics_interrupt(base, XENOLITH_INTERRUPT_REACHED, processor);
                    }
                }
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
                uint32_t into = 0;
                if (!where_a_block_begins(what, &into)) {
                    word_count = 0;
                }
                into += what & 0x7ffu;
                for (uint32_t word = 0; word < word_count; word++) {
                    set_register(base, into + word, xenolith_load32(base, where + word * 4));
                    read_so_far.registers++;
                }
                break;
            }

            /* Filling in registers from the words that follow, in the block and
             * from the place the first of them names. */
            if (opcode == XENOLITH_PACKET_SET_CONSTANT && follows >= 1) {
                uint32_t what = xenolith_load32(base, stream + (at + 1) * 4);
                uint32_t into = 0;
                if (where_a_block_begins(what, &into)) {
                    into += what & 0x7ffu;
                    for (uint32_t word = 1; word < follows; word++) {
                        set_register(base, into + word - 1,
                                     xenolith_load32(base, stream + (at + 1 + word) * 4));
                        read_so_far.registers++;
                    }
                }
                break;
            }

            /* Setting a register after looking at one.
             *
             * A title writes a register through this rather than through a
             * register packet, saying the comparison always holds. Anything
             * else here is a write that waits on something, and a register left
             * holding what it held before is a shader reading numbers nobody
             * sent. */
            if (opcode == XENOLITH_PACKET_COMPARE && follows >= 6) {
                uint32_t how = xenolith_load32(base, stream + (at + 1) * 4);
                if ((how & 7u) == XENOLITH_COMPARE_ALWAYS
                    && (how & XENOLITH_WRITES_MEMORY) == 0) {
                    set_register(base, xenolith_load32(base, stream + (at + 5) * 4),
                                 xenolith_load32(base, stream + (at + 6) * 4));
                    read_so_far.registers++;
                    break;
                }
            }

            /* Showing a frame, in the packet the kernel wrote when the title
             * asked: the picture to show is the word after its tag. */
            if (opcode == XENOLITH_PACKET_SWAP && follows >= 2) {
                show_a_frame(xenolith_load32(base, stream + (at + 2) * 4));
                break;
            }

            /* Saying that what was handed over is finished.
             *
             * Everything written here is finished the moment it is read, so the
             * value goes where it was asked for as soon as this is reached.
             * Nothing waits, because there is nothing to wait for.
             *
             * The word goes in the byte order the low two bits of the address
             * say. A title that asks for no swap reads the word back the other
             * way round itself, and a word not written at all is one it waits on
             * for ever. */
            if (opcode == XENOLITH_PACKET_REPORT && follows >= 3) {
                uint32_t where = xenolith_load32(base, stream + (at + 2) * 4);
                uint32_t what = xenolith_load32(base, stream + (at + 3) * 4);
                uint32_t read_back = as_the_processor_reads(what, where);
                xenolith_store32(base, where & ~3u, read_back);
                break;
            }

            /* Where on the screen what was drawn reached: the least and most
             * across and down, in squares of eight, and the nearest and
             * furthest. Six halves, written two to a word in the byte order
             * the low two bits of the address name, as a report is.
             *
             * A title drawing a part of the screen at a time works out from
             * these which parts each of its draws touches, and marks the draw
             * for only those. Never written, what it read was whatever the
             * memory held, and in one title most of the scene was marked for
             * the left half only and the right half came out nearly empty.
             * The whole screen is said for every draw, so that every draw is
             * marked for every part: drawing one where it lands outside a part
             * leaves nothing behind there, and leaving one out where it lands
             * inside leaves a hole. */
            if (opcode == XENOLITH_PACKET_WHERE_IT_REACHED && follows >= 2) {
                static const uint32_t reached[3] = {(2560u / 8u) << 16, (2560u / 8u) << 16,
                                                    1u << 16};
                uint32_t where = xenolith_load32(base, stream + (at + 2) * 4);
                for (uint32_t one = 0; one < 3u; one++) {
                    xenolith_store32(base, (where & ~3u) + one * 4u,
                                     as_the_processor_reads(reached[one], where));
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
            say_what_a_packet_held(base, stream + at * 4);
            say_an_opcode_was_not_read(opcode, stream + at * 4);
            read_so_far.not_read++;
            break;
        }
        }

        at += 1 + follows;
        if (depth == 0 && ring_read_from != XENOLITH_NOT_THE_RING
            && graphics.read_back_at != 0) {
            xenolith_store32(base, graphics.read_back_at,
                             (ring_read_from + at) % graphics.ring_words);
        }
    }

    return at;
}
