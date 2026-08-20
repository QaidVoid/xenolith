/* A runtime that implements the interface, and the little of the console it
 * takes to get a title as far as its own code.
 *
 * What this is for is linking. An emitted title compiles a unit at a time
 * whether or not the units agree with each other, and only a link says they do:
 * that every address named has a definition, that none has two, and that
 * nothing was left declared and forgotten.
 *
 * What this is NOT is an environment a game can run in. No thread exists and
 * nothing draws. A handful of kernel entry points are implemented, each one
 * because a title reached it on the way out of its own startup, and each one
 * saying beside it what it is founded on. Everything else reports itself and
 * stops, which is an accurate account of how far this has got.
 */

#include "xenolith.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
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
void xenolith_dispatch(xenolith_context *ctx, uint8_t *base, uint32_t address,
                       uint32_t from) {
    xenolith_function target = xenolith_lookup(address);
    if (target == NULL) {
        fprintf(stderr,
                "xenolith: dispatched from %#010x to %#010x, which is not a lifted function\n",
                from, address);
        exit(2);
    }
    target(ctx, base);
}

/* Where the guest's heap is handed out from.
 *
 * Clear of the image, of the stack, and of the low addresses a null pointer
 * would reach. The whole space is mapped and zeroed before a title runs, so
 * handing out an address is the whole of what allocating one takes here.
 */
#define XENOLITH_HEAP_BASE 0x30000000u
#define XENOLITH_HEAP_END 0x6f000000u

/* What the console allocates in. A request is rounded up to this, and the next
 * one starts on it, so no two overlap. */
#define XENOLITH_HEAP_GRAIN 0x10000u

/* Success, as the kernel reports it. */
#define XENOLITH_STATUS_SUCCESS 0u

/* No memory left. */
#define XENOLITH_STATUS_NO_MEMORY 0xC0000017u

/* Reserving and committing guest memory.
 *
 * The documented shape is what is implemented: two pointers into guest memory
 * holding an address and a size, both read and both written back, and a status
 * returned. Where the address arrives as zero the runtime chooses one.
 *
 * The flags saying whether this reserves or commits are deliberately not
 * consulted. Every guest address is mapped and zeroed before a title starts, so
 * there is no state here for the two to differ in, and honouring a distinction
 * this runtime does not have would be a pretence.
 */
static void allocate_virtual_memory(xenolith_context *ctx, uint8_t *base) {
    static uint32_t next = XENOLITH_HEAP_BASE;

    uint32_t address_at = (uint32_t)ctx->r[3];
    uint32_t size_at = (uint32_t)ctx->r[4];
    uint32_t wanted = address_at ? xenolith_load32(base, address_at) : 0;
    uint32_t size = size_at ? xenolith_load32(base, size_at) : 0;

    uint32_t grains = (size + XENOLITH_HEAP_GRAIN - 1) / XENOLITH_HEAP_GRAIN;
    uint32_t rounded = grains * XENOLITH_HEAP_GRAIN;
    if (rounded == 0) {
        rounded = XENOLITH_HEAP_GRAIN;
    }

    uint32_t at = wanted & ~(XENOLITH_HEAP_GRAIN - 1);
    if (at == 0) {
        if (rounded > XENOLITH_HEAP_END - next) {
            ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
            return;
        }
        at = next;
        next += rounded;
    }

    if (address_at) {
        xenolith_store32(base, address_at, at);
    }
    if (size_at) {
        xenolith_store32(base, size_at, rounded);
    }
    ctx->r[3] = XENOLITH_STATUS_SUCCESS;
}

/* Says once that an answer rests on an assumption rather than on evidence.
 *
 * Most of what is implemented here follows from something checkable: a
 * documented shape, or the fact that this runtime has one thread. A few answers
 * do not, and rather than let those pass for the rest they say so the first
 * time they are given. Nothing here is silently guessed.
 */
static void assumed(const char *what, const char *why) {
    static const char *said[16];
    static unsigned count = 0;

    for (unsigned at = 0; at < count; at++) {
        if (said[at] == what) {
            return;
        }
    }
    if (count < sizeof said / sizeof *said) {
        said[count++] = what;
    }
    fprintf(stderr, "xenolith: assuming %s, because %s\n", what, why);
}

/* Which kind of process a title runs as.
 *
 * Assumed. A title is not the kernel and not the idle process, so of the three
 * kinds it is the user one, and one is the value that names it. The title
 * records what it is told here in a structure of its own and does not branch on
 * it, so the startup path does not turn on this being right.
 */
static void current_process_type(xenolith_context *ctx) {
    assumed("a title runs as a user process",
            "it is neither the kernel nor the idle process");
    ctx->r[3] = 1;
}

/* Whether the executable holds a privilege.
 *
 * Assumed. What each privilege number means is not recorded anywhere this can
 * read, and a title that is told it holds one it does not would take a path
 * nothing here can service, so the safer answer of the two is that it does not.
 */
static void check_executable_privilege(xenolith_context *ctx) {
    assumed("a title holds no special privilege",
            "what each privilege means is not recorded in the container");
    ctx->r[3] = 0;
}

/* A lock, on a runtime that has one thread.
 *
 * Nothing can be contended where nothing else runs, so entering and leaving are
 * bookkeeping and initialising is clearing what the title will read back. This
 * is the same reasoning the reservation below rests on, and it stops being true
 * the moment a second thread exists.
 */
static void initialize_critical_section(xenolith_context *ctx, uint8_t *base) {
    uint32_t at = (uint32_t)ctx->r[3];
    if (at != 0) {
        for (uint32_t word = 0; word < 7; word++) {
            xenolith_store32(base, at + word * 4, 0);
        }
    }
}

/* Slots a title keeps its per thread state in.
 *
 * One thread, so one set of them. A slot is handed out in order and never
 * given back, which is what a title expects of the ones it takes at startup.
 */
#define XENOLITH_TLS_SLOTS 64

static uint32_t tls_values[XENOLITH_TLS_SLOTS];
static uint32_t tls_taken = 0;

static void tls_alloc(xenolith_context *ctx) {
    if (tls_taken >= XENOLITH_TLS_SLOTS) {
        ctx->r[3] = 0xFFFFFFFFu;
        return;
    }
    ctx->r[3] = tls_taken;
    tls_taken++;
}

static void tls_set_value(xenolith_context *ctx) {
    uint32_t slot = (uint32_t)ctx->r[3];
    if (slot < XENOLITH_TLS_SLOTS) {
        tls_values[slot] = (uint32_t)ctx->r[4];
    }
    ctx->r[3] = 1;
}

/* Giving a slot back. Nothing reuses one, since a title takes the few it wants
 * at startup and keeps them, and handing the same slot to a later caller would
 * be worse than running out. */
static void tls_free(xenolith_context *ctx) {
    uint32_t slot = (uint32_t)ctx->r[3];
    if (slot < XENOLITH_TLS_SLOTS) {
        tls_values[slot] = 0;
    }
    ctx->r[3] = 1;
}

static void tls_get_value(xenolith_context *ctx) {
    uint32_t slot = (uint32_t)ctx->r[3];
    ctx->r[3] = slot < XENOLITH_TLS_SLOTS ? tls_values[slot] : 0;
}

/* What the runtime answers to, by ordinal within a library.
 *
 * Each entry is here because a title reached it. Nothing is implemented ahead
 * of a title asking for it, so this list is a record of what was needed rather
 * than a guess at what might be.
 */
static int serviced(xenolith_context *ctx, uint8_t *base, const char *library,
                    uint32_t ordinal) {
    if (strcmp(library, "xboxkrnl.exe") != 0) {
        return 0;
    }
    switch (ordinal) {
    case 102:
        current_process_type(ctx);
        return 1;
    case 204:
        allocate_virtual_memory(ctx, base);
        return 1;
    case 293:
    case 304:
        /* Entering and leaving a lock nothing else can hold. */
        return 1;
    case 302:
        initialize_critical_section(ctx, base);
        return 1;
    case 338:
        tls_alloc(ctx);
        return 1;
    case 339:
        tls_free(ctx);
        return 1;
    case 340:
        tls_get_value(ctx);
        return 1;
    case 341:
        tls_set_value(ctx);
        return 1;
    case 404:
        check_executable_privilege(ctx);
        return 1;
    default:
        return 0;
    }
}

/* Nothing here provides what the console did beyond the above. Reporting which
 * import was wanted is the whole of what this can honestly do.
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

    if (serviced(ctx, base, library, ordinal)) {
        return;
    }

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
