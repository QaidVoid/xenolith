/* The interface lifted code is written against.
 *
 * This declares the machine state emitted code reads and writes, how it reaches
 * guest memory, and how control leaves one lifted function for another. It does
 * not implement any of it. Saying what emitted code needs is what lets the
 * emitted code be checked; providing what it needs to run is separate work.
 *
 * Guest memory is big endian and the host may not be. Every access here states
 * the byte order rather than leaving it to the caller, because a single missed
 * swap produces code that passes every test on one host and fails on another.
 */

#ifndef XENOLITH_H
#define XENOLITH_H

#include <stddef.h>
#include <stdint.h>

/* How many registers each bank holds. The vector bank is the console's, which
 * reaches four times as far as the architecture it extends. */
#define XENOLITH_GENERAL_REGISTERS 32
#define XENOLITH_FLOATING_REGISTERS 32
#define XENOLITH_VECTOR_REGISTERS 128
#define XENOLITH_CONDITION_FIELDS 8

/* A vector register, held as the guest's bytes in the guest's order.
 *
 * A register is 128 bits the instruction set reads as bytes, halfwords, words,
 * doublewords, or floats, and the guest reads every one of those big end first.
 * A host of the other byte order cannot hold a layout that makes all of those
 * views right at once: keeping the bytes means a word read directly comes back
 * reversed, and keeping each lane in host order means the bytes come back in
 * the wrong lane order.
 *
 * The bytes are kept, which is the same decision guest memory takes, and every
 * lane is assembled by the accessors below. There is deliberately no second
 * member to read a lane through, because reading one would be the mistake those
 * accessors exist to prevent, and it would pass every test on one host.
 */
typedef struct xenolith_vector {
    uint8_t u8[16];
} xenolith_vector;

/* A floating point register.
 *
 * Held as both a value and its bits, because the instruction set reads the same
 * register either way: the arithmetic works on the value and the conversions
 * work on the bits. The two members are the same width, so which one was last
 * written does not change where the bytes are.
 */
typedef union xenolith_float {
    double f64;
    uint64_t u64;
    int64_t s64;
} xenolith_float;

/* One condition field, held as its four bits rather than packed, so that
 * setting one does not require reading the others. */
typedef struct xenolith_condition {
    uint8_t lt;
    uint8_t gt;
    uint8_t eq;
    uint8_t so;
} xenolith_condition;

/* The machine state emitted code operates on.
 *
 * General purpose registers are held at their full width. Reading one narrower
 * is a cast at the point of use rather than a second field, because a union
 * would read a different half depending on the host's byte order.
 */
typedef struct xenolith_context {
    uint64_t r[XENOLITH_GENERAL_REGISTERS];
    xenolith_float f[XENOLITH_FLOATING_REGISTERS];
    xenolith_vector v[XENOLITH_VECTOR_REGISTERS];
    xenolith_condition cr[XENOLITH_CONDITION_FIELDS];
    uint64_t lr;
    uint64_t ctr;
    uint64_t xer;

    /* The two below are storage, and their architectural effects are NOT
     * modelled. Emitted code reads back what it wrote to either and nothing
     * else happens.
     *
     * msr: masking interrupts does nothing, because emitted code has none. The
     * only use of it in the titles this was built against is the save and
     * restore pair around a reservation, where the round trip is consistent and
     * the masking is what is being skipped.
     *
     * fpscr: a rounding mode written here does not change how a later operation
     * rounds, and an exception enable written here arms nothing. The emitted
     * arithmetic is the host's, in the host's default mode, which is the mode
     * this processor starts in.
     */
    uint64_t msr;
    uint64_t fpscr;
} xenolith_context;

/* A lifted function. Every one takes the machine state and the base of guest
 * memory, and returns nothing: where control goes next is decided inside. */
typedef void (*xenolith_function)(xenolith_context *ctx, uint8_t *base);

/* Guest memory is reached through these rather than by casting a pointer, so
 * that the byte order is stated once here instead of at every access. */

static inline uint8_t xenolith_load8(const uint8_t *base, uint32_t address) {
    return base[address];
}

static inline uint16_t xenolith_load16(const uint8_t *base, uint32_t address) {
    return (uint16_t)((uint16_t)base[address] << 8 | (uint16_t)base[address + 1]);
}

static inline uint32_t xenolith_load32(const uint8_t *base, uint32_t address) {
    return (uint32_t)base[address] << 24 | (uint32_t)base[address + 1] << 16 |
           (uint32_t)base[address + 2] << 8 | (uint32_t)base[address + 3];
}

static inline uint64_t xenolith_load64(const uint8_t *base, uint32_t address) {
    return (uint64_t)xenolith_load32(base, address) << 32 |
           (uint64_t)xenolith_load32(base, address + 4);
}

static inline void xenolith_store8(uint8_t *base, uint32_t address, uint8_t value) {
    base[address] = value;
}

static inline void xenolith_store16(uint8_t *base, uint32_t address, uint16_t value) {
    base[address] = (uint8_t)(value >> 8);
    base[address + 1] = (uint8_t)value;
}

static inline void xenolith_store32(uint8_t *base, uint32_t address, uint32_t value) {
    base[address] = (uint8_t)(value >> 24);
    base[address + 1] = (uint8_t)(value >> 16);
    base[address + 2] = (uint8_t)(value >> 8);
    base[address + 3] = (uint8_t)value;
}

static inline void xenolith_store64(uint8_t *base, uint32_t address, uint64_t value) {
    xenolith_store32(base, address, (uint32_t)(value >> 32));
    xenolith_store32(base, address + 4, (uint32_t)value);
}

/* Reaching one lane of a vector register.
 *
 * Lane zero is the one at the lowest guest address, which is how the
 * instruction set numbers elements. Each lane is assembled from the bytes the
 * same way a word is assembled from memory, so the byte order is stated here
 * once instead of being inherited from whichever host this is built on.
 */

static inline uint8_t xenolith_vector_u8(const xenolith_vector *v, unsigned lane) {
    return v->u8[lane];
}

static inline void xenolith_vector_set_u8(xenolith_vector *v, unsigned lane, uint8_t value) {
    v->u8[lane] = value;
}

static inline uint16_t xenolith_vector_u16(const xenolith_vector *v, unsigned lane) {
    return (uint16_t)(((uint16_t)v->u8[lane * 2] << 8) | v->u8[lane * 2 + 1]);
}

static inline void xenolith_vector_set_u16(xenolith_vector *v, unsigned lane, uint16_t value) {
    v->u8[lane * 2] = (uint8_t)(value >> 8);
    v->u8[lane * 2 + 1] = (uint8_t)value;
}

static inline uint32_t xenolith_vector_u32(const xenolith_vector *v, unsigned lane) {
    return ((uint32_t)v->u8[lane * 4] << 24) | ((uint32_t)v->u8[lane * 4 + 1] << 16) |
           ((uint32_t)v->u8[lane * 4 + 2] << 8) | v->u8[lane * 4 + 3];
}

static inline void xenolith_vector_set_u32(xenolith_vector *v, unsigned lane, uint32_t value) {
    v->u8[lane * 4] = (uint8_t)(value >> 24);
    v->u8[lane * 4 + 1] = (uint8_t)(value >> 16);
    v->u8[lane * 4 + 2] = (uint8_t)(value >> 8);
    v->u8[lane * 4 + 3] = (uint8_t)value;
}

static inline uint64_t xenolith_vector_u64(const xenolith_vector *v, unsigned lane) {
    return ((uint64_t)xenolith_vector_u32(v, lane * 2) << 32) | xenolith_vector_u32(v, lane * 2 + 1);
}

static inline void xenolith_vector_set_u64(xenolith_vector *v, unsigned lane, uint64_t value) {
    xenolith_vector_set_u32(v, lane * 2, (uint32_t)(value >> 32));
    xenolith_vector_set_u32(v, lane * 2 + 1, (uint32_t)value);
}

/* A float lane moves through its bits rather than through a pointer aimed at
 * them, because copying is what the language defines and the pointer is what a
 * compiler is allowed to assume nothing does. */
static inline float xenolith_vector_f32(const xenolith_vector *v, unsigned lane) {
    uint32_t bits = xenolith_vector_u32(v, lane);
    float value;
    __builtin_memcpy(&value, &bits, 4);
    return value;
}

static inline void xenolith_vector_set_f32(xenolith_vector *v, unsigned lane, float value) {
    uint32_t bits;
    __builtin_memcpy(&bits, &value, 4);
    xenolith_vector_set_u32(v, lane, bits);
}

/* Converting a single precision value to a fixed point one, with saturation.
 *
 * The instruction set clamps rather than wrapping, and a value that is not a
 * number becomes zero. Writing a cast instead would be undefined for exactly
 * the inputs this has to get right.
 */
static inline int32_t xenolith_saturate_signed(float value) {
    if (!(value == value)) {
        return 0;
    }
    if (value >= 2147483648.0f) {
        return 2147483647;
    }
    if (value <= -2147483648.0f) {
        return -2147483648;
    }
    return (int32_t)value;
}

static inline uint32_t xenolith_saturate_unsigned(float value) {
    if (!(value == value) || value <= 0.0f) {
        return 0;
    }
    if (value >= 4294967296.0f) {
        return 4294967295u;
    }
    return (uint32_t)value;
}

/* The high half of a doubleword product.
 *
 * C has no type wider than the operands to hold it in, so it is built from the
 * word products the way it would be done by hand. A compiler that has a
 * double-width type of its own recognizes this and emits the single
 * instruction; one that does not still gets the right answer.
 */
static inline uint64_t xenolith_multiply_high(uint64_t left, uint64_t right) {
    uint64_t low_left = left & 0xffffffffull;
    uint64_t high_left = left >> 32;
    uint64_t low_right = right & 0xffffffffull;
    uint64_t high_right = right >> 32;

    uint64_t low = low_left * low_right;
    uint64_t cross_a = high_left * low_right;
    uint64_t cross_b = low_left * high_right;
    uint64_t high = high_left * high_right;

    uint64_t carry = ((low >> 32) + (cross_a & 0xffffffffull) + (cross_b & 0xffffffffull)) >> 32;
    return high + (cross_a >> 32) + (cross_b >> 32) + carry;
}

static inline int64_t xenolith_multiply_high_signed(int64_t left, int64_t right) {
    uint64_t high = xenolith_multiply_high((uint64_t)left, (uint64_t)right);

    /* An unsigned product treats a negative operand as its two's complement,
     * which overstates the high half by the other operand once per negative. */
    if (left < 0) {
        high -= (uint64_t)right;
    }
    if (right < 0) {
        high -= (uint64_t)left;
    }
    return (int64_t)high;
}

/* Shifting a whole vector register, across lane boundaries.
 *
 * The amount is in bits and may be anything up to the width of the register, so
 * a byte of the result generally takes bits from two bytes of the source. The
 * shifts that move whole bytes are the same operation with a multiple of eight.
 */
static inline void xenolith_vector_shift_left(xenolith_vector *into,
                                              const xenolith_vector *from, unsigned bits) {
    unsigned bytes = bits / 8;
    unsigned rest = bits % 8;
    for (unsigned lane = 0; lane < 16; lane++) {
        unsigned at = lane + bytes;
        uint32_t high = at < 16 ? from->u8[at] : 0;
        uint32_t low = at + 1 < 16 ? from->u8[at + 1] : 0;
        into->u8[lane] = (uint8_t)(((high << 8 | low) >> (8 - rest)) & 0xff);
    }
}

static inline void xenolith_vector_shift_right(xenolith_vector *into,
                                               const xenolith_vector *from, unsigned bits) {
    unsigned bytes = bits / 8;
    unsigned rest = bits % 8;
    for (unsigned lane = 0; lane < 16; lane++) {
        uint32_t high = lane >= bytes + 1 ? from->u8[lane - bytes - 1] : 0;
        uint32_t low = lane >= bytes ? from->u8[lane - bytes] : 0;
        into->u8[lane] = (uint8_t)(((high << 8 | low) >> rest) & 0xff);
    }
}

/* Clamping a wider intermediate back into a lane.
 *
 * The saturating forms of the vector arithmetic stop at the end of the range
 * rather than wrapping, so each is computed at a width that cannot overflow and
 * then brought back. A clamp at the wrong bound gives an answer that is almost
 * right, which is the kind that survives review.
 */
static inline int64_t xenolith_clamp(int64_t value, int64_t low, int64_t high) {
    return value < low ? low : (value > high ? high : value);
}

static inline uint64_t xenolith_clamp_unsigned(int64_t value, uint64_t high) {
    if (value < 0) {
        return 0;
    }
    return (uint64_t)value > high ? high : (uint64_t)value;
}

/* Zeroing a data cache block.
 *
 * The block size is implementation defined. This console spells two of them
 * with one opcode and a field that selects between them: 32 bytes, and 128 for
 * the long form. The address is aligned down to whichever was asked for.
 *
 * A general purpose emulator does not share either size, so this is the one
 * instruction here with no execution oracle behind it.
 */
static inline void xenolith_zero_block(uint8_t *base, uint32_t address, uint32_t size) {
    uint32_t start = address & ~(size - 1);
    for (uint32_t byte = 0; byte < size; byte++) {
        base[start + byte] = 0;
    }
}

/* The condition register as one word, and back.
 *
 * The first field occupies the most significant four bits, and within a field
 * the order is less than, greater than, equal, summary overflow. Both
 * directions are written here rather than at each use, because the same bit
 * order mistake made in both would cancel out and survive.
 *
 * The mask names fields from the first, so its most significant bit selects
 * field zero.
 */
static inline uint32_t xenolith_condition_pack(const xenolith_condition *cr) {
    uint32_t packed = 0;
    for (unsigned field = 0; field < XENOLITH_CONDITION_FIELDS; field++) {
        uint32_t bits = (cr[field].lt ? 8u : 0u) | (cr[field].gt ? 4u : 0u) |
                        (cr[field].eq ? 2u : 0u) | (cr[field].so ? 1u : 0u);
        packed |= bits << (28 - 4 * field);
    }
    return packed;
}

static inline void xenolith_condition_unpack(xenolith_condition *cr, uint32_t packed,
                                             uint32_t mask) {
    for (unsigned field = 0; field < XENOLITH_CONDITION_FIELDS; field++) {
        if (!(mask & (1u << (7 - field)))) {
            continue;
        }
        uint32_t bits = (packed >> (28 - 4 * field)) & 0xfu;
        cr[field].lt = (uint8_t)((bits >> 3) & 1u);
        cr[field].gt = (uint8_t)((bits >> 2) & 1u);
        cr[field].eq = (uint8_t)((bits >> 1) & 1u);
        cr[field].so = (uint8_t)(bits & 1u);
    }
}

/* Where a trap that fired goes.
 *
 * A trap leaves the function it was in, so what happens next is not something
 * emitted code can express. The environment decides. Implemented there, not
 * here.
 */
void xenolith_trap(xenolith_context *ctx, uint8_t *base, uint32_t address);

/* Where a target unknown when the code was emitted becomes a function.
 *
 * This is the single place an address is resolved at run time, so that a branch
 * whose table could not be read has one path rather than many. Implemented by
 * the environment, not here.
 */
void xenolith_dispatch(xenolith_context *ctx, uint8_t *base, uint32_t address,
                       uint32_t from);

/* Where a call to a function that was never translated goes.
 *
 * Told apart from a trap because the two are different events. A trap is an
 * instruction the title ran on purpose, and this is a function the translation
 * could not express, so reading one as the other sends a reader looking in the
 * wrong place. Implemented by the environment, not here.
 */
void xenolith_unlifted(xenolith_context *ctx, uint8_t *base, uint32_t address);

/* Where a call to an imported function goes.
 *
 * The container names an import by ordinal within a library and never by name,
 * so the ordinal is what the title actually said. The name beside it comes from
 * a catalogue of what the console's libraries export and is null where the
 * catalogue has no entry, so a reader gets something to act on without the
 * ordinal ever being replaced by a guess at it.
 *
 * What the ordinal does is the environment's to decide. Implemented there, not
 * here.
 */
void xenolith_import(xenolith_context *ctx, uint8_t *base, const char *library,
                     uint32_t ordinal, const char *name);

/* Where a reservation is taken and where it is redeemed.
 *
 * A program with one thread and no sharing could load and store these directly
 * and be right, which is exactly why they are not emitted that way: the
 * assumption would sit where nothing could find it. An environment that has
 * threads implements these with real atomics instead.
 *
 * The conditional store reports whether the store happened, which is what the
 * retry branch after one reads.
 */
uint32_t xenolith_reserve32(const uint8_t *base, uint32_t address);
uint64_t xenolith_reserve64(const uint8_t *base, uint32_t address);
uint8_t xenolith_conditional32(uint8_t *base, uint32_t address, uint32_t value);
uint8_t xenolith_conditional64(uint8_t *base, uint32_t address, uint64_t value);

/* The time base, which advances.
 *
 * At what rate is the environment's decision. Emitting a constant would produce
 * a program whose timing loops never finish.
 */
uint64_t xenolith_timebase(void);

/* Where an address becomes the function lifted for it.
 *
 * Only the lift knows which addresses it emitted, so the table is written
 * beside the code rather than here. Anything not in it is not a function this
 * program has.
 */
xenolith_function xenolith_lookup(uint32_t address);

/* Mapping guest memory and loading the image into it.
 *
 * Emitted code reaches memory by adding a guest address to a base, so the base
 * has to be a mapping wide enough that any address it could form indexes
 * inside it. Returns the base, or a null pointer if the image could not be
 * read.
 */
uint8_t *xenolith_boot(const char *image, uint32_t load_address);

/* Where the guest's stack pointer starts.
 *
 * A title expects to have been given one. Entered with the register at zero it
 * still runs, because its first frame subtracts from zero and wraps to the top
 * of a space this maps all of, but every frame it takes then sits somewhere
 * nothing chose. Naming a stack costs nothing and makes where it lives a
 * decision rather than an accident.
 *
 * Clear of where the image loads, and far enough below the top that the
 * linkage area a caller writes above the pointer is inside the space.
 */
#define XENOLITH_STACK_TOP 0x70000000u

#endif /* XENOLITH_H */
