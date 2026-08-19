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

#include <stdint.h>

/* How many registers each bank holds. The vector bank is the console's, which
 * reaches four times as far as the architecture it extends. */
#define XENOLITH_GENERAL_REGISTERS 32
#define XENOLITH_FLOATING_REGISTERS 32
#define XENOLITH_VECTOR_REGISTERS 128
#define XENOLITH_CONDITION_FIELDS 8

/* A vector register, addressable as the lanes the instructions use. */
typedef union xenolith_vector {
    uint8_t u8[16];
    uint16_t u16[8];
    uint32_t u32[4];
    uint64_t u64[2];
    float f32[4];
    double f64[2];
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
void xenolith_dispatch(xenolith_context *ctx, uint8_t *base, uint32_t address);

/* Where a call to an imported function goes.
 *
 * The container names an import by ordinal within a library and never by name,
 * so that is everything known about which function is meant. What the ordinal
 * does is the environment's to decide. Implemented there, not here.
 */
void xenolith_import(xenolith_context *ctx, uint8_t *base, const char *library,
                     uint32_t ordinal);

#endif /* XENOLITH_H */
