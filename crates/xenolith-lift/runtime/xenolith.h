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

#include <math.h>
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
 * that the byte order is stated once here instead of at every access.
 *
 * Each takes the pointer before adding to it. Adding to the address first would
 * be adding in the guest's own thirty two bits, so a word begun in the last few
 * bytes would carry into the first few instead of running past the end, and a
 * title reading through a pointer that is already nonsense would read the front
 * of its own memory rather than nothing. */

static inline uint8_t xenolith_load8(const uint8_t *base, uint32_t address) {
    return (base + address)[0];
}

static inline uint16_t xenolith_load16(const uint8_t *base, uint32_t address) {
    const uint8_t *at = base + address;
    return (uint16_t)((uint16_t)at[0] << 8 | (uint16_t)at[1]);
}

static inline uint32_t xenolith_load32(const uint8_t *base, uint32_t address) {
    const uint8_t *at = base + address;
    return (uint32_t)at[0] << 24 | (uint32_t)at[1] << 16 | (uint32_t)at[2] << 8 |
           (uint32_t)at[3];
}

static inline uint64_t xenolith_load64(const uint8_t *base, uint32_t address) {
    return (uint64_t)xenolith_load32(base, address) << 32 |
           (uint64_t)xenolith_load32(base, address + 4);
}

static inline void xenolith_store8(uint8_t *base, uint32_t address, uint8_t value) {
    (base + address)[0] = value;
}

static inline void xenolith_store16(uint8_t *base, uint32_t address, uint16_t value) {
    uint8_t *at = base + address;
    at[0] = (uint8_t)(value >> 8);
    at[1] = (uint8_t)value;
}

static inline void xenolith_store32(uint8_t *base, uint32_t address, uint32_t value) {
    uint8_t *at = base + address;
    at[0] = (uint8_t)(value >> 24);
    at[1] = (uint8_t)(value >> 16);
    at[2] = (uint8_t)(value >> 8);
    at[3] = (uint8_t)value;
}

static inline void xenolith_store64(uint8_t *base, uint32_t address, uint64_t value) {
    xenolith_store32(base, address, (uint32_t)(value >> 32));
    xenolith_store32(base, address + 4, (uint32_t)value);
}

/* Converting a single precision value to a whole number, with saturation.
 *
 * The instruction set clamps rather than wrapping, and a C cast does neither:
 * a value that is not a number gives the console's one answer for that too, and
 * a C cast leaves it to the host. Both are stated here so that every host gives
 * the console's answer rather than its own.
 *
 * The rounding mode is not modelled: the header says so at the floating point
 * status register. Both `nearbyint` and `llrint` honour the mode, so this holds
 * only because nothing here changes it and the host starts at round to nearest
 * with ties to even. A program that called `fesetround` would move these.
 *
 * Rounding happens before the range is tested, because a value just inside the
 * range can round to just outside it: 2147483647.5 is in range and rounds up to
 * 2147483648, which a cast cannot represent.
 */
static inline int64_t xenolith_to_a_whole_number(double given) {
    /* The console's answer for a value that is not a number is its most negative
     * whole number, which is what a machine holding an invalid operation gives. */
    if (isnan(given)) {
        return INT64_MIN;
    }
    /* And for a value too large to be one, it is the largest. The comparison is
     * against 2^63 rather than the largest whole number, because a double cannot
     * hold INT64_MAX: `(double)INT64_MAX` is 2^63, so testing `>` against it
     * would let exactly 2^63 through and the cast would be undefined. */
    double rounded = nearbyint(given);
    if (rounded >= 9223372036854775808.0) {
        return INT64_MAX;
    }
    if (rounded < -9223372036854775808.0) {
        return INT64_MIN;
    }
    return (int64_t)rounded;
}

/* The same for the form that gives a 32-bit whole number.
 *
 * The result is sign extended into the 64-bit register. The instruction does not
 * define what the upper word holds; sign extension is this implementation's
 * choice, taken so that the overflow path and the ordinary path agree. The value
 * out of range is the 32-bit one the console gives, not the 64-bit one: a 32-bit
 * conversion that overflows has nowhere to put a 64-bit answer.
 */
static inline int64_t xenolith_to_a_whole_word(double given) {
    if (isnan(given)) {
        return (int64_t)(int32_t)INT32_MIN;
    }
    double rounded = nearbyint(given);
    if (rounded >= 2147483648.0) {
        return (int64_t)INT32_MAX;
    }
    if (rounded < -2147483648.0) {
        return (int64_t)(int32_t)INT32_MIN;
    }
    return (int64_t)(int32_t)rounded;
}

/* The form that cuts the fraction off rather than rounding.
 *
 * A cast does exactly this, so the cast is the whole of it. What a cast does not
 * do is answer for a value outside the range, and that is the same as above: a
 * value that is not a number, and a value too large, each have the console's one
 * answer and this states it rather than leaving it to the host.
 */
static inline int64_t xenolith_to_a_whole_number_by_cutting(double given) {
    if (isnan(given)) {
        return INT64_MIN;
    }
    if (given >= 9223372036854775808.0) {
        return INT64_MAX;
    }
    if (given < -9223372036854775808.0) {
        return INT64_MIN;
    }
    return (int64_t)given;
}

static inline int64_t xenolith_to_a_whole_word_by_cutting(double given) {
    if (isnan(given)) {
        return (int64_t)(int32_t)INT32_MIN;
    }
    if (given >= 2147483648.0) {
        return (int64_t)INT32_MAX;
    }
    if (given < -2147483648.0) {
        return (int64_t)(int32_t)INT32_MIN;
    }
    return (int64_t)(int32_t)given;
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

/* Turns one of the console's half sized floats into a whole one.
 *
 * It is not quite the one everybody else keeps: there is no value that stands
 * for infinity and none that stands for not a number, so the largest exponent
 * is just the largest exponent, and a value too small to be written with an
 * exponent of its own is nought rather than being written the long way. Both
 * follow from the shift being done as it stands, which is why this is written
 * out rather than handed to a host instruction that would round it the other
 * way.
 */
static inline float xenolith_half_to_float(uint16_t half) {
    uint32_t mantissa = half & 0x3ffu;
    uint32_t exponent = (half >> 10) & 0x1fu;
    uint32_t made;
    float out;

    if (exponent == 0) {
        mantissa = 0;
        exponent = (uint32_t)-112;
    }
    made = ((uint32_t)(half & 0x8000u) << 16) | ((exponent + 112u) << 23) | (mantissa << 13);
    __builtin_memcpy(&out, &made, sizeof out);
    return out;
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

/* One lane of a colour a title is packing into a vertex.
 *
 * The console does not convert the number, it reads the bits underneath it. A
 * shader hands over a value between three and the next two hundred and
 * fifty five representable above it, so the byte wanted is already sitting in
 * the low bits of the float and the whole of the work is deciding whether the
 * value is in that window. Below it, or not a number at all, gives nothing;
 * above it gives everything.
 *
 * Comparing the bits for the upper bound is only sound once the value is known
 * to be at least three, which the first test establishes: over the positive
 * floats the bit patterns run in the same order as the values.
 */
static inline uint32_t xenolith_pack_colour(uint32_t bits) {
    float value;
    __builtin_memcpy(&value, &bits, 4);
    if (!(value >= 3.0f)) {
        return 0;
    }
    if (bits > 0x404000ffu) {
        return 255;
    }
    return bits & 0xffu;
}

/* One lane of a pair or a quartet of signed halfwords, read the same way.
 *
 * The window is the thirty two thousand seven hundred and sixty seven either
 * side of three, and what falls outside it stops at the end it left by. The
 * low bound is one above the most negative halfword, which is what the console
 * saturates to rather than the halfword below it.
 */
static inline uint32_t xenolith_pack_short(uint32_t bits) {
    float value;
    float low;
    uint32_t low_bits = 0x403f8001u;
    __builtin_memcpy(&value, &bits, 4);
    __builtin_memcpy(&low, &low_bits, 4);
    if (!(value >= low)) {
        return 0x8001u;
    }
    if (bits > 0x40407fffu) {
        return 0x7fffu;
    }
    return bits & 0xffffu;
}

/* One lane of a pair or a quartet of half precision values.
 *
 * This one is a conversion rather than a reading of bits, so it rounds, and it
 * rounds to nearest with ties going to even the way every other narrowing in
 * this instruction set does. The format has no infinity and no value that is
 * not a number, so a value too large to hold takes the largest exponent and one
 * that is not a number keeps a mantissa bit set; neither means what the names
 * would mean in an ordinary half, and widening either gives a finite float.
 */
static inline uint32_t xenolith_pack_half(uint32_t bits) {
    uint32_t sign = (bits >> 16) & 0x8000u;
    uint32_t raw = (bits >> 23) & 0xffu;
    uint32_t mantissa = bits & 0x7fffffu;
    int32_t exponent = (int32_t)raw - 127 + 15;
    uint32_t rounded;

    if (raw == 0xffu) {
        return mantissa != 0 ? (sign | 0x7e00u | (mantissa >> 13)) : (sign | 0x7c00u);
    }
    if (exponent >= 0x1f) {
        return sign | 0x7c00u;
    }
    if (exponent <= 0) {
        uint32_t places;
        if (exponent < -10) {
            return sign;
        }
        mantissa |= 0x800000u;
        places = (uint32_t)(14 - exponent);
        rounded = mantissa >> places;
        if ((mantissa & ((1u << places) - 1u)) > (1u << (places - 1u))
            || ((mantissa & ((1u << places) - 1u)) == (1u << (places - 1u)) && (rounded & 1u))) {
            rounded++;
        }
        return sign | rounded;
    }
    rounded = mantissa >> 13;
    if ((mantissa & 0x1fffu) > 0x1000u
        || ((mantissa & 0x1fffu) == 0x1000u && (rounded & 1u))) {
        rounded++;
    }
    if (rounded == 0x400u) {
        rounded = 0;
        exponent++;
        if (exponent >= 0x1f) {
            return sign | 0x7c00u;
        }
    }
    return sign | ((uint32_t)exponent << 10) | rounded;
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
 * The block is the console's cache line, which is 128 bytes. This console
 * spells it two ways, and the decoder folds both into one opcode, so both
 * clear a line. The address is aligned down to the line.
 *
 * A general purpose emulator does not share the line size, so this is the one
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

/* A title asking for a debugger, with the number saying why.
 *
 * A console with no debugger attached carries on past one, and so does this,
 * after saying where it was asked for the first time each place asks. */
void xenolith_break(xenolith_context *ctx, uint8_t *base, uint32_t address, uint32_t why);

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

/* The block a title reaches through r13, which is what it reads the kernel's
 * tick count and its own processor number out of.
 *
 * A title that finds nothing there reads processor nought on every thread, and
 * its vector code picks a slot from that number and steps a counter in it, so
 * two threads on one block step the same counter and carry it past the end.
 */
uint32_t xenolith_processor_block(uint8_t *base);

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
/* What a title says the shape of what it is drawing is.
 *
 * The console's own numbering. Only the ones a title was seen to ask for are
 * named: points, and the ways it asks for a flat piece of screen.
 */
#define XENOLITH_SHAPE_POINTS 1u
#define XENOLITH_SHAPE_LINES 2u
#define XENOLITH_SHAPE_LINE_RUN 3u
#define XENOLITH_SHAPE_TRIANGLES 4u
#define XENOLITH_SHAPE_TRIANGLE_FAN 5u
#define XENOLITH_SHAPE_TRIANGLE_RUN 6u
#define XENOLITH_SHAPE_RECTANGLES 8u
#define XENOLITH_SHAPE_QUADS 13u

/* Says a shader in a host's terms while a title runs, for one it loaded from
 * the data beside its executable rather than out of the executable itself.
 *
 * Returns how many words the answer takes, which is nought where the shader
 * could not be said. Where that is more than there was room for, nothing is
 * written and the caller may ask again with more.
 *
 * Only there where the library that says shaders was linked in. */
uint32_t xenolith_say_a_shader(uint32_t kind, const uint32_t *from, uint32_t count,
                               uint32_t *into, uint32_t room);

/* How many shaders said while a title runs are kept, so that one handed over
 * again is not said again. A title's scene loads a few hundred, and saying one
 * costs more than drawing with it, so this is room for all of them. */
#define XENOLITH_SAID_WHILE_RUNNING 4096

/* How many counts come before the words of a shader said while a title runs. */
#define XENOLITH_COUNTS 3u

/* How many words each stream of a shader said while a title runs takes. */
#define XENOLITH_WORDS_A_STREAM 5u

const uint32_t *xenolith_said(uint32_t kind, uint32_t sum, uint32_t words,
                              uint32_t *length);

/* Where a shader goes for its vertices, as five words a stream: which fetch
 * constant says where it is, how far apart two vertices sit in it, how far into
 * one this reads, how much of it, and one where the number it fetches by is
 * one the shader worked out rather than the vertex's own. Nothing where the
 * shader fetches none.
 *
 * A host needs these where it has to lay a vertex out itself rather than hand
 * over what a title wrote, which is what a shape the hardware completes asks
 * for. Found the same way a shader is, because it belongs to one.
 */
const uint32_t *xenolith_reads(uint32_t kind, uint32_t sum, uint32_t words,
                               uint32_t *streams);

/* Which of the places a picture is kept a shader reads one from. Nothing where
 * it reads none.
 *
 * A host makes ready only the pictures a shader will look at. Making every place
 * ready instead means turning a picture round and handing it over for each one a
 * title has named, whether anything reads it or not, which for a title that
 * names a whole screen is gigabytes of work a frame that nothing looks at.
 */
const uint32_t *xenolith_looks_at(uint32_t kind, uint32_t sum, uint32_t words,
                                  uint32_t *pictures);

/* Whether there is anywhere to draw, which there is not on a machine with no
 * graphics driver and in a build made without one. Says so once. */
int xenolith_can_draw(void);

/* Hands a shader the constants a title wrote, taken from where a title wrote
 * them and turned into the order a host reads. */
int xenolith_hand_over_constants(const uint8_t *from, const uint8_t *truths,
                                 const uint8_t *counts);

/* Says what the hardware would have done to a position a shader put out, as
 * something to multiply by and something to add. Passing nothing says to leave
 * a position as the shader gave it. */
void xenolith_move_a_position_by(const float *scale, const float *offset);


/* Keeps drawing inside the window a title named, in the pixels of what is
 * being drawn into. A window reaching past the edge is brought back to it. */
void xenolith_clip_to(uint32_t left, uint32_t top, uint32_t right, uint32_t bottom);

/* Hands over a picture a shader reads, laid out the way a host reads one.
 *
 * The console keeps a picture in an order that puts the parts near one another
 * on the screen near one another in memory, and squeezed into squares of four
 * by four. A host squeezes them the same way, so what is squeezed is handed over
 * as it is and only the order is undone. Says whether it took.
 *
 * How it is held says how it is sampled in its lowest twelve bits, and above
 * them which part of the picture each of the four a shader reads is, three bits
 * each in the console's numbering.
 */
int xenolith_look_at_a_picture(uint32_t slot, const uint8_t *from, uint32_t where, uint32_t across,
                               uint32_t down, uint32_t pitch, int in_squares, uint32_t order,
                               uint32_t kept, uint32_t held_how);

/* Leaves a place a shader reads a picture from holding nothing.
 *
 * A title fills the same few places over and over, so a place left as it was is
 * a place still holding a picture the title named for an earlier draw. Where
 * what it names now cannot be read, that is another of the title's own pictures
 * drawn in its stead, which looks like a drawing gone wrong rather than like
 * something not being read. */
void xenolith_look_at_nothing(uint32_t slot);

/* Says what shape of place a title is about to draw into, so that what each of
 * them holds is kept apart from what the others hold, and how many samples
 * across and down each of its pixels is. */
void xenolith_drawing_into(uint32_t shaped, uint32_t across, uint32_t down);

/* Says which register the next draw's pixel shader finds where its pixel is
 * in, which a title asks the hardware for rather than working out, or all ones
 * where it asks for none. */
void xenolith_find_the_pixel_in(uint32_t which);

/* Says whether the copy just made asked for how far away things are to start
 * again before the next drawing, and what to start it as. */
void xenolith_wipe_how_far_after_a_copy(int wipe, float to);

/* Says whether the copy just made asked for the colours to start again before
 * the next drawing, and what to start them as. */
void xenolith_wipe_the_colours_after_a_copy(int wipe, uint32_t to);

/* Says which kit a title was built against, read out of its container.
 *
 * The graphics library a title links keeps its device in a layout that changes
 * between kits, and the runtime answers a few words of that layout: how far the
 * hardware has got, in the two forms the library keeps. Told the kit, it knows
 * where those words are; told nothing, it answers neither and a title that
 * watches them decides the hardware has stopped. */
void xenolith_built_against(uint32_t build);

/* Says what a title's own container declares it is allowed to do.
 *
 * A title asks whether it holds a numbered privilege, and the answer is a bit
 * in the system flags word its executable carries. That word is in the
 * container and nowhere else, so it is passed in rather than guessed at. A
 * title told it holds nothing takes the path for a build that was not licensed
 * to do what it does, which on the titles tried so far is to shut itself down
 * before it draws anything.
 *
 * Nought means the container did not say, which is answered the same way as a
 * word with the bit clear: nothing here invents a privilege. */
void xenolith_privileges(uint32_t flags);

/* Gives a variable the kernel exports a home in guest memory, and puts its
 * address in the slot the title reads it through.
 *
 * A title imports a few of the kernel's variables as well as its functions, and
 * reads each through a slot that the console's loader fills with the variable's
 * address. Nothing else fills it here, so the boot program calls this once for
 * each slot, after saying what kernel build the title was made against. Each
 * starts as nothing, and the ones a title reads a value out of are given one: the
 * graphics clock, the kernel's version, and the clock readings, which are kept
 * current for as long as the run lasts. One this does not know is reported. */
void xenolith_import_variable(uint8_t *base, uint32_t slot, const char *library,
                              uint32_t ordinal);

/* Somewhere to put the vertices a shader goes and gets, and how many words of it
 * there is room for. Returns nothing where there is nothing to draw with. */
uint32_t *xenolith_room_for_vertices(uint32_t *how_many);

/* Where the room last given begins, in words from the front of what a shader
 * fetches from, and through `given_out` a number that changes whenever room
 * given earlier is given out again, which says whether words put there earlier
 * are still there for a later draw to read. */
uint32_t xenolith_where_the_room_begins(uint32_t *given_out);

/* Says where each of the next draw's streams begins, a word for each of the
 * ninety six a fetch can name, in words from the front of what a shader fetches
 * from. Said after the draw's constants are handed over. */
void xenolith_streams_begin_at(const uint32_t *where);

/* Says the vertices are in place, and how many words of the room given were
 * written. A frame keeps every draw's words at once, so what one draw took has
 * to be known before the next is given room. */
void xenolith_done_with_vertices(uint32_t words);

/* Says what number each vertex of the next draw arrives at its shader with:
 * the `count` words `at` words into the room the vertices were put in, one for
 * each vertex drawn, or where `count` is nought, one after another from
 * `first`. A word of all ones in the first starts a new run of shapes.
 *
 * A shader works out what it fetches from the number it is given, and on the
 * console that number is the title's own, however the shape is put together.
 * Laying the vertices out again in the order they are drawn instead, as a host
 * otherwise has to for a quad, hands the shader numbers the title never gave:
 * one that reads a particle's corner and centre at a quarter of its number
 * reads another particle's. So the shape is drawn by the title's numbers. */
void xenolith_number_the_vertices(uint32_t at, uint32_t count, uint32_t first);

/* Says how much further away what is drawn next is taken to be when it is
 * compared with what is already there: a distance added outright, as a
 * fraction of the whole depth, and one added for each sixteenth of a pixel a
 * shape slopes away by, which is how the console's polygon offset is said.
 * Nought for both draws as the shape lies.
 *
 * A title draws a light or a mark flat onto a surface it has already drawn,
 * and pushes it a little nearer so that it is not half hidden by the surface
 * it lies on. */
void xenolith_push_away_by(float outright, float along_a_slope);

/* Forgets any picture already turned that came out of memory the title has
 * since written over, so that the next draw reads what is there now. */
void xenolith_forget_pictures_at(uint32_t where, uint32_t bytes);

/* Shows the drawing that was last copied out to where a title names, rather
 * than whatever has been drawn since. Says whether there was one to show. */
int xenolith_show_what_was_copied_to(uint32_t where);

/* Shows whatever is on the drawing surface now, for a title that hands a frame
 * straight to the display without copying it out first. The copied-frame path
 * above is preferred where a title uses it; this is the fallback. */
int xenolith_show_the_frame(void);

/* Says where on what is drawn into a position between one and minus one lands,
 * as the console says it: what to multiply a position by and what to add.
 * Nothing said means the whole of what is drawn into. */
void xenolith_look_at(const float *scale, const float *offset);

/* What a title asked to have copied out of what it drew, and where to.
 *
 * The rectangle is in the pixels of the place drawn into, and is where it lands
 * in the picture copied into as well: a title drawing a frame in parts moves
 * where the picture starts for each part rather than the rectangle. How many
 * samples a place keeps across and down for each pixel says how much of the
 * surface one pixel covers, and those are averaged into it on the way out.
 *
 * A title asks for how far away things came out as often as it asks for their
 * colours, and reads that back as a picture to work out haze, fog and what is
 * out of focus. Handed the colours instead it draws none of those, so which of
 * the two is wanted is said here. How far away is kept either as a fraction of
 * the whole or as the console's small float. A colour goes out red last unless
 * the copy asks for red and blue to be swapped. */
struct xenolith_copy {
    uint32_t into;
    uint32_t pitch;
    uint32_t height;
    uint32_t left;
    uint32_t top;
    uint32_t right;
    uint32_t bottom;
    uint32_t samples_across;
    uint32_t samples_down;
    int as_one_float;
    int from_how_far;
    int how_far_as_a_float;
    int red_and_blue_swapped;
};

/* How many bytes from where a copy starts it writes over, kept in squares of
 * thirty two parts on a side, four bytes each, laid a row of squares at a time.
 *
 * Up to the last square it writes rather than a whole picture's worth from
 * where it starts. A copy of the right half of a frame starts part of the way
 * into its picture, and a whole picture's worth from there reaches into
 * whatever the title keeps after it. */
static inline uint32_t xenolith_what_a_copy_reaches(uint32_t pitch, uint32_t right,
                                                    uint32_t bottom) {
    uint32_t squares_a_row = (pitch + 31u) / 32u;
    uint32_t rows = (bottom + 31u) / 32u;
    if (rows == 0 || right == 0) {
        return 0;
    }
    return ((rows - 1u) * squares_a_row + (right + 31u) / 32u) * 4096u;
}

/* Hands what was drawn back into a title's own memory, as the copy says.
 * Returns zero where there is nothing to hand back.
 *
 * The copy is recorded with what was drawn, and the memory is written once
 * that is handed over: when a frame is shown, when a picture is made out of
 * that memory, or when xenolith_wait_for_what_was_drawn is asked. */
int xenolith_hand_back_what_was_drawn(uint8_t *base, const struct xenolith_copy *copy);

/* Hands over everything recorded so far, waits for it to run, and writes what
 * the copies among it read back into the memory their titles named. A copy's
 * memory holds what was copied once this has been asked for. */
int xenolith_wait_for_what_was_drawn(void);

/* Hands over everything recorded so far without waiting for it to run, so
 * that the device draws it while the next is recorded. What copies among it
 * read reaches memory once a later wait, or a picture made out of that memory,
 * asks for it. */
int xenolith_send_what_was_drawn(void);

/* What is held down on the keyboard, as a controller's buttons, the two
 * thumbsticks across and up, the left one first, and how far the two triggers
 * are pulled, the left one first. Returns zero where there is no window to
 * read one from. */
int xenolith_what_is_held(uint16_t *buttons, int16_t *thumbs, uint8_t *triggers);

/* Starts a frame, cleared to nothing. Reports whether there was anywhere to
 * start one. */
int xenolith_begin_a_frame(void);

/* The test that draws a place however solid it is, which is what a title asking
 * for no test at all comes to. Its three bits stand for drawing a place less
 * solid than the test, exactly as solid, and more solid. */
#define XENOLITH_DRAWN_HOWEVER_SOLID 7u

/* A word of a title's vertices as a fetch reads it, from the word as the
 * processor reads it and the order the fetch says its bytes are in.
 *
 * The processor reads a word with its first byte highest. A fetch says how the
 * graphics hardware turns the bytes it reads: not at all (nought), within each
 * half (one), whole (two, which is the processor's own order), or the halves
 * about each other (three). A title keeping pairs of halfwords turns within
 * each half, and read the processor's way the two halves of every pair change
 * places. */
static inline uint32_t xenolith_as_a_fetch_reads(uint32_t word, uint32_t order) {
    switch (order & 3u) {
    case 0:
        return __builtin_bswap32(word);
    case 1:
        return (word << 16) | (word >> 16);
    case 3:
        return ((word & 0x00ff00ffu) << 8) | ((word >> 8) & 0x00ff00ffu);
    default:
        return word;
    }
}

/* The mode a title draws in when only how far away things are is kept. */
#define XENOLITH_ONLY_HOW_FAR 5u

/* The bit of what a title lays a colour down with that says a place is to be
 * thrown away where how solid it is does not answer the test beside it. */
#define XENOLITH_A_TEST_IS_SET 8u

/* The test a draw is given, from how a title says colours are laid down and the
 * mode it draws in.
 *
 * A draw that keeps only how far away things are runs no pixel shader on the
 * console, so nothing it would have put out is tested. A title wipes its
 * shadows that way with a shader putting out nothing solid and the test left
 * on from the pass before, and tested here the wipe was thrown away whole. */
static inline uint32_t xenolith_test_a_draw_is_given(uint32_t colour_control, uint32_t mode) {
    if (mode == XENOLITH_ONLY_HOW_FAR || (colour_control & XENOLITH_A_TEST_IS_SET) == 0) {
        return XENOLITH_DRAWN_HOWEVER_SOLID;
    }
    return colour_control & 7u;
}

/* Draws what a title asked to draw, with the shaders it last handed over.
 *
 * A place is thrown away before it is drawn where how solid it is does not
 * answer `cut_off`, the console's own three bits, against `cut_off_at`, which
 * carries the bits of the number they are measured against. */
int xenolith_draw_now(uint32_t blending, uint32_t written, uint32_t how_far, uint32_t cut_off,
                      uint32_t cut_off_at, uint32_t vertex_sum, const uint32_t *vertex,
                      uint32_t vertex_length, uint32_t pixel_sum, const uint32_t *pixel,
                      uint32_t pixel_length, uint32_t vertices, uint32_t kind);

/* Finishes a frame and reads it back. */
int xenolith_end_a_frame(void);

/* Whether a pair of shaders can be drawn with, building what a device needs
 * the first time the pair is asked for. */
int xenolith_can_draw_with(uint32_t vertex_sum, const uint32_t *vertex, uint32_t vertex_length,
                           uint32_t pixel_sum, const uint32_t *pixel, uint32_t pixel_length);

/* Writes what was drawn where somebody can look at it. */
int xenolith_keep_a_frame(const char *where);

/* How far a decoder of compressed sound is behind the frames it was handed. */
struct xenolith_frames_kept {
    int64_t behind;
    int64_t settled;
    int mending;
};

/* Counts the frames a packet finished against the samples the decoder gave for
 * it, and says how many samples of silence make up for frames it threw away.
 *
 * A decoder holds the same samples back for as long as it decodes well, so how
 * far it is behind stays put. One that throws a broken frame away, and the rest
 * of its packet with it, falls behind by just what was lost, and may give some
 * of that late. So what is owed is said only once it gives sound again from a
 * packet that was not broken. */
static inline uint32_t xenolith_samples_owed(struct xenolith_frames_kept *kept, uint32_t frames,
                                             uint32_t samples_a_frame, uint32_t given,
                                             int broken) {
    kept->behind += (int64_t)frames * samples_a_frame - given;
    if (broken) {
        kept->mending = 1;
        return 0;
    }
    if (given == 0) {
        return 0;
    }
    uint32_t owed = 0;
    if (kept->mending && kept->behind > kept->settled) {
        owed = (uint32_t)(kept->behind - kept->settled);
        kept->behind = kept->settled;
    }
    kept->mending = 0;
    kept->settled = kept->behind;
    return owed;
}

/* What a controller was last read as. */
struct xenolith_pad_seen {
    uint16_t held;
    int16_t tilted[4];
    uint8_t pulled[2];
    uint32_t reading;
};

/* The number a controller's reading carries, which moves on each time what it
 * holds changes.
 *
 * A title handed the same number twice takes it that nothing changed and looks
 * no further. Always the same, one title never saw a button pressed and could
 * not be told to skip what it was showing. */
static inline uint32_t xenolith_pad_reading(struct xenolith_pad_seen *seen, uint16_t buttons,
                                            const int16_t thumbs[4], const uint8_t triggers[2]) {
    int moved = buttons != seen->held;
    for (int at = 0; at < 4; at++) {
        moved |= thumbs[at] != seen->tilted[at];
        seen->tilted[at] = thumbs[at];
    }
    for (int at = 0; at < 2; at++) {
        moved |= triggers[at] != seen->pulled[at];
        seen->pulled[at] = triggers[at];
    }
    seen->held = buttons;
    seen->reading += (uint32_t)moved;
    return seen->reading;
}

#define XENOLITH_STACK_TOP 0x70000000u

#endif /* XENOLITH_H */
