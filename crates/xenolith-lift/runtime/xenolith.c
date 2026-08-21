/* A runtime that implements the interface, and the little of the console it
 * takes to get a title as far as its own code.
 *
 * What this is for is linking. An emitted title compiles a unit at a time
 * whether or not the units agree with each other, and only a link says they do:
 * that every address named has a definition, that none has two, and that
 * nothing was left declared and forgotten.
 *
 * What this is NOT is an environment a game can run in. Threads are real and a
 * title's startup runs, but nothing draws: a title's drawing commands are
 * written into a buffer that nothing reads. The kernel entry points here are
 * each present because a title reached one on the way through its own startup,
 * and each says beside it what it is founded on. Everything else reports itself
 * and stops, which is an accurate account of how far this has got.
 */

/* Asked for before anything is included, because a strict C compiler hides the
 * threading interface otherwise and this needs the whole of it. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"

#include <stdio.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>

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

/* Whether this run is collecting the addresses it wanted rather than calling
 * them.
 *
 * Reaching an address is the evidence that it is a function, and one run
 * reaches many. Setting XENOLITH_TRACE_DISPATCH reports each and carries on
 * without making the call, so a run collects the whole list rather than
 * stopping at the first.
 *
 * Not making a call the title expected is a lie, like answering an import with
 * nothing, and everything after the first one describes the answer given rather
 * than the title. The list is what this is for.
 */
static int collecting_addresses(void) {
    static int tracing = -1;
    if (tracing < 0) {
        tracing = getenv("XENOLITH_TRACE_DISPATCH") != NULL;
    }
    return tracing;
}

/* Reports an address a run wanted and no emitted function answers to.
 *
 * Both callers print the same line when collecting, because both are asking the
 * same question and `lift --roots-from` reads one list.
 */
static void wanted_but_not_lifted(uint32_t address, uint32_t from, const char *what) {
    if (collecting_addresses()) {
        printf("dispatch %#010x from %#010x\n", address, from);
        fflush(stdout);
        return;
    }
    fprintf(stderr, "xenolith: %s %#010x from %#010x, which is not a lifted function\n",
            what, address, from);
    exit(2);
}

/* A trap leaves the function it was in, and there is nowhere for it to go. */
void xenolith_trap(xenolith_context *ctx, uint8_t *base, uint32_t address) {
    (void)ctx;
    (void)base;
    fprintf(stderr, "xenolith: trapped at %#010x\n", address);
    exit(2);
}

/* A function the translation could not express.
 *
 * Reaching one means the title wanted code that is not there, which is a gap in
 * the translation rather than anything the title did wrong. Setting
 * XENOLITH_TRACE_UNLIFTED reports each and returns as though it had done
 * nothing, which walks a title past a gap to see what it asks for on the other
 * side.
 *
 * Returning from a function that should have done something is a lie, the same
 * as answering an import with nothing. What comes after describes the answer
 * given rather than the title.
 */
void xenolith_unlifted(xenolith_context *ctx, uint8_t *base, uint32_t address) {
    static int tracing = -1;

    (void)ctx;
    (void)base;
    if (tracing < 0) {
        tracing = getenv("XENOLITH_TRACE_UNLIFTED") != NULL;
    }
    if (tracing) {
        printf("unlifted %#010x\n", address);
        fflush(stdout);
        return;
    }

    /* Reaching one of these means either that the translation refused the
     * function or that nothing ever discovered it, and from here the two look
     * the same. The second is worth feeding back, since a round of discovery
     * given the address will find it, and the first costs nothing to feed back
     * because it was already found. So it is reported the same way an
     * unreachable branch target is, and the same list takes both. */
    wanted_but_not_lifted(address, 0, "was never lifted, so nothing to call at");
}

/* An address unknown when the code was emitted, resolved against the functions
 * that were. One that is not among them is not something this program can
 * reach, and guessing would be worse than stopping. */
void xenolith_dispatch(xenolith_context *ctx, uint8_t *base, uint32_t address,
                       uint32_t from) {
    xenolith_function target = xenolith_lookup(address);
    if (target == NULL) {
        wanted_but_not_lifted(address, from, "dispatched to");
        return;
    }
    target(ctx, base);
}

/* Where the guest's heap is handed out from.
 *
 * Clear of the image, of the stack, and of the low addresses a null pointer
 * would reach. The whole space is mapped and zeroed before a title runs, so
 * handing out an address is the whole of what allocating one takes here.
 *
 * Where it sits is not arbitrary. A title hands the graphics hardware addresses
 * it has converted itself, by clearing the top three bits, because on the
 * console those bits choose which view of the same memory an address is and the
 * hardware wants the memory rather than the view. That conversion loses nothing
 * only if what it is applied to is already below the boundary. Handing out an
 * address above it means a title converts it, hands it over, and reads back
 * from somewhere the runtime never wrote, with no fault and no sign of one.
 *
 * So the heap lives where the conversion is the identity. That it is also the
 * size of the memory the console actually had is worth having: a title that
 * asks for more than one held will be told no here, as it would have been
 * there.
 */
#define XENOLITH_HEAP_BASE 0x00100000u
#define XENOLITH_HEAP_END 0x1f000000u

/* What the console allocates in. A request is rounded up to this, and the next
 * one starts on it, so no two overlap. */
#define XENOLITH_HEAP_GRAIN 0x10000u

/* Success, as the kernel reports it. */
#define XENOLITH_STATUS_SUCCESS 0u

/* No memory left. */
#define XENOLITH_STATUS_NO_MEMORY 0xC0000017u

/* What starting networking reports where there is no network to start. */
#define XENOLITH_STATUS_NO_NETWORK 0xC00000BEu

/* What looking something up reports where there is nothing to look in. */
#define XENOLITH_STATUS_NOT_FOUND 0xC0000225u

/* Guest memory is handed out a grain at a time, so a size becomes a whole
 * number of grains, and a size of nothing still takes one. */
static uint32_t whole_grains(uint32_t size) {
    uint32_t grains = (size + XENOLITH_HEAP_GRAIN - 1) / XENOLITH_HEAP_GRAIN;
    return grains == 0 ? XENOLITH_HEAP_GRAIN : grains * XENOLITH_HEAP_GRAIN;
}

/* Where guest memory is handed out from.
 *
 * One arena serves both the virtual and the physical allocator. The console
 * distinguishes the two and this does not: every guest address is mapped and
 * zeroed before a title starts, and none of it is nearer the graphics hardware
 * than any other.
 *
 * Nothing is ever given back. A title that allocated and freed in a loop would
 * exhaust this where the console would not, which has not happened and is
 * written down here rather than discovered later.
 */
static uint32_t handed_out = XENOLITH_HEAP_BASE;
static pthread_mutex_t handing_out = PTHREAD_MUTEX_INITIALIZER;

static uint32_t take_guest_memory(uint32_t size, uint32_t alignment) {
    if (alignment < XENOLITH_HEAP_GRAIN) {
        alignment = XENOLITH_HEAP_GRAIN;
    }

    uint32_t rounded = whole_grains(size);

    pthread_mutex_lock(&handing_out);
    uint32_t at = (handed_out + alignment - 1) & ~(alignment - 1);
    if (at < handed_out || rounded > XENOLITH_HEAP_END - at) {
        pthread_mutex_unlock(&handing_out);
        return 0;
    }
    handed_out = at + rounded;
    pthread_mutex_unlock(&handing_out);
    return at;
}

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
    uint32_t address_at = (uint32_t)ctx->r[3];
    uint32_t size_at = (uint32_t)ctx->r[4];
    uint32_t wanted = address_at ? xenolith_load32(base, address_at) : 0;
    uint32_t size = size_at ? xenolith_load32(base, size_at) : 0;

    uint32_t at = wanted & ~(XENOLITH_HEAP_GRAIN - 1);
    if (at == 0) {
        at = take_guest_memory(size, XENOLITH_HEAP_GRAIN);
        if (at == 0) {
            ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
            return;
        }
    }

    if (address_at) {
        xenolith_store32(base, address_at, at);
    }
    if (size_at) {
        xenolith_store32(base, size_at, whole_grains(size));
    }
    ctx->r[3] = XENOLITH_STATUS_SUCCESS;
}

/* Memory the graphics hardware can reach.
 *
 * This cannot share the shape above and that is the whole point of it being
 * separate: it takes its size by value rather than through a pointer, and it
 * answers with the address itself rather than a status. Read the other way it
 * takes a flag word for a pointer, writes the answer over whatever that
 * addresses, and hands the title nothing, which is what it did.
 *
 * The range a title asks the memory to fall between is not honoured. Every
 * guest address is mapped here and none is nearer the hardware than any other,
 * so there is no range for this to choose among.
 */
static void allocate_physical_memory(xenolith_context *ctx) {
    ctx->r[3] = take_guest_memory((uint32_t)ctx->r[4], (uint32_t)ctx->r[8]);
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

/* How fast the console's timebase counts.
 *
 * Assumed. The rate is a property of the hardware and is written down nowhere
 * this can read. What a title does with it is divide by it, so a wrong rate
 * makes everything timed run at the wrong speed rather than not run.
 */
#define XENOLITH_TIMEBASE_HZ 50000000u

static void query_performance_frequency(xenolith_context *ctx) {
    assumed("the timebase counts at fifty million a second",
            "the rate is a property of the hardware and is written down nowhere here");
    ctx->r[3] = XENOLITH_TIMEBASE_HZ;
}

/* Objects a title makes, reaches through a handle, and waits on.
 *
 * The console hands back a handle and takes it again later, so what a handle
 * means has to live here rather than in guest memory. A handle is an index into
 * this table with one added, so that nothing is ever handle zero.
 */
#define XENOLITH_OBJECTS 4096

enum object_kind {
    OBJECT_NONE = 0,
    OBJECT_EVENT,
    OBJECT_MUTANT,
    OBJECT_SEMAPHORE,
    OBJECT_THREAD,
};

struct object {
    enum object_kind kind;
    pthread_mutex_t lock;
    pthread_cond_t changed;
    int signalled;
    int manual;
    long count;
    long limit;
    pthread_t thread;
    int running;
    int suspended;
    /* What a thread was told to run, kept until it is resumed. */
    uint32_t startup;
    uint32_t entry;
    uint32_t argument;
    uint8_t *base;
};

static struct object objects[XENOLITH_OBJECTS];
static pthread_mutex_t objects_lock = PTHREAD_MUTEX_INITIALIZER;

/* Returns a fresh handle, or zero when there are no more. */
static uint32_t take_object(enum object_kind kind) {
    pthread_mutex_lock(&objects_lock);
    for (uint32_t at = 0; at < XENOLITH_OBJECTS; at++) {
        if (objects[at].kind == OBJECT_NONE) {
            struct object *made = &objects[at];
            memset(made, 0, sizeof *made);
            made->kind = kind;
            pthread_mutex_init(&made->lock, NULL);
            pthread_cond_init(&made->changed, NULL);
            pthread_mutex_unlock(&objects_lock);
            return at + 1;
        }
    }
    pthread_mutex_unlock(&objects_lock);
    return 0;
}

/* Returns what a handle names, or nothing when it names nothing. */
static struct object *object_of(uint32_t handle) {
    if (handle == 0 || handle > XENOLITH_OBJECTS) {
        return NULL;
    }
    struct object *found = &objects[handle - 1];
    return found->kind == OBJECT_NONE ? NULL : found;
}

/* Waits until an object is signalled, taking it if taking is what waiting on
 * that kind means. */
static uint32_t wait_on(struct object *waited) {
    pthread_mutex_lock(&waited->lock);
    while (!waited->signalled) {
        pthread_cond_wait(&waited->changed, &waited->lock);
    }
    if (!waited->manual) {
        waited->signalled = 0;
    }
    pthread_mutex_unlock(&waited->lock);
    return XENOLITH_STATUS_SUCCESS;
}

/* Sets an object signalled and wakes whoever was waiting. */
static void signal_object(struct object *woken, int state) {
    pthread_mutex_lock(&woken->lock);
    woken->signalled = state;
    pthread_cond_broadcast(&woken->changed);
    pthread_mutex_unlock(&woken->lock);
}

/* Where each thread's stack is taken from, below the one the first thread was
 * given so that none of them meet. */
#define XENOLITH_THREAD_STACK 0x00100000u

static uint32_t thread_stack_next = XENOLITH_STACK_TOP - XENOLITH_THREAD_STACK;
static pthread_mutex_t thread_stack_lock = PTHREAD_MUTEX_INITIALIZER;

static uint32_t take_thread_stack(void) {
    pthread_mutex_lock(&thread_stack_lock);
    thread_stack_next -= XENOLITH_THREAD_STACK;
    uint32_t top = thread_stack_next;
    pthread_mutex_unlock(&thread_stack_lock);
    return top;
}

/* What a guest thread runs.
 *
 * The console gives a thread a shim to start in, which takes what to run and
 * what to pass it. Where a title named one it is entered with those two, and
 * where it did not the thread starts at what was named directly.
 */
static void *run_guest_thread(void *given) {
    struct object *self = given;
    static _Thread_local xenolith_context state;

    state.r[1] = take_thread_stack() - 64;
    if (self->startup != 0) {
        state.r[3] = self->entry;
        state.r[4] = self->argument;
    } else {
        state.r[3] = self->argument;
    }

    /* A thread whose start was never lifted would otherwise do nothing and
     * exit, which looks exactly like a thread that ran and finished. Every
     * thread a title starts goes through a shim it names at runtime, so that
     * one address decides whether any guest thread runs at all, and silence
     * about it is worse than stopping. */
    uint32_t at = self->startup != 0 ? self->startup : self->entry;
    xenolith_function body = xenolith_lookup(at);
    if (body == NULL) {
        wanted_but_not_lifted(at, self->entry, "started a thread at");
    } else {
        body(&state, self->base);
    }

    self->manual = 1;
    signal_object(self, 1);
    return NULL;
}

/* Locks a title takes, kept beside the guest address each one lives at.
 *
 * A critical section is a structure in guest memory and a real lock cannot be
 * put inside one, so the address is what identifies it here.
 */
#define XENOLITH_SECTIONS 1024

static struct {
    uint32_t at;
    pthread_mutex_t lock;
} sections[XENOLITH_SECTIONS];
static pthread_mutex_t sections_lock = PTHREAD_MUTEX_INITIALIZER;

static pthread_mutex_t *section_of(uint32_t at) {
    pthread_mutex_lock(&sections_lock);
    for (unsigned k = 0; k < XENOLITH_SECTIONS; k++) {
        unsigned slot = (at / 4 + k) % XENOLITH_SECTIONS;
        if (sections[slot].at == at) {
            pthread_mutex_unlock(&sections_lock);
            return &sections[slot].lock;
        }
        if (sections[slot].at == 0) {
            sections[slot].at = at;
            pthread_mutexattr_t how;
            pthread_mutexattr_init(&how);
            pthread_mutexattr_settype(&how, PTHREAD_MUTEX_RECURSIVE);
            pthread_mutex_init(&sections[slot].lock, &how);
            pthread_mutexattr_destroy(&how);
            pthread_mutex_unlock(&sections_lock);
            return &sections[slot].lock;
        }
    }
    pthread_mutex_unlock(&sections_lock);
    return NULL;
}

/* Objects a title names by where they live rather than by a handle.
 *
 * The kernel has two ways of naming the same kind of thing. One hands out a
 * handle and takes it back, and those are the objects above. The other takes
 * the address of a structure the title keeps inside its own memory, which this
 * runtime never allocated and has no say in the layout of. So the address is
 * what identifies it here, the same way a critical section is identified by the
 * address it lives at.
 *
 * What such an object was initialized as is not read out of that structure. Its
 * layout is a property of the kernel rather than of the title, and reading
 * fields out of it by guesswork would put invented state behind every wait. One
 * is made on first mention instead, which is stated where it matters.
 */
#define XENOLITH_NAMED_OBJECTS 1024

static struct {
    uint32_t at;
    uint32_t handle;
} named[XENOLITH_NAMED_OBJECTS];
static pthread_mutex_t named_lock = PTHREAD_MUTEX_INITIALIZER;

static struct object *object_at(uint32_t at, enum object_kind kind) {
    if (at == 0) {
        return NULL;
    }

    pthread_mutex_lock(&named_lock);
    for (unsigned k = 0; k < XENOLITH_NAMED_OBJECTS; k++) {
        unsigned slot = (at / 4 + k) % XENOLITH_NAMED_OBJECTS;
        if (named[slot].at == at) {
            struct object *found = object_of(named[slot].handle);
            pthread_mutex_unlock(&named_lock);
            return found;
        }
        if (named[slot].at == 0) {
            named[slot].at = at;
            named[slot].handle = take_object(kind);
            struct object *found = object_of(named[slot].handle);
            pthread_mutex_unlock(&named_lock);
            return found;
        }
    }
    pthread_mutex_unlock(&named_lock);
    return NULL;
}

/* Creating a thread, which a title does suspended and resumes after it has said
 * how the thread should be scheduled. What it is told about scheduling is not
 * acted on: a host decides that, and saying otherwise would be a pretence.
 */
static void create_thread(xenolith_context *ctx, uint8_t *base) {
    uint32_t handle_at = (uint32_t)ctx->r[3];
    uint32_t id_at = (uint32_t)ctx->r[5];
    uint32_t handle = take_object(OBJECT_THREAD);
    struct object *made = object_of(handle);
    if (made == NULL) {
        ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
        return;
    }

    made->startup = (uint32_t)ctx->r[6];
    made->entry = (uint32_t)ctx->r[7];
    made->argument = (uint32_t)ctx->r[8];
    made->base = base;
    made->suspended = ((uint32_t)ctx->r[9] & 1) != 0;

    if (handle_at != 0) {
        xenolith_store32(base, handle_at, handle);
    }
    if (id_at != 0) {
        xenolith_store32(base, id_at, handle);
    }
    if (!made->suspended) {
        made->running = 1;
        pthread_create(&made->thread, NULL, run_guest_thread, made);
    }
    ctx->r[3] = XENOLITH_STATUS_SUCCESS;
}

/* Letting a suspended thread go. */
static void resume_thread(xenolith_context *ctx) {
    struct object *found = object_of((uint32_t)ctx->r[3]);
    if (found != NULL && found->kind == OBJECT_THREAD && !found->running) {
        found->running = 1;
        found->suspended = 0;
        pthread_create(&found->thread, NULL, run_guest_thread, found);
    }
    ctx->r[3] = XENOLITH_STATUS_SUCCESS;
}

/* Making something to wait on. The kind decides what waiting takes. */
static void create_waitable(xenolith_context *ctx, uint8_t *base, enum object_kind kind) {
    uint32_t handle_at = (uint32_t)ctx->r[3];
    uint32_t handle = take_object(kind);
    struct object *made = object_of(handle);
    if (made == NULL) {
        ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
        return;
    }

    if (kind == OBJECT_EVENT) {
        /* The kind is which of the two an event is: one that stays signalled
         * until it is cleared, or one that lets a single waiter through. */
        made->manual = (uint32_t)ctx->r[5] == 0;
        made->signalled = (uint32_t)ctx->r[6] != 0;
    } else if (kind == OBJECT_MUTANT) {
        made->manual = 0;
        made->signalled = (uint32_t)ctx->r[5] == 0;
    } else {
        made->manual = 0;
        made->count = (long)(uint32_t)ctx->r[5];
        made->limit = (long)(uint32_t)ctx->r[6];
        made->signalled = made->count > 0;
    }

    if (handle_at != 0) {
        xenolith_store32(base, handle_at, handle);
    }
    ctx->r[3] = XENOLITH_STATUS_SUCCESS;
}

/* Turning a count of time into the parts of a date.
 *
 * The console counts hundreds of nanoseconds from the start of 1601, and this
 * is the documented arithmetic for taking that apart: the days are separated
 * from the time of day, the day is walked forward through four hundred year
 * cycles, and what is left is the month and the day of it.
 *
 * Nothing here is assumed. A calendar is a definition rather than a property of
 * the hardware.
 */
static void time_to_fields(xenolith_context *ctx, uint8_t *base) {
    static const uint16_t before[2][13] = {
        {0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334, 365},
        {0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335, 366},
    };

    uint32_t from = (uint32_t)ctx->r[3];
    uint32_t into = (uint32_t)ctx->r[4];
    if (from == 0 || into == 0) {
        return;
    }

    uint64_t ticks = xenolith_load64(base, from);
    uint64_t milliseconds = ticks / 10000u;
    uint64_t days = milliseconds / 86400000u;
    uint32_t rest = (uint32_t)(milliseconds % 86400000u);

    /* The first of January 1601 was a Monday. */
    uint32_t weekday = (uint32_t)((days + 1) % 7);

    uint32_t year = 1601;
    for (;;) {
        int leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        uint32_t length = leap ? 366u : 365u;
        if (days < length) {
            break;
        }
        days -= length;
        year++;
    }

    int leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    uint32_t month = 1;
    while (month < 12 && days >= before[leap][month]) {
        month++;
    }
    uint32_t day = (uint32_t)days - before[leap][month - 1] + 1;

    const uint16_t fields[] = {
        (uint16_t)year,
        (uint16_t)month,
        (uint16_t)day,
        (uint16_t)(rest / 3600000u),
        (uint16_t)(rest / 60000u % 60u),
        (uint16_t)(rest / 1000u % 60u),
        (uint16_t)(rest % 1000u),
        (uint16_t)weekday,
    };
    for (uint32_t at = 0; at < sizeof fields / sizeof *fields; at++) {
        xenolith_store16(base, into + at * 2, fields[at]);
    }
}

/* What the console reports itself as.
 *
 * The title says what it wants: it compares what it is told against a number
 * written into its own code and goes elsewhere when it is smaller. So this is
 * read out of the title rather than assumed, and is the least it will accept.
 */
#define XENOLITH_SYSTEM_VERSION 0x200a3200u

/* What a title told the graphics hardware, kept because it is the only record
 * of it.
 *
 * Nothing here draws. A title writes drawing commands into a ring buffer and
 * reads back how far the hardware has got through them, and there is no
 * hardware. What is kept is where that buffer is, how big, and where the title
 * reads the progress from, which is what something consuming those commands
 * would need and costs nothing to hold.
 */
static struct {
    uint32_t ring_at;
    uint32_t ring_size;
    uint32_t read_back_at;
    uint32_t callback;
    uint32_t callback_argument;
    /* How many times a title has asked for what it drew to be shown. */
    unsigned long frames;
} graphics;

/* Where a title says how far it has written into the ring buffer.
 *
 * The console maps the graphics hardware's registers into the address space, and
 * a title publishes the ring buffer's write pointer by storing to one of them,
 * fenced on both sides so the hardware sees the commands before the pointer.
 * Nothing here is hardware, so that store lands in ordinary memory and the value
 * can be read back out of it.
 */
#define XENOLITH_RING_WRITE_POINTER 0x7fc80714u

/* How far back the block a title asks for progress in begins.
 *
 * A title names where the read pointer should be written and, beside it, how
 * big a block that sits in. The size arrives as a power of two and comes out at
 * sixty four bytes, and the address named is sixty bytes into it, so the block
 * starts sixty bytes earlier. The word at the front of it is a count of how far
 * the hardware has worked through what it was given: a title sets it behind
 * where it wants to get to and waits for it to catch up.
 *
 * Both numbers are read out of what a title passes rather than assumed.
 */
#define XENOLITH_PROGRESS_BEHIND 60u

/* Where a title reads whether there is finished work to account for.
 *
 * Another of the same block of registers. A title checks the lowest bit of it
 * before it will retire anything, so leaving it clear means a title is told a
 * frame finished and declines to do anything about it.
 */
#define XENOLITH_GRAPHICS_COMPLETED 0x7fc86544u

/* Which interrupt a title is being told about.
 *
 * Both numbers are read out of the title, which compares against each and does
 * something different for both. One retires the work it had in flight. The
 * other runs what it left to be run when its commands were reached.
 */
#define XENOLITH_INTERRUPT_RETIRE 0u
#define XENOLITH_INTERRUPT_REACHED 1u

/* Reports the hardware as having consumed whatever the title has written.
 *
 * A title writes commands into the ring buffer and then waits for the hardware
 * to work through them, watching the place it asked for progress to be reported
 * to. Nothing here consumes those commands. A truthful report is that none were
 * consumed, and a title given that waits forever on hardware that is not there.
 *
 * So the title's own write pointer is copied to the place it reads progress
 * from, which says the commands were consumed the moment they were written.
 * They were consumed by nothing. That is the assumption, and it is what lets a
 * title past its own waiting rather than any claim that something was drawn.
 *
 * It runs on its own thread because nothing calls back into the runtime while a
 * title spins, so there is no other moment to do it in.
 */
/* Tells a title the display finished a frame.
 *
 * The hardware interrupts when it reaches the end of a frame, and the kernel
 * calls what the title registered for it. A title uses that to retire the work
 * it had in flight, and until it happens the count of outstanding work never
 * falls and the title will not queue more.
 *
 * Nothing here interrupts, so this is the runtime saying it instead. Which is a
 * claim about timing rather than about anything drawn: the frame it reports is
 * one where nothing was rendered.
 */
static void raise_the_graphics_interrupt(uint8_t *base, uint32_t source) {
    static _Thread_local xenolith_context state;
    static _Thread_local uint32_t stack;

    if (graphics.callback == 0) {
        return;
    }

    xenolith_function body = xenolith_lookup(graphics.callback);
    if (body == NULL) {
        wanted_but_not_lifted(graphics.callback, 0, "answered an interrupt at");
        return;
    }

    if (stack == 0) {
        stack = take_thread_stack();
    }
    state.r[1] = stack - 64;
    /* Which interrupt this is, and what the title asked to be handed back.
     *
     * The number is not a placeholder and there is more than one of them. The
     * handler compares what it is given and does something different for each,
     * returning without doing anything at all for the rest. One of them runs
     * what a title left to be run when its commands were reached. Another
     * retires the work it has in flight, and until that happens the count of
     * outstanding work never falls and a title queues nothing further. Both
     * have to be raised, and which is which is read out of the title. */
    state.r[3] = source;
    state.r[4] = graphics.callback_argument;
    body(&state, base);
}

/* The one packet whose effect a title can see.
 *
 * Everything else in the buffer sets hardware state or draws, and there is no
 * hardware here for either. This one carries an address and a value, and the
 * value appearing at the address is how a title learns the work it queued was
 * reached. Nothing it queues afterwards happens until that shows up.
 */
#define XENOLITH_PACKET_WRITE 0x3fu

/* How far through the ring buffer this has read, in words. */
static uint32_t consumed;

/* Reads the packets a title wrote and does the one thing it can see.
 *
 * What a header means is read out of what a title actually wrote rather than
 * taken from a description of the hardware. The top two bits say which of four
 * kinds a packet is and the next fourteen say how many words follow, which is
 * checkable against the buffer: the lengths those two fields give land exactly
 * on the next header, four packets running, with nothing left over.
 *
 * The packets whose effect is not visible from outside the hardware are read
 * for their length and otherwise passed over. That is not the same as executing
 * them, and a frame reported finished here still had nothing drawn in it.
 */
static void consume_the_ring(uint8_t *base, uint32_t written) {
    if (graphics.ring_at == 0) {
        return;
    }

    /* Going backwards is the buffer having wrapped, and everything from where
     * this had got to up to the end was read before it did.
     *
     * Wrapping is worked out this way rather than from the buffer's length,
     * because the length is given as a power of two whose unit is not
     * recoverable from what a title passes. Reading it wrongly would put the
     * walk half way through a packet, which is worse than not knowing it. This
     * needs only the pointer, which is unambiguous. */
    if (written < consumed) {
        consumed = 0;
    }

    while (consumed < written) {
        uint32_t header = xenolith_load32(base, graphics.ring_at + consumed * 4);
        uint32_t follows;

        switch (header >> 30) {
        case 2:
            /* A filler word, carrying nothing. */
            follows = 0;
            break;
        case 1:
            follows = 2;
            break;
        default:
            follows = ((header >> 16) & 0x3fff) + 1;
            break;
        }

        if (consumed + 1 + follows > written) {
            /* The rest of this packet has not been written yet. */
            break;
        }

        if ((header >> 30) == 3 && ((header >> 8) & 0xff) == XENOLITH_PACKET_WRITE
            && follows >= 2) {
            uint32_t where = xenolith_load32(base, graphics.ring_at + (consumed + 1) * 4);
            uint32_t value = xenolith_load32(base, graphics.ring_at + (consumed + 2) * 4);
            xenolith_store32(base, where, value);
        }

        consumed += 1 + follows;
    }
}

static void *drain_the_ring(void *given) {
    uint8_t *base = given;
    unsigned ticks = 0;

    assumed("the graphics hardware keeps up with the title",
            "nothing here reads the commands a title writes, so the only "
            "alternative is a title waiting forever");

    while (graphics.read_back_at != 0) {
        uint32_t written = xenolith_load32(base, XENOLITH_RING_WRITE_POINTER);
        consume_the_ring(base, written);
        xenolith_store32(base, graphics.read_back_at, consumed);

        /* And how far it has worked through what it was given, which is all of
         * it. A title sets this behind where it wants to get to and waits, so
         * leaving it still is the same as saying the hardware stopped. */
        if (graphics.read_back_at > XENOLITH_PROGRESS_BEHIND) {
            uint32_t head = graphics.read_back_at - XENOLITH_PROGRESS_BEHIND;
            xenolith_store32(base, head, xenolith_load32(base, head) + 1);
        }

        /* A display finishes a frame sixty times a second. This one finishes
         * nothing, at the same rate.
         *
         * Before saying so, the bit a title checks before it will retire
         * anything is set. It reads that out of a graphics register, and what
         * it means is that there is finished work to account for. Everything
         * written here is finished the moment it is written, so the bit is
         * always the truth of this runtime even though it is never the truth of
         * any hardware. */
        if (++ticks % 160 == 0) {
            xenolith_store32(base, XENOLITH_GRAPHICS_COMPLETED, 1);
            raise_the_graphics_interrupt(base, XENOLITH_INTERRUPT_RETIRE);
            raise_the_graphics_interrupt(base, XENOLITH_INTERRUPT_REACHED);
        }

        struct timespec briefly = {0, 100000};
        nanosleep(&briefly, NULL);
    }
    return NULL;
}

/* What the display is, written into the structure a title hands over.
 *
 * Two separate things are needed here and only one of them was recoverable.
 *
 * The shape was. A title reads the structure straight after asking for it, and
 * what it reads says where the fields are: two words at the front taken as
 * whole numbers, and twelve bytes further on a value loaded into a floating
 * register, added to one half, and truncated. Rounding a float to the nearest
 * whole number is what a refresh rate gets done to it and not much else, and
 * the one half is in the title's own constant pool where it can be read off.
 * So the offsets below are derived from the artefact rather than copied.
 *
 * The values were not. What a console was set to show is a property of that
 * console and its television, and the container never held it. What is written
 * is a console showing 1280 by 720 at sixty, which is a state a real one can be
 * in and is the size this title asks for elsewhere, and it is said outright
 * rather than passed off as recovered.
 *
 * Only the fields the title was seen to read are written. The three words
 * between them are set to nothing, matching the console with nothing turned on
 * that the settings answer describes. Nothing past that is touched, since where
 * the structure ends is not derivable from a title that never reads that far.
 */
static void report_the_display(uint8_t *base, uint32_t at) {
    if (at == 0) {
        return;
    }

    assumed("a display of 1280 by 720 at sixty",
            "what a console was set to show is not a property of the title and "
            "is not in the container");

    uint32_t rate;
    float sixty = 60.0f;
    memcpy(&rate, &sixty, sizeof rate);

    xenolith_store32(base, at, 1280);
    xenolith_store32(base, at + 4, 720);
    xenolith_store32(base, at + 8, 0);
    xenolith_store32(base, at + 12, 0);
    xenolith_store32(base, at + 16, 0);
    xenolith_store32(base, at + 20, rate);
}

/* What the runtime answers to, by ordinal within a library.
 *
 * Each entry is here because a title reached it. Nothing is implemented ahead
 * of a title asking for it, so this list is a record of what was needed rather
 * than a guess at what might be.
 */
static int serviced(xenolith_context *ctx, uint8_t *base, const char *library,
                    uint32_t ordinal) {
    if (strcmp(library, "xam.xex") == 0) {
        switch (ordinal) {
        case 1:
        case 51:
            /* Starting networking, of which there is none here. Saying it
             * started would send a title looking for one. */
            ctx->r[3] = XENOLITH_STATUS_NO_NETWORK;
            return 1;
        case 642:
            ctx->r[3] = XENOLITH_SYSTEM_VERSION;
            return 1;
        case 977:
            report_the_display(base, (uint32_t)ctx->r[3]);
            return 1;
        default:
            return 0;
        }
    }

    if (strcmp(library, "xboxkrnl.exe") != 0) {
        return 0;
    }
    switch (ordinal) {
    case 102:
        current_process_type(ctx);
        return 1;
    case 13:
        create_thread(ctx, base);
        return 1;
    case 129:
    case 151:
    case 153:
        /* How a thread should be scheduled, which a host decides. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 131:
        query_performance_frequency(ctx);
        return 1;
    case 132: {
        /* What the clock says, in the hundred nanosecond ticks the console
         * counts. Where it counts from is not modelled, so this counts from
         * when the program started. */
        uint32_t at = (uint32_t)ctx->r[3];
        if (at != 0) {
            xenolith_store64(base, at, xenolith_timebase() * 100);
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 186:
        allocate_physical_memory(ctx);
        return 1;
    case 206:
    case 246: {
        struct object *found = object_of((uint32_t)ctx->r[3]);
        if (found != NULL) {
            signal_object(found, ordinal == 246);
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 209:
        create_waitable(ctx, base, OBJECT_EVENT);
        return 1;
    case 212:
        create_waitable(ctx, base, OBJECT_MUTANT);
        return 1;
    case 213:
        create_waitable(ctx, base, OBJECT_SEMAPHORE);
        return 1;
    case 242: {
        struct object *found = object_of((uint32_t)ctx->r[3]);
        if (found != NULL) {
            signal_object(found, 1);
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 243: {
        /* Letting a semaphore go, which lets a thread waiting on it run. What
         * the count was beforehand is written back where a title asked for it,
         * and is the count this runtime keeps rather than one it invented. */
        struct object *found = object_of((uint32_t)ctx->r[3]);
        uint32_t before_at = (uint32_t)ctx->r[5];
        if (found != NULL) {
            if (before_at != 0) {
                xenolith_store32(base, before_at, (uint32_t)found->count);
            }
            found->count += (long)(int32_t)ctx->r[4];
            signal_object(found, 1);
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 245:
        resume_thread(ctx);
        return 1;
    case 251: {
        /* Releasing one object and waiting on another, which a title uses to
         * hand work to a thread and wait for the answer without missing the
         * reply. The console does the two with nothing in between. Here they
         * are two steps, which differs only for a title that depends on no
         * other thread being let go between them. */
        struct object *released = object_of((uint32_t)ctx->r[3]);
        struct object *waited = object_of((uint32_t)ctx->r[4]);
        if (released != NULL) {
            signal_object(released, 1);
        }
        ctx->r[3] = waited == NULL ? XENOLITH_STATUS_SUCCESS : wait_on(waited);
        return 1;
    }
    case 253: {
        struct object *found = object_of((uint32_t)ctx->r[3]);
        ctx->r[3] = found == NULL ? XENOLITH_STATUS_SUCCESS : wait_on(found);
        return 1;
    }
    case 207:
    case 261:
        /* Letting go of a handle, and dropping a reference. Nothing here counts
         * references or takes a handle back. A thread of this runtime's own may
         * still be inside an object a title has finished with, and reclaiming
         * the slot would hand it to something else while that is true. What it
         * costs is that handles are only ever handed out, which has not run out
         * and is written down rather than found later. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 272: {
        /* Turning a handle into something to hold. A handle is what this
         * runtime hands out, so it is also what it hands back. */
        uint32_t out = (uint32_t)ctx->r[5];
        if (out != 0) {
            xenolith_store32(base, out, (uint32_t)ctx->r[3]);
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 198: {
        /* How much memory there is and how much is left.
         *
         * The title hands over a structure with its own length at the front and
         * reads three fields out of it, two of which it shifts up by twelve, so
         * those two are counts of pages. What they are counts of is answered
         * from this runtime's own arena, which is the only memory there is
         * here, so the numbers are true of it whatever the console would have
         * reported. Which of the two is the whole and which the remainder is
         * not recoverable from what the title does with them, and both are
         * quantities of the same arena either way. */
        uint32_t into = (uint32_t)ctx->r[3];
        if (into != 0) {
            uint32_t page = 0x1000u;
            uint32_t whole = (XENOLITH_HEAP_END - XENOLITH_HEAP_BASE) / page;
            pthread_mutex_lock(&handing_out);
            uint32_t left = (XENOLITH_HEAP_END - handed_out) / page;
            pthread_mutex_unlock(&handing_out);

            for (uint32_t at = 4; at < 104; at += 4) {
                xenolith_store32(base, into + at, 0);
            }
            xenolith_store32(base, into + 4, whole);
            xenolith_store32(base, into + 12, left);
            xenolith_store32(base, into + 20, left);
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 294: {
        /* Filling memory with a whole word at a time. The length is in bytes
         * and the console rounds it down to a whole number of words, since a
         * word is what it writes. */
        uint32_t at = (uint32_t)ctx->r[3];
        uint32_t bytes = (uint32_t)ctx->r[4];
        uint32_t value = (uint32_t)ctx->r[5];
        for (uint32_t done = 0; done + 4 <= bytes; done += 4) {
            xenolith_store32(base, at + done, value);
        }
        ctx->r[3] = at;
        return 1;
    }
    case 300: {
        /* Describing a run of characters by where it starts and how long it is.
         * The shape is documented and exact: how many characters, how many
         * there is room for counting the ending nothing, and where they are. */
        uint32_t into = (uint32_t)ctx->r[3];
        uint32_t text = (uint32_t)ctx->r[4];
        if (into == 0) {
            return 1;
        }

        uint16_t length = 0;
        if (text != 0) {
            while (length < 0xfffeu && xenolith_load8(base, text + length) != 0) {
                length++;
            }
        }
        xenolith_store16(base, into, length);
        xenolith_store16(base, into + 2, text == 0 ? 0 : (uint16_t)(length + 1));
        xenolith_store32(base, into + 4, text);
        return 1;
    }
    case 303:
        initialize_critical_section(ctx, base);
        return 1;
    case 405:
    case 407:
        /* Asking for a module by name, and then for something inside it by
         * ordinal. Nothing is loaded here but the title, so there is no module
         * to hand back and no address inside one to find. The title reads a
         * failure and takes the path it has for not finding one. */
        ctx->r[3] = XENOLITH_STATUS_NOT_FOUND;
        return 1;
    case 473:
        /* Telling the graphics hardware where to find something. There is no
         * graphics hardware, so there is nothing to tell. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 16: {
        /* A setting of the console the title is running on, which is not in
         * the container because it was never a property of the title. The
         * console it was recorded from is gone and what it was set to went
         * with it.
         *
         * A console always has these, so reporting that the setting is missing
         * describes something that never happens and sends a title down a path
         * it has never taken. What is answered instead is a console with
         * nothing turned on, which is a state a real one can be in, and it is
         * said outright rather than passed off as recovered. */
        assumed("a console with no settings turned on",
                "what one was set to is not a property of the title and is not "
                "in the container");
        uint32_t into = (uint32_t)ctx->r[5];
        uint32_t room = (uint32_t)ctx->r[6];
        uint32_t needed_at = (uint32_t)ctx->r[7];
        if (into != 0 && room >= 4) {
            xenolith_store32(base, into, 0);
        }
        if (needed_at != 0) {
            xenolith_store32(base, needed_at, 4);
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 189:
        /* Giving memory back. Nothing here takes it back, which is written
         * down where the memory is handed out. Reporting a failure would be a
         * different lie and a louder one. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 199:
        /* Changing what a page allows. Every guest page is readable and
         * writable from the moment the space is mapped, and this runtime has no
         * way to make one less than that which a title would benefit from. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 190:
        /* Where something is to the hardware rather than to the title. There
         * is one address space here and everything in it is mapped, so a thing
         * is at the address it is already at. */
        return 1;
    case 21:
        /* Asking to be told when the title is being torn down. Nothing tears
         * one down here, so there is nothing to tell it. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 479:
        /* A routine the kernel names so that a title has something to point at
         * where it wants nothing done. Doing nothing is the whole of it. */
        return 1;
    case 3:
        /* Saying something to a debugger. A retail console with none attached
         * does nothing observable with this, and neither does this. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 177: {
        /* Taking a lock and raising the interrupt level, which is one call on
         * the console because the two go together there. The level is not
         * modelled: nothing here interrupts a thread, so there is no level to
         * be at. What a title is handed back is the level it should return to,
         * and it hands that straight back to the release below without reading
         * it, so nothing is lost by it being nothing. */
        pthread_mutex_t *lock = section_of((uint32_t)ctx->r[3]);
        if (lock != NULL) {
            pthread_mutex_lock(lock);
        }
        ctx->r[3] = 0;
        return 1;
    }
    case 180: {
        pthread_mutex_t *lock = section_of((uint32_t)ctx->r[3]);
        if (lock != NULL) {
            pthread_mutex_unlock(lock);
        }
        return 1;
    }
    case 77: {
        /* Taking a lock the console holds with interrupts already off. There
         * are no interrupts here and the threads are real, so what it protects
         * against is another thread, and a lock is what does that. */
        pthread_mutex_t *lock = section_of((uint32_t)ctx->r[3]);
        if (lock != NULL) {
            pthread_mutex_lock(lock);
        }
        return 1;
    }
    case 137: {
        pthread_mutex_t *lock = section_of((uint32_t)ctx->r[3]);
        if (lock != NULL) {
            pthread_mutex_unlock(lock);
        }
        return 1;
    }
    case 95:
    case 125:
        /* Holding off the kernel's own interruptions around a stretch of work.
         * Nothing here interrupts a thread that way, so there is nothing to
         * hold off and nothing to let back in. */
        return 1;
    case 454:
        /* Whether the link to the display trained. There is no link, and a
         * title told it failed goes looking for a fault to report rather than
         * carrying on, so this reports the state a working console is in. */
        ctx->r[3] = 1;
        return 1;
    case 617:
    case 618:
        /* Retraining the memory the graphics hardware draws into. There is
         * none, so there is nothing to retrain. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 143: {
        struct object *found = object_at((uint32_t)ctx->r[3], OBJECT_EVENT);
        if (found != NULL) {
            signal_object(found, 0);
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 176: {
        /* Waiting on an object named by where it lives. */
        struct object *found = object_at((uint32_t)ctx->r[3], OBJECT_EVENT);
        ctx->r[3] = found == NULL ? XENOLITH_STATUS_SUCCESS : wait_on(found);
        return 1;
    }
    case 445: {
        /* Where the system's own command buffer is, and what names it.
         *
         * A title asks for one and is handed a place to write and something to
         * call it by. The place is memory from this runtime, since there is no
         * hardware to have reserved any. What names it is read straight back
         * out and passed along without being looked at, so what it is matters
         * less than that it is the same each time. */
        static uint32_t buffer;
        if (buffer == 0) {
            buffer = take_guest_memory(XENOLITH_HEAP_GRAIN, XENOLITH_HEAP_GRAIN);
        }
        uint32_t where = (uint32_t)ctx->r[3];
        uint32_t names = (uint32_t)ctx->r[4];
        if (where != 0) {
            xenolith_store32(base, where, buffer);
        }
        if (names != 0) {
            xenolith_store32(base, names, buffer);
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 603:
        /* Showing what was drawn.
         *
         * Nothing was drawn and there is no display to show it on, so what
         * this does is count that a title asked. A title reaching this has
         * finished a frame's worth of work, which is worth knowing even when
         * the frame is empty. */
        graphics.frames++;
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 442: {
        /* What the display is currently doing, in a structure whose shape is
         * not in the container. Only the part a title was seen to read is
         * written: one byte, five in, which it compares against one. Nothing
         * is claimed about the rest, and nothing past the word holding that
         * byte is touched. */
        uint32_t into = (uint32_t)ctx->r[3];
        if (into != 0) {
            xenolith_store32(base, into, 0);
            xenolith_store32(base, into + 4, 0);
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 467:
        /* Telling the display what to be. There is no display, and what a
         * title is told one is remains what the query above reports. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 453:
        /* Filling a buffer with the commands that scale a picture up to the
         * display. Nothing here scales anything and nothing reads what would
         * be written, so no commands are put there. What is reported is how
         * many words were written, which is none. */
        ctx->r[3] = 0;
        return 1;
    case 455:
        /* Keeping the last picture on screen across a change. There is no
         * screen and nothing was on it, so there is nothing to keep. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 450:
        /* Starting the command processor, of which there is none. */
        return 1;
    case 451:
        graphics.ring_at = (uint32_t)ctx->r[3];
        graphics.ring_size = 1u << ((uint32_t)ctx->r[4] & 0x1f);
        return 1;
    case 438: {
        graphics.read_back_at = (uint32_t)ctx->r[3];
        pthread_t draining;
        if (pthread_create(&draining, NULL, drain_the_ring, base) == 0) {
            pthread_detach(draining);
        }
        return 1;
    }
    case 469:
        graphics.callback = (uint32_t)ctx->r[3];
        graphics.callback_argument = (uint32_t)ctx->r[4];
        return 1;
    case 433:
        /* Telling whoever registered an interest that the hardware reached a
         * point. Nothing reaches one, so there is nothing to pass on. */
        return 1;
    case 457:
        /* What the display is doing beyond its size, which is a property of a
         * console and its television rather than of the title. Nothing is
         * claimed. */
        ctx->r[3] = 0;
        return 1;
    case 458:
        report_the_display(base, (uint32_t)ctx->r[3]);
        return 1;
    case 441: {
        /* What the display's gamma is set to, which nothing here reads back
         * and nothing here applies. */
        uint32_t which = (uint32_t)ctx->r[3];
        uint32_t value = (uint32_t)ctx->r[4];
        if (which != 0) {
            xenolith_store32(base, which, 0);
        }
        if (value != 0) {
            xenolith_store32(base, value, 0);
        }
        return 1;
    }
    case 320:
        time_to_fields(ctx, base);
        return 1;
    case 321: {
        pthread_mutex_t *lock = section_of((uint32_t)ctx->r[3]);
        ctx->r[3] = lock != NULL && pthread_mutex_trylock(lock) == 0;
        return 1;
    }
    case 204:
        allocate_virtual_memory(ctx, base);
        return 1;
    case 293: {
        pthread_mutex_t *lock = section_of((uint32_t)ctx->r[3]);
        if (lock != NULL) {
            pthread_mutex_lock(lock);
        }
        return 1;
    }
    case 304: {
        pthread_mutex_t *lock = section_of((uint32_t)ctx->r[3]);
        if (lock != NULL) {
            pthread_mutex_unlock(lock);
        }
        return 1;
    }
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

/* A reservation, now that more than one thread can take one.
 *
 * What each thread reserved is kept beside it, and redeeming one compares
 * memory against what was reserved under a lock and stores only where the two
 * still agree. That is a compare and swap rather than the reservation the
 * hardware has, and the difference shows where a value is written back to what
 * it already was: the hardware would notice and this does not.
 *
 * A title uses these to build its own locks, and for that the difference does
 * not arise, since what it writes back is never what it read.
 */
static pthread_mutex_t reservation_lock = PTHREAD_MUTEX_INITIALIZER;
static _Thread_local uint32_t reserved_at;
static _Thread_local uint64_t reserved_was;

uint32_t xenolith_reserve32(const uint8_t *base, uint32_t address) {
    uint32_t held = xenolith_load32(base, address);
    reserved_at = address;
    reserved_was = held;
    return held;
}

uint64_t xenolith_reserve64(const uint8_t *base, uint32_t address) {
    uint64_t held = xenolith_load64(base, address);
    reserved_at = address;
    reserved_was = held;
    return held;
}

uint8_t xenolith_conditional32(uint8_t *base, uint32_t address, uint32_t value) {
    uint8_t stored = 0;
    pthread_mutex_lock(&reservation_lock);
    if (reserved_at == address && xenolith_load32(base, address) == (uint32_t)reserved_was) {
        xenolith_store32(base, address, value);
        stored = 1;
    }
    reserved_at = 0;
    pthread_mutex_unlock(&reservation_lock);
    return stored;
}

uint8_t xenolith_conditional64(uint8_t *base, uint32_t address, uint64_t value) {
    uint8_t stored = 0;
    pthread_mutex_lock(&reservation_lock);
    if (reserved_at == address && xenolith_load64(base, address) == reserved_was) {
        xenolith_store64(base, address, value);
        stored = 1;
    }
    reserved_at = 0;
    pthread_mutex_unlock(&reservation_lock);
    return stored;
}

/* A counter that advances, which is all a timing loop needs to finish. What it
 * counts at bears no relation to the console. */
uint64_t xenolith_timebase(void) {
    static uint64_t ticks;
    return ++ticks;
}
