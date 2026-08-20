/* A runtime that implements the interface and nothing more.
 *
 * What this is for is linking. An emitted title compiles a unit at a time
 * whether or not the units agree with each other, and only a link says they do:
 * that every address named has a definition, that none has two, and that
 * nothing was left declared and forgotten.
 *
 * What this is NOT is an environment a game can run in. No import is serviced,
 * no thread exists, and nothing draws. A program built against this reaches its
 * first call into the operating system and stops there, which is an accurate
 * account of how far the translation has got.
 */

#include "xenolith.h"

#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>

/* How many calls a trace reports before giving up.
 *
 * A title told that every import returned zero goes wrong quickly, and once it
 * has, the calls it makes say more about the lie than about the title. */
#define XENOLITH_TRACE_LIMIT 20000

/* The guest addresses a 32-bit pointer can form.
 *
 * Reserved rather than committed. Only the pages actually touched become
 * resident, so the cost of covering the whole space is address space rather
 * than memory.
 */
#define XENOLITH_GUEST_SPAN ((size_t)1 << 32)

uint8_t *xenolith_boot(const char *image, uint32_t load_address) {
    void *base = mmap(NULL, XENOLITH_GUEST_SPAN, PROT_READ | PROT_WRITE,
                      MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (base == MAP_FAILED) {
        fprintf(stderr, "xenolith: could not map the guest address space\n");
        return NULL;
    }

    FILE *file = fopen(image, "rb");
    if (file == NULL) {
        fprintf(stderr, "xenolith: could not open %s\n", image);
        return NULL;
    }

    uint8_t *at = (uint8_t *)base + load_address;
    size_t read = fread(at, 1, XENOLITH_GUEST_SPAN - load_address, file);
    fclose(file);
    if (read == 0) {
        fprintf(stderr, "xenolith: %s held nothing\n", image);
        return NULL;
    }
    return (uint8_t *)base;
}

/* A trap leaves the function it was in, and there is nowhere for it to go. */
void xenolith_trap(xenolith_context *ctx, uint8_t *base, uint32_t address) {
    (void)ctx;
    (void)base;
    fprintf(stderr, "xenolith: trapped at %#010x\n", address);
    exit(2);
}

/* An address unknown when the code was emitted, resolved against the functions
 * that were. One that is not among them is not something this program can
 * reach, and guessing would be worse than stopping. */
void xenolith_dispatch(xenolith_context *ctx, uint8_t *base, uint32_t address) {
    xenolith_function target = xenolith_lookup(address);
    if (target == NULL) {
        fprintf(stderr, "xenolith: dispatched to %#010x, which is not a lifted function\n",
                address);
        exit(2);
    }
    target(ctx, base);
}

/* Nothing here provides what the console did. Reporting which import was wanted
 * is the whole of what this can honestly do.
 *
 * Naming one is not enough to know what a title needs, because the first is all
 * a run ever reaches. Setting XENOLITH_TRACE_IMPORTS reports each one and
 * carries on as though it had returned zero, which walks the title far enough
 * to see what else it asks for.
 *
 * That is a diagnostic and not an environment. Returning zero from something
 * that was meant to allocate memory or open a file is a lie the title will
 * believe, so the run after the first import means nothing except as a list of
 * what was wanted. It is labelled on every line so that no output of this can be
 * mistaken for a title running.
 */
void xenolith_import(xenolith_context *ctx, uint8_t *base, const char *library,
                     uint32_t ordinal, const char *name) {
    static int tracing = -1;
    static unsigned long reached = 0;

    (void)base;
    if (tracing < 0) {
        tracing = getenv("XENOLITH_TRACE_IMPORTS") != NULL;
    }

    if (!tracing) {
        fprintf(stderr, "xenolith: %s ordinal %u (%s) is not implemented\n", library,
                ordinal, name ? name : "unnamed");
        exit(3);
    }

    if (reached >= XENOLITH_TRACE_LIMIT) {
        fprintf(stderr, "xenolith: trace: stopping after %lu calls\n", reached);
        exit(3);
    }
    reached++;

    /* Where it was called from, and the registers the calling convention puts
     * arguments in. Which of them mean anything depends on the import, and
     * nothing here knows which, so all of them are reported. */
    printf("trace %s %-34s ordinal %-5u from %#010llx  args", library,
           name ? name : "unnamed", ordinal, (unsigned long long)ctx->lr);
    for (int at = 3; at <= 10; at++) {
        printf(" %016llx", (unsigned long long)ctx->r[at]);
    }
    printf("\n");
    fflush(stdout);

    /* Carrying on as though it returned nothing. */
    ctx->r[3] = 0;
}

/* One thread, so a reservation cannot be lost between taking it and redeeming
 * it, and a conditional store always happens. An environment with threads
 * replaces these with real atomics rather than changing the emitted code. */
uint32_t xenolith_reserve32(const uint8_t *base, uint32_t address) {
    return xenolith_load32(base, address);
}

uint64_t xenolith_reserve64(const uint8_t *base, uint32_t address) {
    return xenolith_load64(base, address);
}

uint8_t xenolith_conditional32(uint8_t *base, uint32_t address, uint32_t value) {
    xenolith_store32(base, address, value);
    return 1;
}

uint8_t xenolith_conditional64(uint8_t *base, uint32_t address, uint64_t value) {
    xenolith_store64(base, address, value);
    return 1;
}

/* A counter that advances, which is all a timing loop needs to finish. What it
 * counts at bears no relation to the console. */
uint64_t xenolith_timebase(void) {
    static uint64_t ticks;
    return ++ticks;
}
