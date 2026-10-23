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

#include <dirent.h>
#include <dlfcn.h>
#include <errno.h>
#include <stdarg.h>
#include <stdio.h>
#include <pthread.h>
#include <signal.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>
#include <time.h>

/* How many calls a trace reports before giving up.
 *
 * A title told that every import returned zero goes wrong quickly, and once it
 * has, the calls it makes say more about the lie than about the title. */
static void read_file(xenolith_context *ctx, uint8_t *base);
static void write_file(xenolith_context *ctx, uint8_t *base);
static void query_file(xenolith_context *ctx, uint8_t *base);
static void set_file(xenolith_context *ctx, uint8_t *base);
static void query_directory(xenolith_context *ctx, uint8_t *base);
static void open_file(xenolith_context *ctx, uint8_t *base, int may_make);
static void remember_where_the_image_was(const char *image);

#define XENOLITH_TRACE_LIMIT 20000

/* The guest addresses a 32-bit pointer can form.
 *
 * Reserved rather than committed. Only the pages actually touched become
 * resident, so the cost of covering the whole space is address space rather
 * than memory.
 */
#define XENOLITH_GUEST_SPAN ((size_t)1 << 32)

/* A page past the end, so that reading a word that starts in the last few bytes
 * lands on something.
 *
 * Guest addresses are four bytes wide and a read works out where it is reading
 * before taking the bytes, so a word begun at the very top runs off the end
 * rather than round to the bottom. Only a title reading through a pointer that
 * is already nonsense gets there, and it reads nothing either way, but it should
 * not bring the whole run down when it does.
 */
#define XENOLITH_A_LITTLE_PAST_THE_END ((size_t)4096)

static void say_what_was_read_in_total(void);
static void say_what_was_read_and_stop(int which);
static void say_a_report(const char *format, ...);

/* Ends a run the runtime has decided cannot go on, with the code that says why.
 *
 * What was read is said first, the way any other end of a run says it. The
 * process then ends at once rather than through the exit handlers: those
 * include the graphics driver's own, which take the device apart while the
 * thread reading the title's commands may be in the middle of drawing with it,
 * and a run stopped for a reason it reported then crashes on the way out with a
 * code that says something else. */
static _Noreturn void stop_the_run(int code) {
    say_what_was_read_in_total();
    fflush(stdout);
    fflush(stderr);
    _exit(code);
}

uint8_t *xenolith_boot(const char *image, uint32_t load_address) {
    remember_where_the_image_was(image);

    /* What the run read, said however it ends.
     *
     * `atexit` covers a run that finishes and a run that stops on a gap in the
     * translation. It does not cover the one that matters most: a title spinning
     * in its own code is stopped by whatever started it, and a signal does not
     * run the exit handlers. So both are registered here, where every run goes
     * through, rather than from the thread that reads the command stream, which
     * a spinning title may never have started. */
    atexit(say_what_was_read_in_total);
    signal(SIGTERM, say_what_was_read_and_stop);
    signal(SIGINT, say_what_was_read_and_stop);
    void *base = mmap(NULL, XENOLITH_GUEST_SPAN + XENOLITH_A_LITTLE_PAST_THE_END,
                      PROT_READ | PROT_WRITE,
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
    stop_the_run(2);
}

/* A title asking for a debugger. There is none, so the run carries on, as a
 * console's does; each place that asks is said once, because what a title
 * printed just before asking is usually why. */
void xenolith_break(xenolith_context *ctx, uint8_t *base, uint32_t address, uint32_t why) {
    static uint32_t said[64];
    static unsigned count;
    static pthread_mutex_t saying = PTHREAD_MUTEX_INITIALIZER;

    (void)ctx;
    (void)base;
    pthread_mutex_lock(&saying);
    for (unsigned at = 0; at < count; at++) {
        if (said[at] == address) {
            pthread_mutex_unlock(&saying);
            return;
        }
    }
    if (count < sizeof said / sizeof *said) {
        said[count++] = address;
    }
    pthread_mutex_unlock(&saying);
    say_a_report("the title asked for a debugger at %#010x for reason %u, and with none "
                 "there it carries on",
                 address, why);
}

/* A trap leaves the function it was in, and there is nowhere for it to go. */
void xenolith_trap(xenolith_context *ctx, uint8_t *base, uint32_t address) {
    (void)ctx;
    (void)base;
    fprintf(stderr, "xenolith: trapped at %#010x\n", address);
    stop_the_run(2);
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
        /* What is at the address, before saying it is not a function.
         *
         * A call through a table that was never filled names nought, and a call
         * through one filled with something else names whatever that is. The
         * two are different faults and look the same from the address alone, so
         * the words there are said beside it. */
        fprintf(stderr, "xenolith: %#010x was called from %#010x and is not a lifted "
                        "function; it holds %08x %08x\n",
                address, from, xenolith_load32(base, address),
                xenolith_load32(base, address + 4u));
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

/* Why a socket could not be made, as the console's socket library says it. */
#define XENOLITH_NETWORK_IS_DOWN 10050u

/* What looking something up reports where there is nothing to look in. */
#define XENOLITH_STATUS_NOT_FOUND 0xC0000225u

/* What a wait reports when the length of time it was given ran out. */
#define XENOLITH_STATUS_TIMEOUT 0x00000102u

/* What opening a file that is not there reports. */
#define XENOLITH_STATUS_NO_SUCH_FILE 0xC0000034u

/* What reading past the end of a file reports. */
#define XENOLITH_STATUS_END_OF_FILE 0xC0000011u

/* The packet the kernel writes to show a frame, how many words it covers with
 * its header, and the four letters it starts with. */
#define XENOLITH_PACKET_SWAP 0x64u
#define XENOLITH_SWAP_WORDS 64u
#define XENOLITH_SWAP_TAG 0x53574150u

/* The kernel's flag for a page that can be read and written. */
#define XENOLITH_PAGE_READ_WRITE 0x04u

/* How large a context for the sound decoder is: sixteen words. */
#define XENOLITH_SOUND_CONTEXT 64u

/* What a call given an argument it cannot use reports. */
#define XENOLITH_STATUS_INVALID_PARAMETER 0xC000000Du

/* What asking about a file for something this does not answer reports. */
#define XENOLITH_STATUS_INVALID_INFO_CLASS 0xC0000003u

/* The kinds of thing a title asks about a file, by the kernel's numbers. */
#define XENOLITH_FILE_STANDARD 5u
#define XENOLITH_FILE_POSITION 14u
#define XENOLITH_FILE_NETWORK_OPEN 34u

/* What a file is, as the last word of what is said about one: an ordinary file
 * with nothing special about it. */
#define XENOLITH_FILE_ATTRIBUTE_NORMAL 0x80u

/* The place a title names when it means the end of a file rather than a number
 * of bytes into it. The number is the console's. */
#define XENOLITH_AT_THE_END 0xfffffffffffffffeull

/* What a title says to do with a file that is or is not already there, and what
 * it is told was done. Both numbers are the console's. */
#define XENOLITH_REPLACE_IT 0u
#define XENOLITH_OPEN_IT 1u
#define XENOLITH_MAKE_IT 2u
#define XENOLITH_OPEN_OR_MAKE_IT 3u
#define XENOLITH_EMPTY_IT 4u
#define XENOLITH_EMPTY_OR_MAKE_IT 5u

#define XENOLITH_WAS_REPLACED 0u
#define XENOLITH_WAS_OPENED 1u
#define XENOLITH_WAS_MADE 2u
#define XENOLITH_WAS_EMPTIED 3u

/* Which parts of a file a title asked to be able to reach. The two whole kinds
 * sit at the top and the particular ones at the bottom. */
#define XENOLITH_MAY_WRITE_ANY 0x40000000u
#define XENOLITH_MAY_WRITE_DATA 0x00000002u
#define XENOLITH_MAY_ADD_TO_IT 0x00000004u

/* Where a call's ninth argument sits above the stack pointer, the eight
 * registers reaching only as far as the eighth. */
#define XENOLITH_NINTH_ARGUMENT 0x54u

/* Where the doubleword slot the ninth argument takes begins, of which the word
 * above is the low half. */
#define XENOLITH_NINTH_SLOT 0x50u

/* A title asking for a directory rather than a file says so here. */
#define XENOLITH_WANTS_A_DIRECTORY 0x00000001u

/* What one thing in a directory takes before its name, and what a title is told
 * a thing is. The console's own numbers. */
#define XENOLITH_WHAT_ONE_THING_TAKES 0x40u
#define XENOLITH_IT_IS_A_FILE 0x00000080u
#define XENOLITH_IT_IS_A_DIRECTORY 0x00000010u
#define XENOLITH_NO_MORE_THINGS 0x80000006u

/* How many turns a waiter that gave up can be remembered for.
 *
 * A waiter takes a number on the way in and goes when its number comes up. One
 * that gives up before then is not there to take it, so the numbers it leaves
 * behind have to be written down or the queue waits for ever on one nobody will
 * claim. */
#define XENOLITH_TURNS_LEFT 16

/* Guest memory is handed out a grain at a time, so a size becomes a whole
 * number of grains, and a size of nothing still takes one.
 *
 * A size close to the top of what a word holds rounds up past it, so the
 * rounding is worked out in a wider number and reported as nought where the
 * answer does not fit. Rounded in a word instead, a request for nearly four
 * gigabytes came back as one grain: the addition wrapped to nought, and the
 * caller was handed a sixteenth of a megabyte where it asked for all of memory.
 */
static uint32_t whole_grains(uint32_t size) {
    uint64_t grains = ((uint64_t)size + XENOLITH_HEAP_GRAIN - 1) / XENOLITH_HEAP_GRAIN;
    if (grains == 0) {
        return XENOLITH_HEAP_GRAIN;
    }
    if (grains > UINT32_MAX / XENOLITH_HEAP_GRAIN) {
        return 0;
    }
    return (uint32_t)(grains * XENOLITH_HEAP_GRAIN);
}

/* Where guest memory is handed out from.
 *
 * One arena serves both the virtual and the physical allocator. The console
 * distinguishes the two and this does not: every guest address is mapped and
 * zeroed before a title starts, and none of it is nearer the graphics hardware
 * than any other.
 *
 * Nothing is ever given back. A title that allocated and freed in a loop would
 * exhaust this where the console would not.
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

    /* Where the next grain would start, and whether it fits.
     *
     * The fit is checked against the end rather than as a subtraction from it.
     * Written as `rounded > XENOLITH_HEAP_END - at`, an `at` above the end
     * makes the subtraction wrap to something enormous and the test passes,
     * which hands out an address past the arena and past everything the runtime
     * manages. That is not hypothetical: one title asks for nearly four
     * gigabytes, gets an address past the end, and the fill it then writes
     * reaches the image's own data section and overwrites a live object.
     *
     * Two ways of arriving above the end are both refused. `at` can be above it
     * already, which a wrapped `handed_out + alignment - 1` produces, and
     * `at + rounded` can be, which an ask larger than what is left produces. */
    if (at < handed_out || at > XENOLITH_HEAP_END ||
        rounded > XENOLITH_HEAP_END - at) {
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
 * There is no alignment among the five arguments, so none is read. Whatever a
 * register past them holds is left over from the caller, and taken for an
 * alignment it turns a pointer into a mask that refuses an ask that fits.
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

    /* A size that cannot be rounded into a word is refused rather than handed
     * back as the nought it rounds to, which a title would take for an answer. */
    uint32_t rounded = whole_grains(size);
    if (rounded == 0) {
        ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
        return;
    }

    uint32_t at = wanted & ~(XENOLITH_HEAP_GRAIN - 1);
    if (at == 0) {
        at = take_guest_memory(size, 0);
        if (at == 0) {
            ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
            return;
        }
    }

    if (address_at) {
        xenolith_store32(base, address_at, at);
    }
    if (size_at) {
        xenolith_store32(base, size_at, rounded);
    }
    ctx->r[3] = XENOLITH_STATUS_SUCCESS;
}

/* Memory the graphics hardware can reach.
 *
 * This cannot share the shape above and that is the whole point of it being
 * separate: it takes its size by value rather than through a pointer, and it
 * answers with the address itself rather than a status. Read the other way it
 * takes a flag word for a pointer and writes the answer over whatever that
 * addresses.
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
/* The lock every report is written under.
 *
 * A report is several writes to one stream, and two threads writing at once
 * interleave them: a line arrives with another line's middle inside it, which
 * is worse than no report at all because it reads as a fact. Every report here
 * takes this lock, so a line arrives whole. */
static pthread_mutex_t reporting = PTHREAD_MUTEX_INITIALIZER;

/* Writes one report, under the lock, so that a line arrives whole. */
static void say_a_report(const char *format, ...) {
    va_list arguments;
    va_start(arguments, format);
    pthread_mutex_lock(&reporting);
    fprintf(stderr, "xenolith: ");
    vfprintf(stderr, format, arguments);
    fprintf(stderr, "\n");
    fflush(stderr);
    pthread_mutex_unlock(&reporting);
    va_end(arguments);
}

static void assumed(const char *what, const char *why) {
    static const char *said[16];
    static unsigned count = 0;

    pthread_mutex_lock(&reporting);
    for (unsigned at = 0; at < count; at++) {
        if (said[at] == what) {
            pthread_mutex_unlock(&reporting);
            return;
        }
    }
    if (count < sizeof said / sizeof *said) {
        said[count++] = what;
    }
    fprintf(stderr, "xenolith: assuming %s, because %s\n", what, why);
    fflush(stderr);
    pthread_mutex_unlock(&reporting);
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
 * The answer is a bit in the system flags word the title's own container
 * carries, which is passed in by the program that booted it. A privilege number
 * names a bit, so the question is whether that bit is set.
 *
 * Answering no to everything was the first thing tried, and it is wrong on a
 * real title: one asks whether it may do something, is told no, and shuts
 * itself down before it draws a frame. The evidence for the answer was in the
 * container the whole time.
 *
 * A number past the width of the word names no bit, and is answered the same
 * way as a clear one. */
static uint32_t declared_privileges;

void xenolith_privileges(uint32_t flags) {
    declared_privileges = flags;
    if (flags == 0) {
        say_a_report("the container did not say what the title is allowed to do");
    }
}

static void check_executable_privilege(xenolith_context *ctx) {
    uint32_t which = (uint32_t)ctx->r[3];
    ctx->r[3] = which < 32u ? (declared_privileges >> which) & 1u : 0u;
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
 * The values are the thread's own, and that is the whole point of them: a title
 * puts something in a slot on one thread and expects to read it back on that
 * thread and nowhere else. One set shared between threads gives every thread
 * the last value written by any of them, which is the kind of wrongness that
 * shows up as one worker reading another's work.
 *
 * The slots are numbered across all threads, since a title allocates one on one
 * thread and uses the same number on the others. What is per thread is what is
 * in them.
 */
#define XENOLITH_TLS_SLOTS 64

static _Thread_local uint32_t tls_values[XENOLITH_TLS_SLOTS];
static uint32_t tls_taken = 0;
static pthread_mutex_t taking_a_slot = PTHREAD_MUTEX_INITIALIZER;

static void tls_alloc(xenolith_context *ctx) {
    /* Under the lock, because two threads starting at once would otherwise be
     * given the same slot and write over each other's work. */
    pthread_mutex_lock(&taking_a_slot);
    if (tls_taken >= XENOLITH_TLS_SLOTS) {
        pthread_mutex_unlock(&taking_a_slot);
        ctx->r[3] = 0xFFFFFFFFu;
        return;
    }
    ctx->r[3] = tls_taken;
    tls_taken++;
    pthread_mutex_unlock(&taking_a_slot);
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
    OBJECT_FILE,
    OBJECT_ENUMERATOR,
};

struct object {
    enum object_kind kind;
    pthread_mutex_t lock;
    pthread_cond_t changed;
    int signalled;
    int manual;
    long count;
    long limit;
    /* How many are waiting, and how many of them a set has already let through.
     * Keeping the second apart from the signalled state is what makes two sets
     * release two waiters rather than one. */
    long blocked;
    long releases;
    /* Which of them a set lets through.
     *
     * The kernel takes the one that has waited longest, and where two threads
     * wait on one object for different things, taking another leaves the one
     * that could have carried on asleep and the one that cannot holding the
     * answer. So a waiter takes a number on the way in and goes when its number
     * comes up. */
    unsigned long long taking;
    unsigned long long serving;
    unsigned long long gone[XENOLITH_TURNS_LEFT];
    uint8_t gone_taken[XENOLITH_TURNS_LEFT];
    /* Who holds this and how many times over.
     *
     * One of these is not a mark that anyone may take: it belongs to whoever
     * took it, and that thread may take it again without waiting, as many times
     * as it likes, and lets it go once for each. A thread asking for one it
     * already holds and being made to wait is a thread waiting for itself, and
     * everything else waiting for it waits for ever. */
    pthread_t holder;
    long held;
    /* A file a title has open, and a directory it has asked about. A title
     * names its files with a prefix of its own and never with a path a host
     * would recognise, so what a handle stands for is worked out once here and
     * kept rather than worked out again on every read. */
    FILE *file;
    char *directory;
    uint32_t gone_past;
    /* How far into a file reading or writing is up to, which a title sets
     * before each read rather than relying on the host's own position. */
    long long at;
    pthread_t thread;
    int running;
    int suspended;
    /* Which setting of a timer is the current one. Every set and every cancel
     * moves it on, so a countdown from an earlier setting finds it moved and
     * signals nothing. */
    unsigned long timer_round;
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

/* Puts a turn back that its waiter will not be taking.
 *
 * A waiter that gives up before its turn comes round is not there to take it,
 * and the turn after it is never reached while the queue is still waiting for
 * one nobody will claim. Where the turn given up is the one being served it is
 * simply passed over; where it is further along it is written down, and passed
 * over when the queue reaches it.
 *
 * Where there is nowhere left to write one down the queue is moved on instead.
 * That lets somebody through out of turn, which is worse than the order a
 * console keeps and far better than a queue that stops. */
static void give_up_the_turn(struct object *left, unsigned long long turn) {
    if (left->serving == turn) {
        left->serving++;
        return;
    }
    for (unsigned at = 0; at < XENOLITH_TURNS_LEFT; at++) {
        if (!left->gone_taken[at]) {
            left->gone_taken[at] = 1;
            left->gone[at] = turn;
            return;
        }
    }
    left->serving++;
}

/* Moves the queue past any turns whose waiters have gone. */
static void step_over_the_ones_that_left(struct object *waited) {
    for (;;) {
        unsigned found = XENOLITH_TURNS_LEFT;
        for (unsigned at = 0; at < XENOLITH_TURNS_LEFT; at++) {
            if (waited->gone_taken[at] && waited->gone[at] == waited->serving) {
                found = at;
                break;
            }
        }
        if (found == XENOLITH_TURNS_LEFT) {
            return;
        }
        waited->gone_taken[found] = 0;
        waited->serving++;
    }
}

/* A count of every time any object was let go, for a thread waiting on several
 * at once.
 *
 * Each object wakes only the threads waiting on it, and a thread waiting on any
 * of eight is not waiting on one of them in particular. So every set also steps
 * this and wakes whoever waits on it, and such a thread sleeps here between
 * looking at each of its objects in turn. */
static pthread_mutex_t anything_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t anything_changed = PTHREAD_COND_INITIALIZER;
static unsigned long long anything_count;

static void say_something_changed(void) {
    pthread_mutex_lock(&anything_lock);
    anything_count++;
    pthread_cond_broadcast(&anything_changed);
    pthread_mutex_unlock(&anything_lock);
}

/* Waits until an object is signalled, taking it if taking is what waiting on
 * that kind means.
 *
 * `until` is the point on the host's clock to give up at, or nothing to wait for
 * as long as it takes. A wait that gives up looks once more before it does: a
 * set that lands at the moment a waiter gives up counted that waiter when it
 * decided to let one more through, and walking away leaves it spent on nobody
 * and waiting for whoever asks next.
 */
static uint32_t wait_until(struct object *waited, const struct timespec *until) {
    uint32_t status = XENOLITH_STATUS_SUCCESS;

    pthread_mutex_lock(&waited->lock);

    /* One a thread already holds is taken again on the spot, without joining the
     * queue: it never waited, so there is no turn of its own to take and none of
     * anybody else's to move past. */
    if (waited->kind == OBJECT_MUTANT && waited->held > 0 &&
        pthread_equal(waited->holder, pthread_self())) {
        waited->held++;
        pthread_mutex_unlock(&waited->lock);
        return XENOLITH_STATUS_SUCCESS;
    }

    waited->blocked++;
    unsigned long long mine = waited->taking++;
    for (;;) {
        step_over_the_ones_that_left(waited);
        /* A semaphore is got through by spending one of its count, however many
         * were let go and whenever. A mark of having been let go is one bit, so
         * two let go with nobody waiting would admit one, and the thread after
         * that would wait for ever on a count of one. */
        if (waited->kind == OBJECT_SEMAPHORE) {
            if (waited->count > 0 && waited->serving == mine) {
                waited->count--;
                waited->serving++;
                break;
            }
        } else if (waited->releases > 0 && waited->serving == mine) {
            waited->releases--;
            waited->serving++;
            break;
        } else if (waited->releases == 0 && waited->signalled && waited->serving == mine) {
            if (!waited->manual) {
                waited->signalled = 0;
            }
            waited->serving++;
            break;
        }
        if (until == NULL) {
            pthread_cond_wait(&waited->changed, &waited->lock);
        } else if (pthread_cond_timedwait(&waited->changed, &waited->lock, until) ==
                   ETIMEDOUT) {
            int nothing = waited->kind == OBJECT_SEMAPHORE
                              ? waited->count == 0
                              : waited->releases == 0 && !waited->signalled;
            if (nothing) {
                status = XENOLITH_STATUS_TIMEOUT;
                give_up_the_turn(waited, mine);
                break;
            }
        }
    }
    waited->blocked--;
    /* And getting through one of the other sort is taking it, which is what says
     * who may let it go and who may take it again without waiting. */
    if (status == XENOLITH_STATUS_SUCCESS && waited->kind == OBJECT_MUTANT) {
        waited->holder = pthread_self();
        waited->held = 1;
    }
    pthread_mutex_unlock(&waited->lock);
    return status;
}

/* How many seconds the console's clock counts from before a host's does. The
 * first of January 1601 to the first of January 1970, which is a definition. */
#define XENOLITH_TO_THE_CONSOLE_EPOCH 11644473600LL

/* How long a title asked to wait for, as the moment to give up at.
 *
 * A count is in hundreds of nanoseconds, negative for a stretch from now.
 * Nothing is asking to wait for as long as it takes, and so is a count on the
 * clock a console kept, which is a moment this runtime has no reading of.
 */
static int give_up_at(uint8_t *base, uint32_t asked_at, struct timespec *until) {
    if (asked_at == 0) {
        return 0;
    }

    int64_t asked = (int64_t)xenolith_load64(base, asked_at);

    clock_gettime(CLOCK_REALTIME, until);

    int64_t hundreds;
    if (asked < 0) {
        /* A stretch from now. */
        hundreds = -asked;
    } else {
        /* A moment on the console's clock, which counts hundreds of nanoseconds
         * from the start of 1601 where a host's counts seconds from 1970. How
         * far apart those two are is a definition rather than a property of any
         * hardware, so how far off the moment is can be worked out. */
        int64_t since = ((int64_t)until->tv_sec + XENOLITH_TO_THE_CONSOLE_EPOCH) * 10000000
                        + (int64_t)until->tv_nsec / 100;
        hundreds = asked > since ? asked - since : 0;
    }

    until->tv_sec += (time_t)(hundreds / 10000000);
    until->tv_nsec += (long)((hundreds % 10000000) * 100);
    if (until->tv_nsec >= 1000000000L) {
        until->tv_sec += 1;
        until->tv_nsec -= 1000000000L;
    }
    return 1;
}

/* Sets an object signalled and wakes whoever was waiting.
 *
 * One set lets one waiter through, so what is counted is how many of them a set
 * has let through and not yet spent. Keeping that apart from the signalled state
 * is what makes two sets release two waiters rather than one: a state is a bit,
 * and the second set would land on a bit that is already set.
 *
 * `manual` is the sort that stays set until it is cleared, which lets every
 * waiter through and none of them spend it.
 */
static void signal_object(struct object *woken, int state) {
    pthread_mutex_lock(&woken->lock);
    if (woken->kind == OBJECT_SEMAPHORE) {
        /* Its count is what admits, and was raised by whoever let it go. */
    } else if (state && !woken->manual) {
        /* One set lets one waiter through, whether or not one is waiting yet.
         * Counted rather than remembered as a bit, because a bit is one and
         * two sets would be one set: the second lands on a mark that is already
         * there and the thread after it waits for ever. */
        woken->releases++;
    } else {
        woken->signalled = state;
    }
    pthread_cond_broadcast(&woken->changed);
    pthread_mutex_unlock(&woken->lock);
    say_something_changed();
}

/* Lets go of a lock a thread holds, once, and says how many times over it was
 * held beforehand.
 *
 * Only the thread holding it can let it go, and only as many times as it took
 * it. The last letting go is what lets another thread take it. Set as an event
 * instead, a thread that takes it again straight after finds itself still
 * holding it and walks in without spending the setting, which leaves it free
 * for a second thread as well, and the two are inside the same lock at once. */
static long release_lock(struct object *lock) {
    pthread_mutex_lock(&lock->lock);
    long was = lock->held;
    if (lock->held > 0 && pthread_equal(lock->holder, pthread_self())) {
        lock->held--;
        if (lock->held == 0) {
            lock->signalled = 1;
            pthread_cond_broadcast(&lock->changed);
        }
    }
    pthread_mutex_unlock(&lock->lock);
    say_something_changed();
    return was;
}

/* Waits until any one of several objects is signalled, and takes that one.
 *
 * Each is looked at in turn with a wait that gives up at once, which takes it
 * where it can be taken and leaves it alone where it cannot. Where none can,
 * the thread sleeps until something is let go and looks again. The count is
 * read before looking, so a set that lands while the objects are being looked
 * at is not slept through.
 *
 * What comes back is the place in the list of the one taken, which is what the
 * kernel answers, or the status saying the time ran out. */
static uint32_t wait_for_any(struct object **waited, uint32_t count,
                             const struct timespec *until) {
    for (;;) {
        pthread_mutex_lock(&anything_lock);
        unsigned long long seen = anything_count;
        pthread_mutex_unlock(&anything_lock);

        struct timespec now;
        clock_gettime(CLOCK_REALTIME, &now);
        for (uint32_t at = 0; at < count; at++) {
            if (waited[at] != NULL &&
                wait_until(waited[at], &now) == XENOLITH_STATUS_SUCCESS) {
                return at;
            }
        }

        pthread_mutex_lock(&anything_lock);
        while (anything_count == seen) {
            if (until == NULL) {
                pthread_cond_wait(&anything_changed, &anything_lock);
            } else if (pthread_cond_timedwait(&anything_changed, &anything_lock, until) ==
                       ETIMEDOUT) {
                pthread_mutex_unlock(&anything_lock);
                return XENOLITH_STATUS_TIMEOUT;
            }
        }
        pthread_mutex_unlock(&anything_lock);
    }
}

/* The block a title reaches through r13, and the count of ticks inside it.
 *
 * The console gives every hardware thread a block holding what the kernel keeps
 * about it, and a title reads it for two things this runtime has to answer: how
 * many ticks have passed, and which processor this is.
 *
 * The count advances a thousand times a second. That the unit is a millisecond
 * is an assumption: what is checkable is that a title compares it against five
 * thousand and treats that as long enough to give up on a frame, and five
 * seconds is a length that fits. It is reported as an assumption rather than
 * left implied.
 *
 * Which processor this is matters for a reason worth writing down. A title's
 * vector code picks a slot in the block by that number and steps a counter
 * there. Left at nothing every thread reads processor nought, so they all take
 * the same slot and the same counter, and two threads stepping it at once carry
 * it past the end of the block. What that looks like is memory elsewhere in the
 * image turning into vectors. So each thread is given one of the six in turn. A
 * console runs many software threads on those six and only one holds a hardware
 * thread at a time; here they really are concurrent, so handing them out in
 * turn makes two threads sharing a block rare rather than impossible.
 */
#define XENOLITH_PROCESSOR_SHARED 256u
#define XENOLITH_PROCESSOR_TICKS 88u
#define XENOLITH_PROCESSOR_NUMBER 268u
#define XENOLITH_PROCESSORS 6u

/* The block holding the tick count, shared by every processor's block. */
static uint32_t shared_processor_state;
static pthread_mutex_t starting_the_count = PTHREAD_MUTEX_INITIALIZER;

/* Keeps the shared tick count up to date.
 *
 * What the count says is how long has passed, not how many times this woke.
 * Counting a tick each time round asks to be woken every millisecond and trusts
 * that it was. Asking to sleep for one is asking for at least one, and a host
 * gives back rather more, so a count made that way falls behind whatever it is
 * meant to be a count of by however much the host overshot. Everything a title
 * times against it then takes longer than it asked for, and how much longer
 * depends on what else the machine was doing.
 */
/* Where the kernel's own clock readings live, for a title that imports them, or
 * nought where it does not. The count below keeps them current. */
static uint32_t time_stamp_bundle;

/* Writes the three readings a title takes from the kernel's clock: hundreds of
 * nanoseconds since the start, the same counted from 1601 on the wall clock,
 * and milliseconds since the start. */
static void stamp_the_time(uint8_t *base, uint64_t since_ms, const struct timespec *since) {
    struct timespec wall;
    clock_gettime(CLOCK_REALTIME, &wall);
    uint64_t running = (uint64_t)since->tv_sec * 10000000ull + (uint64_t)since->tv_nsec / 100u;
    uint64_t on_the_wall =
        ((uint64_t)wall.tv_sec + (uint64_t)XENOLITH_TO_THE_CONSOLE_EPOCH) * 10000000ull
        + (uint64_t)wall.tv_nsec / 100u;
    xenolith_store64(base, time_stamp_bundle, running);
    xenolith_store64(base, time_stamp_bundle + 8, on_the_wall);
    xenolith_store32(base, time_stamp_bundle + 16, (uint32_t)since_ms);
}

static void *count_the_ticks(void *given) {
    uint8_t *base = given;
    struct timespec began;

    if (clock_gettime(CLOCK_MONOTONIC, &began) != 0) {
        return NULL;
    }

    while (1) {
        struct timespec now;
        if (clock_gettime(CLOCK_MONOTONIC, &now) == 0) {
            uint64_t since = (uint64_t)(now.tv_sec - began.tv_sec) * 1000ull
                             + (uint64_t)(now.tv_nsec / 1000000)
                             - (uint64_t)(began.tv_nsec / 1000000);
            xenolith_store32(base, shared_processor_state + XENOLITH_PROCESSOR_TICKS,
                             (uint32_t)since);
            if (time_stamp_bundle != 0) {
                struct timespec running = {now.tv_sec - began.tv_sec, now.tv_nsec - began.tv_nsec};
                if (running.tv_nsec < 0) {
                    running.tv_sec -= 1;
                    running.tv_nsec += 1000000000L;
                }
                stamp_the_time(base, since, &running);
            }
        }

        struct timespec once = {0, 1000000};
        nanosleep(&once, NULL);
    }
    return NULL;
}

/* Hands out a block for a guest thread, all of them sharing the one count. */
static uint32_t take_processor_block(uint8_t *base) {
    pthread_mutex_lock(&starting_the_count);
    if (shared_processor_state == 0) {
        assumed("the kernel's tick counts a millisecond",
                "a title compares two readings of it against five thousand and "
                "gives up on a frame, and nothing here says what one tick is");
        shared_processor_state = take_guest_memory(XENOLITH_HEAP_GRAIN, XENOLITH_HEAP_GRAIN);

        pthread_t counting;
        if (pthread_create(&counting, NULL, count_the_ticks, base) == 0) {
            pthread_detach(counting);
        }
    }
    pthread_mutex_unlock(&starting_the_count);

    static uint32_t processors_handed_out;

    uint32_t block = take_guest_memory(XENOLITH_HEAP_GRAIN, XENOLITH_HEAP_GRAIN);
    uint32_t which = __atomic_fetch_add(&processors_handed_out, 1u, __ATOMIC_RELAXED);
    xenolith_store32(base, block + XENOLITH_PROCESSOR_SHARED, shared_processor_state);
    xenolith_store8(base, block + XENOLITH_PROCESSOR_NUMBER,
                    (uint8_t)(which % XENOLITH_PROCESSORS));
    return block;
}

/* The same block, for whatever enters a title rather than a thread it started. */
uint32_t xenolith_processor_block(uint8_t *base) {
    return take_processor_block(base);
}

/* What kernel build the image was made against, kept for anything that asks. */
static uint32_t built_against;

/* How fast the graphics hardware is clocked, in megahertz. A title divides by
 * the display's pixel clock to pace itself against it. */
#define XENOLITH_GPU_CLOCK_MHZ 500u

/* How much room each imported variable is given. The largest the kernel
 * exports that a title was seen to import is the clock readings, at twenty four
 * bytes. */
#define XENOLITH_VARIABLE_ROOM 64u

void xenolith_import_variable(uint8_t *base, uint32_t slot, const char *library,
                              uint32_t ordinal) {
    static uint32_t room, used;
    if (room == 0 || used + XENOLITH_VARIABLE_ROOM > XENOLITH_HEAP_GRAIN) {
        room = take_guest_memory(XENOLITH_HEAP_GRAIN, XENOLITH_HEAP_GRAIN);
        used = 0;
        if (room == 0) {
            say_a_report("no room for %s variable %u", library, ordinal);
            return;
        }
    }
    uint32_t home = room + used;
    used += XENOLITH_VARIABLE_ROOM;
    xenolith_store32(base, slot, home);

    if (strcmp(library, "xboxkrnl.exe") != 0) {
        say_a_report("%s variable %u is not known, and is left as nothing", library, ordinal);
        return;
    }
    switch (ordinal) {
    case 448:
        xenolith_store32(base, home, XENOLITH_GPU_CLOCK_MHZ);
        break;
    case 344:
        /* The kernel's version, as the build the title was made against: a
         * title runs on the kernel it was built for. */
        xenolith_store16(base, home, (uint16_t)(built_against >> 28));
        xenolith_store16(base, home + 2, (uint16_t)((built_against >> 24) & 0xfu));
        xenolith_store16(base, home + 4, (uint16_t)((built_against >> 8) & 0xffffu));
        xenolith_store8(base, home + 7, (uint8_t)built_against);
        break;
    case 173:
        time_stamp_bundle = home;
        break;
    case 27:
    case 89:
    case 403:
    case 430:
    case 446:
    case 449:
    case 614:
        /* Left as nothing, which each of these is on a retail console with no
         * debugger attached, or is a lock nobody holds, or is a pointer or a
         * description only the kernel reads. The command line is empty, which
         * is a title started with no arguments. */
        break;
    default:
        say_a_report("%s variable %u is not known, and is left as nothing", library, ordinal);
        break;
    }
}

void xenolith_built_against(uint32_t build) {
    built_against = build;
    /* Said rather than kept to itself. A title's device layout depends on this
     * number, and nothing here uses it to choose one: what a runtime that does
     * not choose needs is for the number to be visible, so that a run says what
     * it was told and the absence of a choice can be seen rather than guessed
     * at. */
    if (build == 0) {
        say_a_report("the container did not say what kernel build it was made against");
    } else {
        say_a_report("the image was made against kernel build %u", build);
    }
}

/* Where each thread's stack is taken from, below the one the first thread was
 * given so that none of them meet.
 *
 * They come down towards the heap, and the heap is where they must stop. Left
 * to count down past it a stack is handed out over memory a title has been
 * given, and two threads write over each other's stack and each other's data.
 * A thread that cannot be given one is refused instead, which a title reports
 * rather than the runtime corrupting itself. */
#define XENOLITH_THREAD_STACK 0x00100000u

static uint32_t thread_stack_next = XENOLITH_STACK_TOP - XENOLITH_THREAD_STACK;
static pthread_mutex_t thread_stack_lock = PTHREAD_MUTEX_INITIALIZER;

/* The top of a thread's stack, or nought where there is no room for one. */
static uint32_t take_thread_stack(void) {
    pthread_mutex_lock(&thread_stack_lock);
    if (thread_stack_next < XENOLITH_HEAP_BASE + XENOLITH_THREAD_STACK) {
        pthread_mutex_unlock(&thread_stack_lock);
        return 0;
    }
    thread_stack_next -= XENOLITH_THREAD_STACK;
    uint32_t top = thread_stack_next;
    pthread_mutex_unlock(&thread_stack_lock);
    return top;
}

/* The thread object of the guest thread running here, or nothing on the thread
 * that entered the title. */
static _Thread_local struct object *this_thread;

/* Says a thread has finished, which is what anyone waiting on it waits for. */
static void finish_thread(struct object *self) {
    if (self != NULL) {
        self->manual = 1;
        signal_object(self, 1);
    }
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
    this_thread = self;

    /* A thread that cannot be given a stack never runs, and says so rather than
     * running on memory that belongs to something else. */
    uint32_t stack = take_thread_stack();
    if (stack == 0) {
        say_a_report("no room for another thread's stack");
        self->manual = 1;
        signal_object(self, 1);
        return NULL;
    }
    state.r[1] = stack - 64;
    state.r[13] = take_processor_block(self->base);
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

    finish_thread(self);
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
 * What such an object is comes from the head every one of them starts with,
 * read once when the object is first named: a byte saying what kind it is, and a
 * word saying whether it is signalled, or for a semaphore how many it admits. A
 * title writes that head itself rather than asking the kernel to, so it is the
 * only record there is. The kinds are the kernel's numbers. Anything that is not
 * one of them is taken for an event that a set lets one waiter through, which is
 * what such an object was always taken for here.
 */
#define XENOLITH_KIND_NOTIFICATION_EVENT 0u
#define XENOLITH_KIND_SEMAPHORE 5u
#define XENOLITH_NAMED_OBJECTS 1024

static struct {
    uint32_t at;
    uint32_t handle;
} named[XENOLITH_NAMED_OBJECTS];
static pthread_mutex_t named_lock = PTHREAD_MUTEX_INITIALIZER;

/* Makes the object a head in guest memory describes. */
static uint32_t take_object_described_at(uint8_t *base, uint32_t at) {
    uint8_t kind = xenolith_load8(base, at);
    int32_t state = (int32_t)xenolith_load32(base, at + 4);
    uint32_t handle =
        take_object(kind == XENOLITH_KIND_SEMAPHORE ? OBJECT_SEMAPHORE : OBJECT_EVENT);
    struct object *made = object_of(handle);
    if (made == NULL) {
        return 0;
    }
    if (kind == XENOLITH_KIND_SEMAPHORE) {
        made->count = state;
    } else if (kind == XENOLITH_KIND_NOTIFICATION_EVENT) {
        made->manual = 1;
        made->signalled = state != 0;
    } else {
        made->releases = state != 0;
    }
    return handle;
}

static struct object *object_at(uint8_t *base, uint32_t at) {
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
            /* The address is only written down once an object has been made for
             * it. Written first, a slot whose object could not be made stays
             * claimed, and every later wait on that address finds a slot that
             * names nothing and returns nothing for ever. */
            uint32_t handle = take_object_described_at(base, at);
            struct object *found = object_of(handle);
            if (found == NULL) {
                pthread_mutex_unlock(&named_lock);
                say_a_report("no room left for an object at %#x, and none left to make "
                             "one with",
                             (unsigned)at);
                return NULL;
            }
            named[slot].at = at;
            named[slot].handle = handle;
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
        /* One made already held belongs to the thread that made it, which
         * may take it again and lets it go once for each time. */
        made->manual = 0;
        if ((uint32_t)ctx->r[5] != 0) {
            made->holder = pthread_self();
            made->held = 1;
            made->signalled = 0;
        } else {
            made->signalled = 1;
        }
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

/* One setting of a timer, counting down on a thread of its own. */
struct timer_setting {
    struct object *timer;
    unsigned long round;
    struct timespec due;
    long period;
};

/* Signals a timer when it falls due, and again every period after that, until
 * the timer is set again or cancelled. */
static void *count_a_timer_down(void *given) {
    struct timer_setting setting = *(struct timer_setting *)given;
    free(given);

    for (;;) {
        while (clock_nanosleep(CLOCK_REALTIME, TIMER_ABSTIME, &setting.due, NULL) == EINTR) {
        }
        pthread_mutex_lock(&setting.timer->lock);
        int current = setting.timer->timer_round == setting.round;
        pthread_mutex_unlock(&setting.timer->lock);
        if (!current) {
            return NULL;
        }
        signal_object(setting.timer, 1);
        if (setting.period <= 0) {
            return NULL;
        }
        setting.due.tv_sec += setting.period / 1000;
        setting.due.tv_nsec += (setting.period % 1000) * 1000000L;
        if (setting.due.tv_nsec >= 1000000000L) {
            setting.due.tv_sec += 1;
            setting.due.tv_nsec -= 1000000000L;
        }
    }
}

/* Setting a timer to fall due after a stretch or at a moment, and then every
 * so many milliseconds if a period is given.
 *
 * Setting one takes back whatever it was set to before and leaves it not
 * signalled until it falls due. A routine a title names to be run when it does
 * is not run: running one means stopping a thread of the title's in a wait that
 * may be cut short, which nothing here does. That is said once rather than left
 * to look like a timer that never went. */
static void set_timer(xenolith_context *ctx, uint8_t *base) {
    struct object *timer = object_of((uint32_t)ctx->r[3]);
    if (timer == NULL) {
        ctx->r[3] = XENOLITH_STATUS_NOT_FOUND;
        return;
    }
    if ((uint32_t)ctx->r[5] != 0) {
        static int said;
        if (!said) {
            said = 1;
            say_a_report("a timer's routine at %#x is not run when the timer falls due",
                         (unsigned)ctx->r[5]);
        }
    }

    struct timer_setting *setting = malloc(sizeof *setting);
    if (setting == NULL) {
        ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
        return;
    }
    if (!give_up_at(base, (uint32_t)ctx->r[4], &setting->due)) {
        clock_gettime(CLOCK_REALTIME, &setting->due);
    }
    setting->timer = timer;
    setting->period = (long)(int32_t)ctx->r[9];

    pthread_mutex_lock(&timer->lock);
    setting->round = ++timer->timer_round;
    timer->signalled = 0;
    timer->releases = 0;
    pthread_mutex_unlock(&timer->lock);

    pthread_t counting;
    if (pthread_create(&counting, NULL, count_a_timer_down, setting) != 0) {
        free(setting);
        ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
        return;
    }
    pthread_detach(counting);
    ctx->r[3] = XENOLITH_STATUS_SUCCESS;
}

/* Cancelling a timer, which leaves it as it is and stops it falling due. What
 * comes back, where a title asks, is whether it was signalled. */
static void cancel_timer(xenolith_context *ctx, uint8_t *base) {
    struct object *timer = object_of((uint32_t)ctx->r[3]);
    if (timer == NULL) {
        ctx->r[3] = XENOLITH_STATUS_NOT_FOUND;
        return;
    }
    pthread_mutex_lock(&timer->lock);
    timer->timer_round++;
    uint32_t was = timer->signalled || timer->releases > 0;
    pthread_mutex_unlock(&timer->lock);
    if ((uint32_t)ctx->r[4] != 0) {
        xenolith_store32(base, (uint32_t)ctx->r[4], was);
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

/* Which cable a console's video output is going through.
 *
 * The values are the console's own numbering, and the title's startup compares
 * against four of them. Three is the component cable. */
#define XENOLITH_AV_PACK_COMPONENT 3u

/* Where a console was sold and what language it speaks, by the console's own
 * numbers: every North American market, and English. */
#define XENOLITH_REGION_NORTH_AMERICA 0x00ffu
#define XENOLITH_LANGUAGE_ENGLISH 1u

/* The one profile a run has: signed in on the console without the network, in
 * the first slot, with an identity of the kind such a profile has, whose top
 * nibble says it was never online. */
#define XENOLITH_SIGNED_IN_LOCALLY 1u
#define XENOLITH_PROFILE_XUID 0xe000000000000001ull
#define XENOLITH_PROFILE_NAME "Player"

/* What asking about a slot nobody is signed in to reports. */
#define XENOLITH_ERROR_NO_SUCH_USER 0x525u

/* Listeners a title asked for, and what each has still to say.
 *
 * A console tells a title a few things as it starts: that the system's own
 * screens came up and went away again, and that who is signed in changed,
 * twice. A title waits for those before it leaves the screen it starts on, and
 * one that is never told sits on a black screen drawing it for as long as it
 * runs. So the first listener that asks to hear about the system is given them,
 * and nothing after that is ever said, because nothing here changes. */
#define XENOLITH_LISTENERS 16u
#define XENOLITH_NOTICES_HELD 8u
#define XENOLITH_FIRST_LISTENER 0x100u
#define XENOLITH_HEARS_THE_SYSTEM 1u
#define XENOLITH_SYSTEM_SCREENS 0x9u
#define XENOLITH_SIGNED_IN_CHANGED 0xau

static struct {
    uint32_t said[XENOLITH_NOTICES_HELD];
    uint32_t about[XENOLITH_NOTICES_HELD];
    uint32_t held;
    uint64_t hears;
} listeners[XENOLITH_LISTENERS];
static uint32_t listeners_handed_out;
static int told_how_it_started;
static pthread_mutex_t listening = PTHREAD_MUTEX_INITIALIZER;

static uint32_t take_a_listener(uint64_t hears) {
    pthread_mutex_lock(&listening);
    if (listeners_handed_out >= XENOLITH_LISTENERS) {
        pthread_mutex_unlock(&listening);
        return 0;
    }
    uint32_t which = listeners_handed_out++;
    listeners[which].hears = hears;
    if (!told_how_it_started && (hears & XENOLITH_HEARS_THE_SYSTEM) != 0) {
        told_how_it_started = 1;
        const uint32_t said[4] = {XENOLITH_SYSTEM_SCREENS, XENOLITH_SYSTEM_SCREENS,
                                  XENOLITH_SIGNED_IN_CHANGED, XENOLITH_SIGNED_IN_CHANGED};
        const uint32_t about[4] = {1, 0, 1, 1};
        for (uint32_t at = 0; at < 4; at++) {
            listeners[which].said[at] = said[at];
            listeners[which].about[at] = about[at];
        }
        listeners[which].held = 4;
    }
    pthread_mutex_unlock(&listening);
    return XENOLITH_FIRST_LISTENER + which;
}

/* Tells every listener that hears about the system one more thing. One with no
 * room left misses it, which is what a console does with a queue that full. */
static void tell_the_system_listeners(uint32_t said, uint32_t about) {
    pthread_mutex_lock(&listening);
    for (uint32_t which = 0; which < listeners_handed_out; which++) {
        if ((listeners[which].hears & XENOLITH_HEARS_THE_SYSTEM) != 0
            && listeners[which].held < XENOLITH_NOTICES_HELD) {
            listeners[which].said[listeners[which].held] = said;
            listeners[which].about[listeners[which].held] = about;
            listeners[which].held++;
        }
    }
    pthread_mutex_unlock(&listening);
}

/* Takes the next thing a listener has to say, or the next of the kind a title
 * asked for where it names one, and says whether there was one. */
static int hear_from(uint32_t listener, uint32_t only, uint32_t *said, uint32_t *about) {
    uint32_t which = listener - XENOLITH_FIRST_LISTENER;
    int heard = 0;
    pthread_mutex_lock(&listening);
    if (which < listeners_handed_out) {
        for (uint32_t at = 0; at < listeners[which].held; at++) {
            if (only != 0 && listeners[which].said[at] != only) {
                continue;
            }
            *said = listeners[which].said[at];
            *about = listeners[which].about[at];
            for (uint32_t later = at + 1; later < listeners[which].held; later++) {
                listeners[which].said[later - 1] = listeners[which].said[later];
                listeners[which].about[later - 1] = listeners[which].about[later];
            }
            listeners[which].held--;
            heard = 1;
            break;
        }
    }
    pthread_mutex_unlock(&listening);
    return heard;
}

/* Says what the run read, and stops, when something outside it asks.
 *
 * Only the safe part is done here: writing one line through the runtime's own
 * lock is not async-signal-safe in the strictest reading, and the alternative is
 * a run that reports nothing at all. What is written is a snapshot of counters,
 * and the process ends immediately afterwards. */
static void say_what_was_read_and_stop(int which) {
    (void)which;
    say_what_was_read_in_total();
    _exit(3);
}

/* What a title registered as a sound client, if it registered one: what to
 * call to ask for the next frame of sound, and what to hand it. */
static uint32_t sound_client;
static uint32_t sound_client_callback;

/* The number a registered client is known by, and how often a frame is asked
 * for: one frame is two hundred and fifty six samples, and the console plays
 * forty eight thousand a second. */
#define XENOLITH_SOUND_DRIVER 0x41550000u
#define XENOLITH_NANOSECONDS_A_FRAME_OF_SOUND 5333333L

/* Asks the title for a frame of sound as often as the console would.
 *
 * The console's sound hardware calls the client a title registered each time
 * it is ready for another frame, and the title's own sound engine runs from
 * that call: it wakes the thread that mixes, which hands the frame back. A
 * title that is never asked never mixes, and everything it paces by its sound
 * waits with it, a film that keeps to its soundtrack among them. So a thread of
 * its own asks at the console's rate, for as long as a client is registered. */
static void *ask_for_sound(void *given) {
    uint8_t *base = given;
    static xenolith_context state;
    state.r[1] = take_thread_stack() - 64;
    state.r[13] = take_processor_block(base);

    struct timespec next;
    clock_gettime(CLOCK_MONOTONIC, &next);
    for (;;) {
        next.tv_nsec += XENOLITH_NANOSECONDS_A_FRAME_OF_SOUND;
        if (next.tv_nsec >= 1000000000L) {
            next.tv_sec += 1;
            next.tv_nsec -= 1000000000L;
        }
        while (clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &next, NULL) == EINTR) {
        }
        uint32_t callback = __atomic_load_n(&sound_client_callback, __ATOMIC_ACQUIRE);
        if (callback == 0) {
            continue;
        }
        xenolith_function body = xenolith_lookup(callback);
        if (body == NULL) {
            wanted_but_not_lifted(callback, 0, "asked for sound at");
            return NULL;
        }
        state.r[3] = __atomic_load_n(&sound_client, __ATOMIC_ACQUIRE);
        body(&state, base);
    }
    return NULL;
}

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
    uint32_t ring_words;
    uint32_t read_back_at;
    uint32_t callback;
    uint32_t callback_argument;
    /* How many times a title has asked for what it drew to be shown. */
    unsigned long frames;
    /* How many of those the reading has reached and shown. */
    unsigned long shown;
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

/* Which processor the end of a frame is said on. The packet that says commands
 * were reached names its own processors; this one has no packet, and the
 * third processor is the one the console says it on. */
#define XENOLITH_FRAMES_INTERRUPT_PROCESSOR 2u

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
static pthread_mutex_t one_interrupt = PTHREAD_MUTEX_INITIALIZER;

static void raise_the_graphics_interrupt(uint8_t *base, uint32_t source,
                                        uint32_t processor) {
    static _Thread_local xenolith_context state;
    static _Thread_local uint32_t stack;
    static _Thread_local uint32_t blocks[XENOLITH_PROCESSORS];

    if (graphics.callback == 0 || processor >= XENOLITH_PROCESSORS) {
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
    /* The block every thread reaches through r13, saying which processor the
     * handler runs on. That is not incidental: the handler clears the bit for
     * its own processor in a word the hardware waits on before it goes on, so a
     * handler on the wrong processor leaves the hardware waiting for ever. */
    if (blocks[processor] == 0) {
        blocks[processor] = take_processor_block(base);
        xenolith_store8(base, blocks[processor] + XENOLITH_PROCESSOR_NUMBER,
                        (uint8_t)processor);
    }
    state.r[13] = blocks[processor];
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
    /* One handler at a time, as on the processor it runs on: the end of a frame
     * is said from a thread of its own while the commands may say they were
     * reached, and the handler is not written to be entered twice at once. */
    pthread_mutex_lock(&one_interrupt);
    body(&state, base);
    pthread_mutex_unlock(&one_interrupt);
}

/* Where the block of registers sits, and how many there are.
 *
 * A title names a register by index, and the index for the ring buffer's write
 * pointer lands on the address the title stores to, so a register's address is
 * its index four times over from the front of the block.
 */
#define XENOLITH_REGISTERS 0x7fc80000u
#define XENOLITH_REGISTER_COUNT 0x8000u

/* Where a title's shader constants begin among the registers. */
#define XENOLITH_CONSTANT_REGISTER 0x4000u

/* How far through the ring buffer this has read, in words. */
static uint32_t consumed;

/* Reads a graphics register, or nothing where the index names none.
 *
 * The registers are a block of ordinary memory here rather than hardware, so a
 * register is its index four times over from the front of that block. A packet
 * set one and a title read it back, so the two have to be the same place. */
static uint32_t register_says(const uint8_t *base, uint32_t index) {
    if (index >= XENOLITH_REGISTER_COUNT) {
        return 0;
    }
    return xenolith_load32(base, XENOLITH_REGISTERS + index * 4);
}

/* What the processor reads back of a word the graphics hardware wrote to
 * memory, given the swap the hardware was told to make first.
 *
 * The hardware writes memory in the opposite byte order to the processor, after
 * swapping the value as the two bits say. So a swap of every byte reads back as
 * the value itself, a swap within each half as the halves exchanged, a swap of
 * the halves as the bytes of each half exchanged, and no swap at all as every
 * byte reversed. */
static uint32_t as_the_processor_reads(uint32_t value, uint32_t order) {
    uint32_t swapped_in_halves = ((value & 0x00ff00ffu) << 8) | ((value >> 8) & 0x00ff00ffu);
    switch (order & 3u) {
    case 0:
        return (swapped_in_halves << 16) | (swapped_in_halves >> 16);
    case 1:
        return (value << 16) | (value >> 16);
    case 2:
        return value;
    default:
        return swapped_in_halves;
    }
}

/* The scratch registers, and the two saying which of them the hardware copies
 * to memory when written, and where. */
#define XENOLITH_SCRATCH_FIRST 0x0578u
#define XENOLITH_SCRATCH_COUNT 8u
#define XENOLITH_SCRATCH_MASK 0x01dcu
#define XENOLITH_SCRATCH_ADDRESS 0x01ddu

/* Writes a graphics register, or nothing where the index names none.
 *
 * A packet naming a register past the end is a packet this has no reading of,
 * and writing it somewhere else would be worse than not writing it.
 *
 * Writing one of the eight scratch registers also writes it to memory, where
 * the mask says to: a word apiece from the address the title named, in the
 * byte order the top of the mask says. That is how a title is told how far
 * through its commands the hardware has got. It writes a scratch register at
 * the end of each run of commands and waits for the copy in memory to arrive,
 * and a register that only lands in the register file leaves it waiting for
 * ever on a word that never moves. */
static void set_register(uint8_t *base, uint32_t index, uint32_t value) {
    if (index < XENOLITH_REGISTER_COUNT) {
        xenolith_store32(base, XENOLITH_REGISTERS + index * 4, value);
    }
    if (index >= XENOLITH_SCRATCH_FIRST && index < XENOLITH_SCRATCH_FIRST + XENOLITH_SCRATCH_COUNT) {
        uint32_t which = index - XENOLITH_SCRATCH_FIRST;
        uint32_t mask = xenolith_load32(base, XENOLITH_REGISTERS + XENOLITH_SCRATCH_MASK * 4);
        uint32_t where = xenolith_load32(base, XENOLITH_REGISTERS + XENOLITH_SCRATCH_ADDRESS * 4);
        if ((mask >> which) & 1u && where != 0) {
            xenolith_store32(base, (where & ~3u) + which * 4u,
                             as_the_processor_reads(value, mask >> 16));
        }
    }
}

#include "drawing.c"

/* Shows a frame, when the commands reach the packet that says to.
 *
 * Not when the title asks. It asks once it has written a frame's commands, and
 * the reading may still be partway through them. Ending the frame there splits
 * it, and what was drawn before the split is wiped when the rest begins, so a
 * frame shows only some of what the title drew, and which part changes from
 * one frame to the next.
 *
 * What is shown is the copy the title made into the picture the packet names,
 * or the surface where it made none. It is then written out where a run asks
 * for it, which is the only way to see whether a title drew anything rather
 * than only that it asked to. */
static void show_a_frame(uint32_t front) {
    finish_the_frame();
    if (!xenolith_show_what_was_copied_to(front)) {
        xenolith_show_the_frame();
    }
    graphics.shown++;

    const char *keeping = getenv("XENOLITH_FRAMES");
    if (keeping != NULL) {
        char where[512];
        int written = snprintf(where, sizeof where, "%s/frame%04lu.ppm", keeping, graphics.shown);
        if (written > 0 && (size_t)written < sizeof where) {
            xenolith_keep_a_frame(where);
        }
    }
}

#include "command_stream.c"

#include "sound.c"

/* The most words one packet takes: a header and the longest count after it. */
#define XENOLITH_LONGEST_PACKET (0x3fffu + 2u)

/* Where a packet that runs off the end of the ring is put back together. */
static uint32_t mended_at;

/* Reads the packets a title wrote, and does what this can with them.
 *
 * Going backwards is the ring having wrapped. What is left up to its end is
 * read first, and a title carries a packet straight on from the end to the
 * front, so one may start in the last words and finish in the first. That one
 * is put back together in a place of its own and read there, once the title
 * has written the rest of it. Read from the front as it stands, its remainder
 * is taken for a header, and the count that header seems to give is longer
 * than anything written yet, so nothing is read again until the title writes
 * further. A title waiting on that packet never does. */
static void consume_the_ring(uint8_t *base, uint32_t written) {
    if (graphics.ring_at == 0 || graphics.ring_words == 0) {
        return;
    }

    if (written < consumed) {
        consumed += walk_the_commands(base, graphics.ring_at + consumed * 4,
                                      graphics.ring_words - consumed, 0);
        if (consumed < graphics.ring_words) {
            uint32_t left = graphics.ring_words - consumed;
            uint32_t whole = packet_words(xenolith_load32(base, graphics.ring_at + consumed * 4));
            uint32_t rest = whole - left;
            if (rest > written) {
                return;
            }
            if (mended_at == 0) {
                mended_at = take_guest_memory(XENOLITH_LONGEST_PACKET * 4, 0);
                if (mended_at == 0) {
                    return;
                }
            }
            for (uint32_t word = 0; word < whole; word++) {
                uint32_t from = word < left ? consumed + word : word - left;
                xenolith_store32(base, mended_at + word * 4,
                                 xenolith_load32(base, graphics.ring_at + from * 4));
            }
            walk_the_commands(base, mended_at, whole, 0);
            consumed = rest;
        } else {
            consumed = 0;
        }
    }

    if (consumed >= written) {
        return;
    }

    consumed += walk_the_commands(base, graphics.ring_at + consumed * 4,
                                  written - consumed, 0);
}

/* Says that the display finished a frame, sixty times a second, on a thread of
 * its own.
 *
 * Before saying so, the bit a title checks before it will retire anything is
 * set. It reads that out of a graphics register, and what it means is that
 * there is finished work to account for. Everything written here is finished
 * the moment it is written, so the bit is always the truth of this runtime even
 * though it is never the truth of any hardware.
 *
 * Not on the thread reading the commands. A title's commands wait for what the
 * end of a frame does, such as showing the next picture, and a reader waiting
 * there cannot be the one to say the frame ended. That the commands reached a
 * point is said when they reach it, by the packet that asks for it: a title
 * puts what to run in a scratch register just before that packet and a marker
 * back afterwards, and told at any other moment it calls the marker as though
 * it were what to run. */
static void *count_the_frames(void *given) {
    uint8_t *base = given;
    struct timespec next;
    clock_gettime(CLOCK_MONOTONIC, &next);
    for (;;) {
        next.tv_nsec += 16666667L;
        if (next.tv_nsec >= 1000000000L) {
            next.tv_sec += 1;
            next.tv_nsec -= 1000000000L;
        }
        while (clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &next, NULL) == EINTR) {
        }
        xenolith_store32(base, XENOLITH_GRAPHICS_COMPLETED, 1);
        raise_the_graphics_interrupt(base, XENOLITH_INTERRUPT_RETIRE,
                                     XENOLITH_FRAMES_INTERRUPT_PROCESSOR);
    }
    return NULL;
}

static void *drain_the_ring(void *given) {
    uint8_t *base = given;
    unsigned ticks = 0;

    assumed("the graphics hardware keeps up with the title",
            "nothing here reads the commands a title writes, so the only "
            "alternative is a title waiting forever");

    unsigned long reported_at = 0;

    while (graphics.read_back_at != 0) {
        uint32_t written = xenolith_load32(base, XENOLITH_RING_WRITE_POINTER);
        consume_the_ring(base, written);
        xenolith_store32(base, graphics.read_back_at, consumed);

        /* A display finishes a frame sixty times a second. This one finishes
         * nothing, at the same rate.
         *
         * Before saying so, the bit a title checks before it will retire
         * anything is set. It reads that out of a graphics register, and what
         * it means is that there is finished work to account for. Everything
         * written here is finished the moment it is written, so the bit is
         * always the truth of this runtime even though it is never the truth of
         * any hardware. */
        /* What has been read, once a second, so that a run says what it found
         * rather than only what it did not draw. */
        if (read_so_far.packets != reported_at && ++ticks % 600 == 0) {
            reported_at = read_so_far.packets;
            say_what_was_read();
        }

        struct timespec briefly = {0, 100000};
        nanosleep(&briefly, NULL);
    }
    return NULL;
}

/* The standard a North American console's signal follows, and how fast the
 * pixels of a 1280 by 720 picture at sixty go, in whole megahertz. */
#define XENOLITH_VIDEO_STANDARD_NTSC 1u
#define XENOLITH_PIXEL_CLOCK_MHZ 74u

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
 * The three words between them say whether the picture is interlaced, wide and
 * high definition. A picture 1280 wide and 720 tall is not interlaced, and is
 * wide and high definition by what those words mean, so the last two say so.
 *
 * Two words after the rate are written as well, because a title reads the
 * second of them: the standard the signal follows, and how fast its pixels go in
 * megahertz. One title divides the graphics clock by the second, and its
 * compiler put a trap in front of the division for a divisor of nought, which a
 * structure left half written gave it. The signal for this picture is clocked at
 * seventy four and a quarter megahertz, and the standard is the one a North
 * American console sends. Nothing past those is touched, since where the
 * structure ends is not derivable from a title that never reads that far.
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
    xenolith_store32(base, at + 12, 1);
    xenolith_store32(base, at + 16, 1);
    xenolith_store32(base, at + 20, rate);
    xenolith_store32(base, at + 24, XENOLITH_VIDEO_STANDARD_NTSC);
    xenolith_store32(base, at + 28, XENOLITH_PIXEL_CLOCK_MHZ);
}

/* What the runtime answers to, by ordinal within a library.
 *
 * Each entry is here because a title reached it. Nothing is implemented ahead
 * of a title asking for it, so this list is a record of what was needed rather
 * than a guess at what might be.
 */
static int serviced_about_what_it_holds(xenolith_context *ctx, uint8_t *base, uint32_t ordinal);
static int serviced_about_memory_and_time(xenolith_context *ctx, uint8_t *base, uint32_t ordinal);
static int serviced_about_what_it_sees(xenolith_context *ctx, uint8_t *base, uint32_t ordinal);
/* Where a title's own files are.
 *
 * A title names them with a prefix of its own and never with a path a host
 * would recognise. What that prefix stands for is the directory its image was
 * loaded from, because on a console the image and its files were on the same
 * disc and a title has no way of saying anything else. So this is taken from
 * where the image was found rather than configured, and nothing here is per
 * title.
 */
static char title_directory[4096] = ".";

static void remember_where_the_image_was(const char *image) {
    size_t cut = 0;
    for (size_t at = 0; image[at] != '\0'; at++) {
        if (image[at] == '/') {
            cut = at;
        }
    }
    if (cut == 0 || cut >= sizeof title_directory) {
        return;
    }
    memcpy(title_directory, image, cut);
    title_directory[cut] = '\0';
}

/* Turns what a title calls a file into a path a host can open.
 *
 * The name arrives as a counted run of bytes inside a structure, with the
 * device and the path separated by a colon. The device is dropped: a console
 * keeps its own files and a title's saved ones on different devices, and what
 * that comes to here is one directory rather than two.
 */
static int host_path_of(uint8_t *base, uint32_t attributes, char *out, size_t room) {
    if (attributes == 0) {
        return 0;
    }

    uint32_t name_at = xenolith_load32(base, attributes + 4);
    if (name_at == 0) {
        return 0;
    }
    uint16_t length = xenolith_load16(base, name_at);
    uint32_t text = xenolith_load32(base, name_at + 4);
    if (text == 0) {
        return 0;
    }

    uint16_t from = 0;
    for (uint16_t at = 0; at < length; at++) {
        if (xenolith_load8(base, text + at) == ':') {
            from = (uint16_t)(at + 1);
        }
    }

    size_t used = strlen(title_directory);
    if (used + 2 >= room) {
        return 0;
    }
    memcpy(out, title_directory, used);

    char first = from < length ? (char)xenolith_load8(base, text + from) : '/';
    if (first != '/' && first != '\\') {
        out[used++] = '/';
    }

    for (uint16_t at = from; at < length; at++) {
        char letter = (char)xenolith_load8(base, text + at);
        if (letter == '\\') {
            letter = '/';
        }
        if (used + 1 >= room) {
            return 0;
        }
        out[used++] = letter;
    }
    out[used] = '\0';
    return 1;
}

/* Writes what an operation did into the block a title handed over for it. */
static void report_io(uint8_t *base, uint32_t block, uint32_t status, uint32_t did) {
    if (block != 0) {
        xenolith_store32(base, block, status);
        xenolith_store32(base, block + 4, did);
    }
}

/* Makes every directory above a path, so that a file a title names inside a
 * place it has just made for itself can be put there. A directory already there
 * is not a failure. */
static void make_the_way_to(const char *path) {
    char along[4096];
    size_t used = strlen(path);
    if (used >= sizeof along) {
        return;
    }
    memcpy(along, path, used + 1);
    for (size_t at = 1; at < used; at++) {
        if (along[at] != '/') {
            continue;
        }
        along[at] = '\0';
        mkdir(along, 0777);
        along[at] = '/';
    }
}

/* Opening a file a title names, or making one where it says to. */
static void open_file(xenolith_context *ctx, uint8_t *base, int may_make) {
    char path[4096];
    uint32_t handle_at = (uint32_t)ctx->r[3];
    uint32_t may = (uint32_t)ctx->r[4];
    uint32_t status_at = (uint32_t)ctx->r[6];
    uint32_t say_to = may_make ? (uint32_t)ctx->r[10] : XENOLITH_OPEN_IT;
    uint32_t options = may_make
                           ? (uint32_t)xenolith_load32(base,
                                                       (uint32_t)ctx->r[1] +
                                                           XENOLITH_NINTH_ARGUMENT)
                           : (uint32_t)ctx->r[8];

    if (!host_path_of(base, (uint32_t)ctx->r[5], path, sizeof path)) {
        report_io(base, status_at, XENOLITH_STATUS_NO_SUCH_FILE, 0);
        ctx->r[3] = XENOLITH_STATUS_NO_SUCH_FILE;
        return;
    }

    int writing = (may & (XENOLITH_MAY_WRITE_ANY | XENOLITH_MAY_WRITE_DATA |
                          XENOLITH_MAY_ADD_TO_IT)) != 0;
    int already = access(path, F_OK) == 0;

    /* A directory rather than a file. What is kept is the path, so that what is
     * in it can be read out later. */
    if ((options & XENOLITH_WANTS_A_DIRECTORY) != 0) {
        if (!already && may_make && say_to != XENOLITH_OPEN_IT) {
            make_the_way_to(path);
            mkdir(path, 0777);
            already = access(path, F_OK) == 0;
        }
        if (!already) {
            report_io(base, status_at, XENOLITH_STATUS_NO_SUCH_FILE, 0);
            ctx->r[3] = XENOLITH_STATUS_NO_SUCH_FILE;
            return;
        }
        uint32_t held = take_object(OBJECT_FILE);
        struct object *made = object_of(held);
        if (made == NULL) {
            report_io(base, status_at, XENOLITH_STATUS_NO_MEMORY, 0);
            ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
            return;
        }
        free(made->directory);
        made->directory = strdup(path);
        made->gone_past = 0;
        if (handle_at != 0) {
            xenolith_store32(base, handle_at, held);
        }
        report_io(base, status_at, XENOLITH_STATUS_SUCCESS, XENOLITH_WAS_OPENED);
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return;
    }

    /* What a title said to do about the file being there or not. A refusal here
     * is the title's own instruction and not a gap. */
    const char *how = "rb";
    uint32_t did = XENOLITH_WAS_OPENED;
    if (writing) {
        switch (say_to) {
        case XENOLITH_OPEN_IT:
            how = already ? "r+b" : NULL;
            break;
        case XENOLITH_MAKE_IT:
            how = already ? NULL : "w+b";
            did = XENOLITH_WAS_MADE;
            break;
        case XENOLITH_OPEN_OR_MAKE_IT:
            how = already ? "r+b" : "w+b";
            did = already ? XENOLITH_WAS_OPENED : XENOLITH_WAS_MADE;
            break;
        case XENOLITH_EMPTY_IT:
            how = already ? "w+b" : NULL;
            did = XENOLITH_WAS_EMPTIED;
            break;
        case XENOLITH_REPLACE_IT:
        case XENOLITH_EMPTY_OR_MAKE_IT:
            how = "w+b";
            did = already ? XENOLITH_WAS_EMPTIED : XENOLITH_WAS_MADE;
            break;
        default:
            how = NULL;
            break;
        }
        if (how != NULL && !already) {
            make_the_way_to(path);
        }
    } else if (!already) {
        how = NULL;
    }

    FILE *opened = how != NULL ? fopen(path, how) : NULL;
    if (opened == NULL) {
        /* Said once, with the name, because a title that cannot find its data
         * looks exactly like one that found it and read nothing. */
        static const char *said[16];
        static unsigned told;
        int before = 0;
        pthread_mutex_lock(&reporting);
        for (unsigned at = 0; at < told; at++) {
            if (strcmp(said[at], path) == 0) {
                before = 1;
            }
        }
        if (!before) {
            if (told < sizeof said / sizeof *said) {
                said[told++] = strdup(path);
            }
            fprintf(stderr, "xenolith: %s is not there\n", path);
            fflush(stderr);
        }
        pthread_mutex_unlock(&reporting);
        report_io(base, status_at, XENOLITH_STATUS_NO_SUCH_FILE, 0);
        ctx->r[3] = XENOLITH_STATUS_NO_SUCH_FILE;
        return;
    }

    uint32_t handle = take_object(OBJECT_FILE);
    struct object *made = object_of(handle);
    if (made == NULL) {
        fclose(opened);
        report_io(base, status_at, XENOLITH_STATUS_NO_MEMORY, 0);
        ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
        return;
    }
    made->file = opened;
    if (handle_at != 0) {
        xenolith_store32(base, handle_at, handle);
    }
    report_io(base, status_at, XENOLITH_STATUS_SUCCESS, did);
    ctx->r[3] = XENOLITH_STATUS_SUCCESS;
}

/* Reading from a file a title has open.
 *
 * What a title handed over to be told when the work was done is signalled
 * before this returns, because reading here is done by the time it returns.
 * Leaving it alone because the reading was not really asynchronous would leave
 * whoever waits on it waiting for a read that already happened.
 */
static void read_file(xenolith_context *ctx, uint8_t *base) {
    struct object *found = object_of((uint32_t)ctx->r[3]);
    struct object *done = object_of((uint32_t)ctx->r[4]);
    uint32_t status_at = (uint32_t)ctx->r[7];
    uint32_t into = (uint32_t)ctx->r[8];
    uint32_t wanted = (uint32_t)ctx->r[9];
    uint32_t offset_at = (uint32_t)ctx->r[10];

    if (found == NULL || found->file == NULL) {
        report_io(base, status_at, XENOLITH_STATUS_NOT_FOUND, 0);
        if (done != NULL) {
            signal_object(done, 1);
        }
        ctx->r[3] = XENOLITH_STATUS_NOT_FOUND;
        return;
    }

    if (offset_at != 0) {
        found->at = (long long)xenolith_load64(base, offset_at);
    }
    if (fseek(found->file, (long)found->at, SEEK_SET) != 0) {
        report_io(base, status_at, XENOLITH_STATUS_END_OF_FILE, 0);
        if (done != NULL) {
            signal_object(done, 1);
        }
        ctx->r[3] = XENOLITH_STATUS_END_OF_FILE;
        return;
    }

    /* Read into the guest's memory directly. Every byte of it is mapped and a
     * file holds bytes rather than words, so there is no order to correct. */
    size_t got = fread(base + into, 1, wanted, found->file);
    found->at += (long long)got;

    uint32_t status =
        got == 0 && wanted != 0 ? XENOLITH_STATUS_END_OF_FILE : XENOLITH_STATUS_SUCCESS;
    report_io(base, status_at, status, (uint32_t)got);
    if (done != NULL) {
        signal_object(done, 1);
    }
    ctx->r[3] = status;
}

/* Writing to one, at where it is up to or at a place a title names.
 *
 * The same shape as reading, and the same reason for signalling what a title
 * handed over the moment it returns: the writing is finished by then.
 */
static void write_file(xenolith_context *ctx, uint8_t *base) {
    struct object *found = object_of((uint32_t)ctx->r[3]);
    struct object *done = object_of((uint32_t)ctx->r[4]);
    uint32_t status_at = (uint32_t)ctx->r[7];
    uint32_t from = (uint32_t)ctx->r[8];
    uint32_t wanted = (uint32_t)ctx->r[9];
    uint32_t offset_at = (uint32_t)ctx->r[10];

    if (found == NULL || found->file == NULL) {
        report_io(base, status_at, XENOLITH_STATUS_NOT_FOUND, 0);
        if (done != NULL) {
            signal_object(done, 1);
        }
        ctx->r[3] = XENOLITH_STATUS_NOT_FOUND;
        return;
    }

    /* A title asking to write where it left off names the end of the file by a
     * number the console keeps for it rather than by a place. */
    if (offset_at != 0) {
        uint64_t said = xenolith_load64(base, offset_at);
        if (said == XENOLITH_AT_THE_END) {
            fseek(found->file, 0, SEEK_END);
            found->at = ftell(found->file);
        } else {
            found->at = (long long)said;
        }
    }
    if (fseek(found->file, (long)found->at, SEEK_SET) != 0) {
        report_io(base, status_at, XENOLITH_STATUS_NO_SUCH_FILE, 0);
        if (done != NULL) {
            signal_object(done, 1);
        }
        ctx->r[3] = XENOLITH_STATUS_NO_SUCH_FILE;
        return;
    }

    size_t put = fwrite(base + from, 1, wanted, found->file);
    found->at += (long long)put;

    uint32_t status = put == wanted ? XENOLITH_STATUS_SUCCESS : XENOLITH_STATUS_NO_MEMORY;
    report_io(base, status_at, status, (uint32_t)put);
    if (done != NULL) {
        signal_object(done, 1);
    }
    ctx->r[3] = status;
}

/* How long a file is, and how far into it reading is up to.
 *
 * A title asks before it reads, to know how much room to make. Answering with
 * nothing makes every file look empty.
 *
 * Three kinds of question are answered, each in the kernel's own layout: the
 * standard one, which holds the length at its second word pair; where reading is
 * up to; and the one asked of a file just opened, which holds four times, the
 * length twice and what the file is. The times are nought because nothing here
 * keeps them. Anything else is refused rather than answered with a success that
 * wrote nothing.
 */
static void query_file(xenolith_context *ctx, uint8_t *base) {
    struct object *found = object_of((uint32_t)ctx->r[3]);
    uint32_t status_at = (uint32_t)ctx->r[4];
    uint32_t into = (uint32_t)ctx->r[5];
    uint32_t room = (uint32_t)ctx->r[6];
    uint32_t kind = (uint32_t)ctx->r[7];

    if (found == NULL || found->file == NULL) {
        report_io(base, status_at, XENOLITH_STATUS_NOT_FOUND, 0);
        ctx->r[3] = XENOLITH_STATUS_NOT_FOUND;
        return;
    }

    long here = ftell(found->file);
    fseek(found->file, 0, SEEK_END);
    long ends = ftell(found->file);
    fseek(found->file, here, SEEK_SET);
    uint64_t length = (uint64_t)(ends < 0 ? 0 : ends);

    uint32_t said;
    if (kind == XENOLITH_FILE_STANDARD && room >= 24) {
        xenolith_store64(base, into, length);
        xenolith_store64(base, into + 8, length);
        xenolith_store32(base, into + 16, 1);
        xenolith_store32(base, into + 20, 0);
        said = 24;
    } else if (kind == XENOLITH_FILE_POSITION && room >= 8) {
        xenolith_store64(base, into, (uint64_t)found->at);
        said = 8;
    } else if (kind == XENOLITH_FILE_NETWORK_OPEN && room >= 56) {
        for (uint32_t time = 0; time < 4; time++) {
            xenolith_store64(base, into + time * 8, 0);
        }
        xenolith_store64(base, into + 32, length);
        xenolith_store64(base, into + 40, length);
        xenolith_store32(base, into + 48, XENOLITH_FILE_ATTRIBUTE_NORMAL);
        xenolith_store32(base, into + 52, 0);
        said = 56;
    } else {
        report_io(base, status_at, XENOLITH_STATUS_INVALID_INFO_CLASS, 0);
        ctx->r[3] = XENOLITH_STATUS_INVALID_INFO_CLASS;
        return;
    }

    report_io(base, status_at, XENOLITH_STATUS_SUCCESS, said);
    ctx->r[3] = XENOLITH_STATUS_SUCCESS;
}

/* Moving where reading a file is up to.
 *
 * Where reading is up to is the one thing a title sets. A read that names no
 * place starts from there, so a title that seeks and then reads without naming
 * one reads from the front of the file when the seek is not kept. Anything else
 * is refused rather than reported as done.
 */
static void set_file(xenolith_context *ctx, uint8_t *base) {
    struct object *found = object_of((uint32_t)ctx->r[3]);
    uint32_t status_at = (uint32_t)ctx->r[4];
    uint32_t from = (uint32_t)ctx->r[5];
    uint32_t room = (uint32_t)ctx->r[6];
    uint32_t kind = (uint32_t)ctx->r[7];

    if (found == NULL || found->file == NULL) {
        report_io(base, status_at, XENOLITH_STATUS_NOT_FOUND, 0);
        ctx->r[3] = XENOLITH_STATUS_NOT_FOUND;
        return;
    }
    if (kind != XENOLITH_FILE_POSITION || room < 8) {
        report_io(base, status_at, XENOLITH_STATUS_INVALID_INFO_CLASS, 0);
        ctx->r[3] = XENOLITH_STATUS_INVALID_INFO_CLASS;
        return;
    }

    found->at = (long long)xenolith_load64(base, from);
    report_io(base, status_at, XENOLITH_STATUS_SUCCESS, 0);
    ctx->r[3] = XENOLITH_STATUS_SUCCESS;
}

/* Reading what is in a directory, one at a time.
 *
 * A title asks with a handle it opened the directory with, somewhere to put the
 * answer and how much room there is, and says whether to start over. What goes
 * back is the console's own shape: a run of numbers about the thing, then its
 * name in plain bytes. Nought in the first word says this is the last.
 *
 * A title reads what it has saved this way, so without it a save it made cannot
 * be read back.
 */
static void query_directory(xenolith_context *ctx, uint8_t *base) {
    struct object *found = object_of((uint32_t)ctx->r[3]);
    uint32_t status_at = (uint32_t)ctx->r[7];
    uint32_t into = (uint32_t)ctx->r[8];
    uint32_t room = (uint32_t)ctx->r[9];
    uint32_t over_again =
        xenolith_load32(base, (uint32_t)ctx->r[1] + XENOLITH_NINTH_ARGUMENT);

    if (found == NULL || found->directory == NULL || into == 0 ||
        room < XENOLITH_WHAT_ONE_THING_TAKES) {
        report_io(base, status_at, XENOLITH_STATUS_NO_SUCH_FILE, 0);
        ctx->r[3] = XENOLITH_STATUS_NO_SUCH_FILE;
        return;
    }
    if (over_again != 0) {
        found->gone_past = 0;
    }

    DIR *held = opendir(found->directory);
    if (held == NULL) {
        report_io(base, status_at, XENOLITH_STATUS_NO_SUCH_FILE, 0);
        ctx->r[3] = XENOLITH_STATUS_NO_SUCH_FILE;
        return;
    }

    char name[256];
    int found_one = 0;
    uint32_t past = 0;
    for (struct dirent *one = readdir(held); one != NULL; one = readdir(held)) {
        if (strcmp(one->d_name, ".") == 0 || strcmp(one->d_name, "..") == 0) {
            continue;
        }
        if (strlen(one->d_name) >= sizeof name) {
            continue;
        }
        if (past++ < found->gone_past) {
            continue;
        }
        memcpy(name, one->d_name, strlen(one->d_name) + 1u);
        found_one = 1;
        break;
    }
    closedir(held);

    if (!found_one) {
        report_io(base, status_at, XENOLITH_NO_MORE_THINGS, 0);
        ctx->r[3] = XENOLITH_NO_MORE_THINGS;
        return;
    }

    char at[4608];
    struct stat about;
    int knows = snprintf(at, sizeof at, "%s/%s", found->directory, name) < (int)sizeof at &&
                stat(at, &about) == 0;
    uint32_t how_long = (uint32_t)strlen(name);
    if (XENOLITH_WHAT_ONE_THING_TAKES + how_long > room) {
        how_long = room - XENOLITH_WHAT_ONE_THING_TAKES;
    }

    for (uint32_t word = 0; word < XENOLITH_WHAT_ONE_THING_TAKES; word += 4u) {
        xenolith_store32(base, into + word, 0);
    }
    xenolith_store32(base, into + 0x28u, 0);
    xenolith_store32(base, into + 0x2cu,
                     knows && !S_ISDIR(about.st_mode) ? (uint32_t)about.st_size : 0u);
    xenolith_store32(base, into + 0x38u,
                     knows && S_ISDIR(about.st_mode) ? XENOLITH_IT_IS_A_DIRECTORY
                                                     : XENOLITH_IT_IS_A_FILE);
    xenolith_store32(base, into + 0x3cu, how_long);
    for (uint32_t letter = 0; letter < how_long; letter++) {
        xenolith_store8(base, into + XENOLITH_WHAT_ONE_THING_TAKES + letter,
                        (uint8_t)name[letter]);
    }

    found->gone_past++;
    report_io(base, status_at, XENOLITH_STATUS_SUCCESS,
              XENOLITH_WHAT_ONE_THING_TAKES + how_long);
    ctx->r[3] = XENOLITH_STATUS_SUCCESS;
}

/* What a request handed over to be finished later is answered with, what one
 * finished without trouble says, and what one given too little room says. */
#define XENOLITH_ERROR_IO_PENDING 997u
#define XENOLITH_ERROR_INSUFFICIENT_BUFFER 122u
#define XENOLITH_ERROR_INVALID_PARAMETER 87u

/* Finishes a request a title handed over a block to be told in.
 *
 * A title asking for something that may take a while is answered that it is
 * under way, and waits on the event in the block or looks at the block's first
 * word until it changes. Everything here is done by the time it returns, so the
 * block is filled in at once: what the request came to, how much it produced,
 * the same result in the form a title reads as an extended error, and the event
 * set. */
static uint32_t finish_at_once(uint8_t *base, uint32_t block, uint32_t result,
                               uint32_t length) {
    xenolith_store32(base, block, result);
    xenolith_store32(base, block + 4, length);
    xenolith_store32(base, block + 24, result == 0 ? 0 : 0x80070000u | result);
    struct object *event = object_of(xenolith_load32(base, block + 12));
    if (event != NULL) {
        signal_object(event, 1);
    }
    return XENOLITH_ERROR_IO_PENDING;
}

/* Whether a slot holds the profile, saying once that it is assumed. */
static int is_the_profile(uint32_t user) {
    assumed("one profile signed in without the network, in the first slot",
            "who was signed in to a console is not a property of the title, and a "
            "title asks for a profile to save to before it starts");
    return user == 0;
}

/* Asking about the profile in a slot: whether one is signed in, who it is, its
 * name, and all of that at once. A slot without one says so. */
static void say_who_is_signed_in(xenolith_context *ctx, uint8_t *base, uint32_t ordinal) {
    uint32_t user = (uint32_t)ctx->r[3];
    if (ordinal == 528) {
        ctx->r[3] = is_the_profile(user) ? XENOLITH_SIGNED_IN_LOCALLY : 0;
        return;
    }
    if (!is_the_profile(user)) {
        ctx->r[3] = XENOLITH_ERROR_NO_SUCH_USER;
        return;
    }

    const char *name = XENOLITH_PROFILE_NAME;
    if (ordinal == 522) {
        uint32_t into = (uint32_t)ctx->r[5];
        if (into != 0) {
            xenolith_store64(base, into, XENOLITH_PROFILE_XUID);
        }
    } else if (ordinal == 526) {
        uint32_t into = (uint32_t)ctx->r[4];
        uint32_t room = (uint32_t)ctx->r[5];
        for (uint32_t at = 0; into != 0 && at < room; at++) {
            char letter = at < strlen(name) ? name[at] : '\0';
            xenolith_store8(base, into + at, (uint8_t)letter);
            if (letter == '\0') {
                break;
            }
        }
    } else {
        /* The whole of it: the identity, flags saying nothing more, the state,
         * no guest number, no sponsor, and the name in sixteen bytes. */
        uint32_t into = (uint32_t)ctx->r[5];
        if (into != 0) {
            xenolith_store64(base, into, XENOLITH_PROFILE_XUID);
            xenolith_store32(base, into + 8, 0);
            xenolith_store32(base, into + 12, XENOLITH_SIGNED_IN_LOCALLY);
            xenolith_store32(base, into + 16, 0);
            xenolith_store32(base, into + 20, 0);
            for (uint32_t at = 0; at < 16; at++) {
                char letter = at < strlen(name) ? name[at] : '\0';
                xenolith_store8(base, into + 24 + at, (uint8_t)letter);
            }
        }
    }
    ctx->r[3] = 0;
}

/* The system's music player, and asking it who controls what plays. */
#define XENOLITH_MUSIC_PLAYER 0xfau
#define XENOLITH_SET_PLAYBACK_CONTROLLER 0x0007001au

/* The system's program for what a player is doing, and telling it a context
 * or a property of the player's game. */
#define XENOLITH_PLAYER_STATUS 0xfbu
#define XENOLITH_SET_CONTEXT 0x000b0006u
#define XENOLITH_SET_PROPERTY 0x000b0007u

/* Answers a message a title sends to one of the system's own programs, with
 * the status it comes to.
 *
 * A message names the program and what is asked of it, and hands over a block
 * of whatever that needs. Only the ones a title was seen to send are answered.
 * Any other stops the run naming both numbers, as an entry point nothing
 * answers does: answering a message with success when nothing was done is a
 * lie a title believes. */
static uint32_t answer_a_message(uint32_t program, uint32_t message) {
    if (program == XENOLITH_MUSIC_PLAYER && message == XENOLITH_SET_PLAYBACK_CONTROLLER) {
        /* Who decides what music plays: the title or the person holding the
         * controller. Nothing here plays the person's own music, so the title
         * is left in control, which is what it asked for. */
        return XENOLITH_STATUS_SUCCESS;
    }
    if (program == XENOLITH_PLAYER_STATUS
        && (message == XENOLITH_SET_CONTEXT || message == XENOLITH_SET_PROPERTY)) {
        /* What the player is doing, which the console shows to their friends.
         * There is no one to show it to, and nothing reads it back, so being
         * told is all there is to it. */
        return XENOLITH_STATUS_SUCCESS;
    }
    fprintf(stderr,
            "xenolith: message %#010x to the system's program %#x is not answered\n",
            message, program);
    stop_the_run(3);
}

/* The number of the one device a title can save to. */
#define XENOLITH_THE_ONE_DEVICE 1u

/* What reading a list that has run out reports, and the most one item of a
 * list a title asks for takes. */
#define XENOLITH_ERROR_NO_MORE_FILES 18u
#define XENOLITH_LARGEST_ITEM 0x200u

/* Makes a list with nothing in it, writes the handle it is read through and
 * how much room a reading of `count` items takes, and says how that went. */
static uint32_t make_an_empty_list(uint8_t *base, uint32_t count, uint32_t room_at,
                                   uint32_t handle_at) {
    uint32_t handle = take_object(OBJECT_ENUMERATOR);
    if (handle == 0) {
        return XENOLITH_ERROR_INVALID_PARAMETER;
    }
    if (room_at != 0) {
        xenolith_store32(base, room_at, count * XENOLITH_LARGEST_ITEM);
    }
    if (handle_at != 0) {
        xenolith_store32(base, handle_at, handle);
    }
    return 0;
}

/* How much room one setting takes where profile settings are written. */
#define XENOLITH_ONE_SETTING 40u

/* Reading profile settings, of which none have ever been written here.
 *
 * Asked first with nowhere to put them, this says how much room they take. Asked
 * again with the room, it writes how many there are and where the list is, then
 * one block for each setting the title named: that it has no value, whose it is,
 * which setting it is, and the kind of value that setting would hold. A title
 * told a setting has no value uses its own default, which is what it gets from
 * a profile that has never saved one. */
static void read_profile_settings(xenolith_context *ctx, uint8_t *base) {
    uint32_t user = (uint32_t)ctx->r[4];
    uint32_t people = (uint32_t)ctx->r[5];
    uint32_t xuids = (uint32_t)ctx->r[6];
    uint32_t count = (uint32_t)ctx->r[7];
    uint32_t ids = (uint32_t)ctx->r[8];
    uint32_t room_at = (uint32_t)ctx->r[9];
    uint32_t into = (uint32_t)ctx->r[10];
    uint32_t block = xenolith_load32(base, (uint32_t)ctx->r[1] + XENOLITH_NINTH_ARGUMENT);

    uint32_t whose = people != 0 ? people : 1;
    if (room_at == 0 || count == 0 || whose > 16 || count > 64) {
        ctx->r[3] = XENOLITH_ERROR_INVALID_PARAMETER;
        return;
    }
    uint32_t needed = 8 + whose * count * XENOLITH_ONE_SETTING;
    if (into == 0 || xenolith_load32(base, room_at) < needed) {
        xenolith_store32(base, room_at, needed);
        ctx->r[3] = XENOLITH_ERROR_INSUFFICIENT_BUFFER;
        return;
    }

    uint32_t list = into + 8;
    xenolith_store32(base, into, whose * count);
    xenolith_store32(base, into + 4, list);
    for (uint32_t person = 0; person < whose; person++) {
        for (uint32_t k = 0; k < count; k++) {
            uint32_t one = list + (person * count + k) * XENOLITH_ONE_SETTING;
            uint32_t id = xenolith_load32(base, ids + k * 4);
            for (uint32_t word = 0; word < XENOLITH_ONE_SETTING; word += 4) {
                xenolith_store32(base, one + word, 0);
            }
            if (people != 0) {
                xenolith_store64(base, one + 8, xenolith_load64(base, xuids + person * 8));
            } else {
                xenolith_store32(base, one + 8, user);
            }
            xenolith_store32(base, one + 16, id);
            xenolith_store8(base, one + 24, (uint8_t)(id >> 28));
        }
    }
    ctx->r[3] = block != 0 ? finish_at_once(base, block, 0, 0) : 0;
}

static int serviced_by_the_kernel(xenolith_context *ctx, uint8_t *base, uint32_t ordinal);

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
        case 3:
        case 4:
        case 5:
        case 6:
        case 7:
        case 8:
        case 9:
        case 11:
        case 12:
        case 13:
        case 14:
        case 15:
        case 18:
        case 20:
        case 22:
        case 24:
            /* Making a socket and everything done with one, on the network
             * that did not start. Each comes back with the value that says it
             * failed, which for making one is also the value that says no
             * socket was made, and asking why says the network is down. A
             * title that makes one and uses it without looking is told the
             * same at every step rather than handed a socket that is not. */
            ctx->r[3] = (uint64_t)(int64_t)-1;
            return 1;
        case 34:
            /* Whether a socket is among those a wait found ready. None is. */
            ctx->r[3] = 0;
            return 1;
        case 2:
        case 52:
            /* Stopping networking, which never started. */
            ctx->r[3] = 0;
            return 1;
        case 27:
            ctx->r[3] = XENOLITH_NETWORK_IS_DOWN;
            return 1;
        case 400: {
            /* What a controller can do.
             *
             * The title names a port and asks what it is. What is answered is a
             * pad with the two sticks, the two triggers and the buttons a
             * console pad has, because that is what the desktop's keyboard is
             * standing in for. The structure is the console's own: a size, then
             * a bit per capability. */
            uint32_t into = (uint32_t)ctx->r[5];
            uint32_t port = (uint32_t)ctx->r[3];
            if (port > 3u) {
                ctx->r[3] = XENOLITH_STATUS_NOT_FOUND;
                return 1;
            }
            if (into != 0) {
                xenolith_store32(base, into, 4);
                /* Everything a console pad has: the four directions, four face
                 * buttons, two shoulder buttons, two triggers, the two sticks
                 * pressed, and the two analogue sticks. */
                xenolith_store32(base, into + 4, 0x0000000fu);
                xenolith_store32(base, into + 8, 0x0000ffffu);
            }
            ctx->r[3] = XENOLITH_STATUS_SUCCESS;
            return 1;
        }
        case 401: {
            /* What a controller is holding now.
             *
             * The console's structure is a packet number, the buttons, the two
             * triggers and the two sticks. What is written is what the host is
             * holding, read the same way a window's keys are.
             *
             * The structure is the third argument. The second is a flag word
             * the title's own wrapper puts there, and written through as an
             * address it sends what is held to the bottom of memory, where the
             * title never looks. */
            uint32_t port = (uint32_t)ctx->r[3];
            uint32_t into = (uint32_t)ctx->r[5];
            if (port > 3u) {
                ctx->r[3] = XENOLITH_STATUS_NOT_FOUND;
                return 1;
            }
            uint16_t buttons = 0;
            int16_t thumbs[4] = {0, 0, 0, 0};
            if (into != 0) {
                xenolith_what_is_held(&buttons, thumbs);
                xenolith_store32(base, into, 1);
                xenolith_store16(base, into + 4, buttons);
                xenolith_store8(base, into + 6, (uint8_t)(thumbs[2] >> 8));
                xenolith_store8(base, into + 7, (uint8_t)(thumbs[3] >> 8));
                xenolith_store16(base, into + 8, (uint16_t)thumbs[0]);
                xenolith_store16(base, into + 10, (uint16_t)thumbs[1]);
            }
            ctx->r[3] = XENOLITH_STATUS_SUCCESS;
            return 1;
        }
        case 402:
            /* Making a controller rumble. Nothing here rumbles, and a pad that
             * cannot is still a pad, so a port that answers the questions above
             * says this was done. */
            ctx->r[3] = (uint32_t)ctx->r[3] > 3u ? XENOLITH_STATUS_NOT_FOUND : 0;
            return 1;
        case 408: {
            /* The next thing a controller did, if anything did.
             *
             * A title that polls this in a loop is asking for events, and what
             * it is told when there are none is that there are none rather than
             * that it failed. Reporting nothing as an error sends a title down
             * its error path on every frame it is idle, which is most of them.
             *
             * The event structure is the console's: the packet number, the
             * buttons, the kind, and the repeats. Nothing here raises an event,
             * so the count of them is nought. */
            uint32_t into = (uint32_t)ctx->r[4];
            uint32_t count_at = (uint32_t)ctx->r[3];
            if (into != 0) {
                xenolith_store32(base, into, 1);
                xenolith_store32(base, into + 4, 0);
                xenolith_store32(base, into + 8, 0);
                xenolith_store32(base, into + 12, 0);
            }
            if (count_at != 0) {
                xenolith_store32(base, count_at, 0);
            }
            ctx->r[3] = XENOLITH_STATUS_SUCCESS;
            return 1;
        }
        case 651: {
            /* Whether a listener has anything to say, and if it has, what and
             * about what. A title polls this every frame and is told either
             * the next thing, or that there is nothing this frame, which is
             * what an idle console says too. */
            uint32_t said = 0;
            uint32_t about = 0;
            int heard = hear_from((uint32_t)ctx->r[3], (uint32_t)ctx->r[4], &said, &about);
            uint32_t said_at = (uint32_t)ctx->r[5];
            uint32_t about_at = (uint32_t)ctx->r[6];
            if (said_at != 0) {
                xenolith_store32(base, said_at, said);
            }
            if (heard && about_at != 0) {
                xenolith_store32(base, about_at, about);
            }
            ctx->r[3] = (uint64_t)heard;
            return 1;
        }
        case 650: {
            /* Registering to be told about system notifications, of the kinds
             * the first argument names a bit apiece. */
            ctx->r[3] = take_a_listener(ctx->r[3]);
            return 1;
        }
        case 642:
            ctx->r[3] = XENOLITH_SYSTEM_VERSION;
            return 1;
        case 537:
            read_profile_settings(ctx, base);
            return 1;
        case 750: {
            /* A list of the profile's achievements, to be read a few at a
             * time. The profile here has earned none and nothing here reads
             * what the title says it can award, so the list is empty. What
             * comes back is a handle to read it through and how much room the
             * title should make for a reading: the sixth argument's count of
             * items at the most an item takes. */
            uint32_t room_at = (uint32_t)ctx->r[9];
            uint32_t handle_at = (uint32_t)ctx->r[10];
            ctx->r[3] = make_an_empty_list(base, (uint32_t)ctx->r[8], room_at, handle_at);
            return 1;
        }
        case 604: {
            /* A list of what has been saved to a device, read the same way.
             * Nothing has been saved, so it is empty too. */
            uint32_t room_at = (uint32_t)ctx->r[6];
            uint32_t handle_at = (uint32_t)ctx->r[7];
            ctx->r[3] = make_an_empty_list(base, (uint32_t)ctx->r[5], room_at, handle_at);
            return 1;
        }
        case 715: {
            /* Asking the player which device to save to. There is one, so it
             * is chosen without asking: its number goes where the fifth
             * argument says, through the block in the sixth where there is
             * one.
             *
             * On a console this puts the system's own screen up and takes it
             * down again, and says so to everything listening, and a title
             * waits to hear the screen has gone before it reads the answer.
             * So that is said here too, as though it had been up and gone. */
            uint32_t chosen_at = (uint32_t)ctx->r[7];
            uint32_t block = (uint32_t)ctx->r[8];
            if (chosen_at != 0) {
                xenolith_store32(base, chosen_at, XENOLITH_THE_ONE_DEVICE);
            }
            tell_the_system_listeners(XENOLITH_SYSTEM_SCREENS, 1);
            tell_the_system_listeners(XENOLITH_SYSTEM_SCREENS, 0);
            ctx->r[3] = block != 0 ? finish_at_once(base, block, 0, 0) : 0;
            return 1;
        }
        case 592: {
            /* Reading the next few items of a list. Every list here is empty,
             * so the answer is that there are no more, with no items read,
             * through the block a title hands over where it hands one. */
            struct object *list = object_of((uint32_t)ctx->r[3]);
            uint32_t read_at = (uint32_t)ctx->r[7];
            uint32_t block = (uint32_t)ctx->r[8];
            uint32_t status = list != NULL && list->kind == OBJECT_ENUMERATOR
                                  ? XENOLITH_ERROR_NO_MORE_FILES
                                  : XENOLITH_ERROR_INVALID_PARAMETER;
            if (read_at != 0) {
                xenolith_store32(base, read_at, 0);
            }
            ctx->r[3] = block != 0 ? finish_at_once(base, block, status, 0) : status;
            return 1;
        }
        case 500:
            /* A message answered before the call returns. */
            ctx->r[3] = answer_a_message((uint32_t)ctx->r[3], (uint32_t)ctx->r[4]);
            return 1;
        case 503:
        case 508: {
            /* A message whose answer can be given later, through the block the
             * third argument names. Every answer here is given at once, so
             * where there is a block it is filled in and the title told the
             * request is under way, and where there is none the status is the
             * answer. */
            uint32_t block = (uint32_t)ctx->r[5];
            uint32_t status = answer_a_message((uint32_t)ctx->r[3], (uint32_t)ctx->r[4]);
            ctx->r[3] = block != 0 ? finish_at_once(base, block, status, 0) : status;
            return 1;
        }
        case 522:
        case 526:
        case 528:
        case 551:
            say_who_is_signed_in(ctx, base, ordinal);
            return 1;
        case 971:
            /* Which cable the console's video output is going through. The
             * title's startup asks, and refuses to start for four of the eight
             * answers it knows about, so this is a setting rather than a
             * formality.
             *
             * The answer is a component cable. That is a state a real console
             * can be in, it is the one that goes with the display this runtime
             * reports elsewhere, and it is one of the four the title accepts.
             * The console it was recorded from is not here and the container
             * does not carry what it was plugged into. */
            assumed("a console with a component video cable",
                    "what one was plugged into is not a property of the title "
                    "and is not in the container");
            ctx->r[3] = XENOLITH_AV_PACK_COMPONENT;
            return 1;
        case 652:
            /* Where on the screen the system's notices should appear. Nothing
             * here shows one, so there is nowhere to move them to. */
            return 1;
        case 972:
            /* Which region the console was sold in, which decides what a
             * title shows and in which order. */
            assumed("a console sold in North America",
                    "where a console was sold is not a property of the title and "
                    "is not in the container");
            ctx->r[3] = XENOLITH_REGION_NORTH_AMERICA;
            return 1;
        case 973:
            /* Which language the console was set to. */
            assumed("a console set to English",
                    "what a console was set to is not a property of the title and "
                    "is not in the container");
            ctx->r[3] = XENOLITH_LANGUAGE_ENGLISH;
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
    return serviced_by_the_kernel(ctx, base, ordinal);
}

/* What a title asks of the kernel itself.
 *
 * Kept apart from the rest because it is most of what the runtime answers, and
 * one function holding every library's answers is a function nobody can read to
 * find one of them.
 */
static int serviced_by_the_kernel(xenolith_context *ctx, uint8_t *base, uint32_t ordinal) {
    if (serviced_about_what_it_holds(ctx, base, ordinal)) {
        return 1;
    }
    if (serviced_about_memory_and_time(ctx, base, ordinal)) {
        return 1;
    }
    return serviced_about_what_it_sees(ctx, base, ordinal);
}

/* What a title asks of the kernel about the things it holds.
 *
 * Threads, objects, waits and the counters they read. */
static int serviced_about_what_it_holds(xenolith_context *ctx, uint8_t *base, uint32_t ordinal) {
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
    case 226: {
        /* Letting everyone waiting on an object through at once and leaving it
         * unsignalled behind them. Anyone who arrives afterwards waits, which
         * is what tells this apart from simply signalling it. */
        struct object *found = object_of((uint32_t)ctx->r[3]);
        uint32_t before_at = (uint32_t)ctx->r[4];
        if (found != NULL) {
            if (before_at != 0) {
                pthread_mutex_lock(&found->lock);
                xenolith_store32(base, before_at, found->signalled ? 1u : 0u);
                pthread_mutex_unlock(&found->lock);
            }
            /* Everyone waiting now, rather than one of them. Setting it and
             * clearing it again lets a single waiter through, because a set
             * that finds threads waiting releases one, and the clear takes the
             * mark back before anybody else looks. */
            pthread_mutex_lock(&found->lock);
            found->releases = found->blocked;
            found->signalled = 0;
            pthread_cond_broadcast(&found->changed);
            pthread_mutex_unlock(&found->lock);
            say_something_changed();
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
        /* Letting go of a lock, with what it was held as before where a title
         * asks. */
        struct object *found = object_of((uint32_t)ctx->r[3]);
        uint32_t before_at = (uint32_t)ctx->r[4];
        long was = 0;
        if (found != NULL && found->kind == OBJECT_MUTANT) {
            was = release_lock(found);
        } else if (found != NULL) {
            signal_object(found, 1);
        }
        if (before_at != 0) {
            xenolith_store32(base, before_at, (uint32_t)was);
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
    case 25:
        /* A thread ending itself. It does not come back from this, so the
         * host thread ends here too, after saying it has finished. On the
         * thread that entered the title the rest of the threads carry on, as
         * they do when a console's first thread ends. */
        finish_thread(this_thread);
        pthread_exit(NULL);
    case 251: {
        /* Releasing one object and waiting on another, which a title uses to
         * hand work to a thread and wait for the answer without missing the
         * reply. The console does the two with nothing in between. Here they
         * are two steps, which differs only for a title that depends on no
         * other thread being let go between them. */
        struct object *released = object_of((uint32_t)ctx->r[3]);
        struct object *waited = object_of((uint32_t)ctx->r[4]);
        if (released != NULL && released->kind == OBJECT_MUTANT) {
            release_lock(released);
        } else if (released != NULL && released->kind == OBJECT_SEMAPHORE) {
            pthread_mutex_lock(&released->lock);
            released->count++;
            pthread_mutex_unlock(&released->lock);
            signal_object(released, 1);
        } else if (released != NULL) {
            signal_object(released, 1);
        }
        if (waited == NULL) {
            ctx->r[3] = XENOLITH_STATUS_SUCCESS;
            return 1;
        }
        /* The length of the wait is the fifth argument. The two before it are
         * the mode and whether a wait may be cut short, and read as the
         * length either is taken for an address, which gives up at once and
         * sends the title round its loop as fast as a host can carry it. */
        struct timespec until;
        int limited = give_up_at(base, (uint32_t)ctx->r[7], &until);
        ctx->r[3] = wait_until(waited, limited ? &until : NULL);
        return 1;
    }
    case 116: {
        /* Making a semaphore. The console keeps one in a structure the title
         * owns, and nothing here uses that structure: what is kept is the
         * runtime's own object, named by the handle the title hands over. So
         * the structure is cleared and the handle stands for it.
         *
         * The count and the limit are what the title asked for and are not
         * needed, because nothing here is limited: a semaphore released more
         * times than its limit says would be a fault on the console, and
         * counting releases is not something a title here depends on. */
        uint32_t at = (uint32_t)ctx->r[3];
        uint32_t count = (uint32_t)ctx->r[4];
        uint32_t limit = (uint32_t)ctx->r[5];
        (void)count;
        (void)limit;
        if (at != 0) {
            for (uint32_t word = 0; word < 8; word++) {
                xenolith_store32(base, at + word * 4, 0);
            }
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 133:
        /* Raising the interrupt level to the one that defers work. Nothing here
         * interrupts a thread, so there is nothing to raise and the level the
         * title was at is what it is told, which is what the console returns. */
        ctx->r[3] = 0;
        return 1;
    case 179:
        /* Putting the interrupt level back. Nothing was raised, so there is
         * nothing to put back. */
        return 1;
    case 146:
        /* Resuming a thread. Threads here run from the moment they are made, so
         * one that is resumed is already running and there is nothing to do. */
        ctx->r[3] = 0;
        return 1;
    case 252:
        /* Suspending a thread.
         *
         * Answering without stopping anything is a lie a title believes: it
         * suspends a worker and carries on expecting that worker not to run,
         * and two threads doing the same work is a fault that looks like a
         * data race. There is no way to stop a thread this runtime made without
         * a signal for it to look at, and adding one is a change to how threads
         * are run rather than to this switch. So this says it cannot, which
         * stops the run where the title wanted it rather than letting it go on
         * with a worker it thinks is stopped. */
        ctx->r[3] = XENOLITH_STATUS_NOT_FOUND;
        return 1;
    case 90: {
        /* Waiting for a stretch of time, or until an object is signalled.
         *
         * A title uses this to pace itself, so it has to actually wait rather
         * than return: a run that returned at once would spin through a title's
         * frame loop as fast as the host allows and every timing the title
         * measured would be wrong. The length is in hundreds of nanoseconds and
         * negative for a stretch from now. */
        uint32_t how = (uint32_t)ctx->r[3];
        struct timespec until;
        /* The stretch is the fifth argument. It was read from the fourth, which
         * is why a wait that was given a length returned at once: the fourth
         * holds the reason for waiting, which is not a length and was never
         * meant to be one. A title pacing itself then spun as fast as the host
         * allows, and one that polls a worker through this spun with it. */
        int limited = give_up_at(base, (uint32_t)ctx->r[5], &until);
        if (!limited) {
            ctx->r[3] = XENOLITH_STATUS_SUCCESS;
            return 1;
        }
        /* Only the wait for a stretch is honoured. A wait for a moment on the
         * console's clock is a title naming a time, and what it means is the
         * same wait, so both end at the same place. */
        while (clock_nanosleep(CLOCK_REALTIME, TIMER_ABSTIME, &until, NULL) == EINTR) {
        }
        /* The console reports a timeout when the wait ran out rather than an
         * object being signalled, and a title branches on the two differently. */
        ctx->r[3] = how == 1 ? XENOLITH_STATUS_TIMEOUT : XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 250:
        set_timer(ctx, base);
        return 1;
    case 205:
        cancel_timer(ctx, base);
        return 1;
    case 231: {
        /* Asking about a file by name, without opening it.
         *
         * The title passes a record holding a pointer to a counted string, and
         * somewhere to put what it asks about. What is answered is whether the
         * file is there: the console fills in a structure of sizes and times,
         * and nothing here has a shape for that to fill in, so a file that is
         * found is answered as found with the structure left as the title
         * cleared it, and one that is not is answered as not found.
         *
         * Answering "found" for everything would be worse than saying nothing: a
         * title that checks for a file and is told it is there goes on to open
         * it, and the open fails a moment later somewhere less informative. */
        uint32_t asked = (uint32_t)ctx->r[3];
        char path[4096];
        /* The name sits where host_path_of looks for it, four bytes into the
         * record, because the record's first word is its own length. */
        if (asked == 0 || !host_path_of(base, asked, path, sizeof path)) {
            ctx->r[3] = XENOLITH_STATUS_NO_SUCH_FILE;
            return 1;
        }
        ctx->r[3] = access(path, F_OK) == 0 ? XENOLITH_STATUS_SUCCESS
                                            : XENOLITH_STATUS_NO_SUCH_FILE;
        return 1;
    }
    case 309:
        /* Turning a status into the one a title's own error handling uses.
         *
         * The console's two numberings agree on success and disagree on
         * everything else, and a title compares the answer against values of its
         * own. Answering the status unchanged would be wrong for every failure
         * and right only for success, so success is the one case handled: a
         * failure is passed through as the error it already is rather than
         * mapped to a number nothing here has a table for. */
        ctx->r[3] = (uint32_t)ctx->r[3] == XENOLITH_STATUS_SUCCESS ? 0u : (uint32_t)ctx->r[3];
        return 1;
    case 215: {
        /* Making a timer, which is waited on like an event of the same two
         * kinds and is signalled when it falls due. A timer starts not
         * signalled, and there is no argument saying otherwise: what sits where
         * an event's starting state would be is left over from the caller. */
        uint32_t handle_at = (uint32_t)ctx->r[3];
        create_waitable(ctx, base, OBJECT_EVENT);
        struct object *made =
            handle_at != 0 ? object_of(xenolith_load32(base, handle_at)) : NULL;
        if (made != NULL) {
            made->signalled = 0;
        }
        return 1;
    }
    case 253: {
        /* Waiting on a handle. The length is the fourth argument, after the
         * mode and whether the wait may be cut short. */
        struct object *found = object_of((uint32_t)ctx->r[3]);
        if (found == NULL) {
            ctx->r[3] = XENOLITH_STATUS_SUCCESS;
            return 1;
        }
        struct timespec until;
        int limited = give_up_at(base, (uint32_t)ctx->r[6], &until);
        ctx->r[3] = wait_until(found, limited ? &until : NULL);
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
    default:
        return 0;
    }
}

/* What a title asks of the kernel about memory, time and the machine. */
/* Where a title's arguments to a formatting call come from, one at a time.
 *
 * Each takes a doubleword, and a word is the low half of it. Those passed in
 * registers come first, then the ones the caller left on its stack, which sit
 * a doubleword apart from the slot of the first argument. A call handed a list
 * already made walks that list the same way, from where it points. */
struct arguments {
    const xenolith_context *ctx;
    uint8_t *base;
    uint32_t next_register;
    uint32_t in_memory;
};

static uint64_t next_argument(struct arguments *from) {
    if (from->next_register != 0 && from->next_register <= 10u) {
        return from->ctx->r[from->next_register++];
    }
    uint64_t taken = xenolith_load64(from->base, from->in_memory);
    from->in_memory += 8u;
    return taken;
}

/* Formats text the way the console's own library does, into a title's memory.
 *
 * Each conversion is handed to the host's own formatting with its flags, width
 * and precision, and its argument taken from the title: a word for the whole
 * numbers and characters, a doubleword for the long ones and the fractions,
 * and an address for the strings, which are read out of the title's memory as
 * bytes or as wide characters. At most `room` bytes are written. What comes
 * back is how many were, or minus one where the text did not fit, in which case
 * nothing ends it, which is what the console's version does. */
static int32_t format_for_a_title(uint8_t *base, uint32_t into, uint32_t room, uint32_t format,
                                  struct arguments *from) {
    uint32_t written = 0;
    int fits = 1;
    char spec[32];
    char piece[512];

    for (uint32_t at = format;; at++) {
        char letter = (char)xenolith_load8(base, at);
        if (letter == '\0') {
            break;
        }
        size_t used = 0;
        if (letter != '%') {
            piece[0] = letter;
            used = 1;
        } else {
            size_t length = 0;
            spec[length++] = '%';
            int wide_size = 0;
            int wide_text = 0;
            for (;;) {
                char next = (char)xenolith_load8(base, ++at);
                if (next == '\0') {
                    at--;
                    break;
                }
                if (next == '*') {
                    int counted = (int)(int32_t)next_argument(from);
                    length += (size_t)snprintf(spec + length, sizeof spec - length, "%d", counted);
                    continue;
                }
                if (next == 'l') {
                    wide_size++;
                    wide_text = 1;
                    continue;
                }
                if (next == 'h' || next == 'z' || next == 't' || next == 'j') {
                    continue;
                }
                if (next == 'I' && xenolith_load8(base, at + 1) == '6'
                    && xenolith_load8(base, at + 2) == '4') {
                    at += 2;
                    wide_size = 2;
                    continue;
                }
                if (length + 3 < sizeof spec) {
                    spec[length++] = next;
                }
                if (strchr("diouxXcCsSpfFeEgGaA%n", next) != NULL) {
                    break;
                }
            }
            spec[length] = '\0';
            char kind = spec[length - 1];
            if (kind == '%') {
                piece[0] = '%';
                used = 1;
            } else if (kind == 'n') {
                next_argument(from);
            } else if (kind == 's' || kind == 'S') {
                uint32_t text = (uint32_t)next_argument(from);
                int wide = kind == 'S' || wide_text;
                char held[256];
                size_t taken = 0;
                for (; text != 0 && taken + 1 < sizeof held; taken++) {
                    uint32_t one = wide ? xenolith_load16(base, text + (uint32_t)taken * 2u)
                                        : xenolith_load8(base, text + (uint32_t)taken);
                    if (one == 0) {
                        break;
                    }
                    held[taken] = one < 0x80u ? (char)one : '?';
                }
                held[taken] = '\0';
                spec[length - 1] = 's';
                used = (size_t)snprintf(piece, sizeof piece, spec, text == 0 ? "(null)" : held);
            } else if (strchr("fFeEgGaA", kind) != NULL) {
                uint64_t bits = next_argument(from);
                double value;
                memcpy(&value, &bits, sizeof value);
                used = (size_t)snprintf(piece, sizeof piece, spec, value);
            } else if (kind == 'c' || kind == 'C') {
                spec[length - 1] = 'c';
                used = (size_t)snprintf(piece, sizeof piece, spec,
                                        (int)(uint8_t)next_argument(from));
            } else if (kind == 'p') {
                spec[length - 1] = 'X';
                used = (size_t)snprintf(piece, sizeof piece, spec,
                                        (unsigned)(uint32_t)next_argument(from));
            } else {
                uint64_t value = next_argument(from);
                char shown[40];
                memcpy(shown, spec, length - 1);
                int signed_kind = kind == 'd' || kind == 'i';
                if (wide_size >= 2) {
                    snprintf(shown + length - 1, sizeof shown - (length - 1), "ll%c", kind);
                    used = signed_kind ? (size_t)snprintf(piece, sizeof piece, shown,
                                                          (long long)value)
                                       : (size_t)snprintf(piece, sizeof piece, shown,
                                                          (unsigned long long)value);
                } else {
                    snprintf(shown + length - 1, sizeof shown - (length - 1), "%c", kind);
                    used = signed_kind
                               ? (size_t)snprintf(piece, sizeof piece, shown,
                                                  (int)(int32_t)value)
                               : (size_t)snprintf(piece, sizeof piece, shown,
                                                  (unsigned)(uint32_t)value);
                }
            }
        }
        if (used > sizeof piece - 1) {
            used = sizeof piece - 1;
        }
        for (size_t k = 0; k < used; k++) {
            if (written >= room) {
                fits = 0;
                break;
            }
            xenolith_store8(base, into + written++, (uint8_t)piece[k]);
        }
        if (!fits) {
            break;
        }
    }
    if (!fits) {
        return -1;
    }
    if (written < room) {
        xenolith_store8(base, into + written, 0);
    }
    return (int32_t)written;
}

static int serviced_about_memory_and_time(xenolith_context *ctx, uint8_t *base, uint32_t ordinal) {
    switch (ordinal) {
    case 315: {
        /* Formatting text with no limit on its length. The arguments after the
         * format are in the registers after it, and then on the caller's
         * stack, from the slot the ninth argument takes. */
        struct arguments from = {ctx, base, 5, (uint32_t)ctx->r[1] + XENOLITH_NINTH_SLOT};
        int32_t made = format_for_a_title(base, (uint32_t)ctx->r[3], 0x7fffffffu,
                                          (uint32_t)ctx->r[4], &from);
        ctx->r[3] = (uint64_t)(int64_t)made;
        return 1;
    }
    case 333: {
        /* Formatting text into a buffer of a given size, from a list of
         * arguments the caller has already laid out. */
        struct arguments from = {ctx, base, 0, (uint32_t)ctx->r[6]};
        int32_t made = format_for_a_title(base, (uint32_t)ctx->r[3], (uint32_t)ctx->r[4],
                                          (uint32_t)ctx->r[5], &from);
        ctx->r[3] = (uint64_t)(int64_t)made;
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
         * said outright rather than passed off as recovered.
         *
         * Two settings are answered with a value rather than nothing, and both
         * because a title's own startup says what it needs. One is asked for as
         * category two, setting two, and bits eight to fifteen of the answer
         * have to be three; a title that gets anything else shuts itself down.
         * The other is asked for as category three, setting ten, and needs bit
         * nine set. Neither number was invented here: each is what the check
         * after the call compares against, read out of the title's own code.
         * Every other setting is still answered with nothing, and a title that
         * needs one will say so by stopping. */
        uint32_t category = (uint32_t)ctx->r[3];
        uint32_t setting = (uint32_t)ctx->r[4];
        uint32_t into = (uint32_t)ctx->r[5];
        uint32_t room = (uint32_t)ctx->r[6];
        uint32_t needed_at = (uint32_t)ctx->r[7];

        uint32_t answer = 0;
        if (category == 2 && setting == 2) {
            /* Bits eight to fifteen are the ones the title masks off, and
             * three is the value it accepts. */
            answer = 0x00000300u;
            assumed("a console set to show 768 lines",
                    "the title refuses every other answer and the console it "
                    "was recorded from is not here");
        } else if (category == 3 && setting == 10) {
            answer = 0x00000200u;
            assumed("a console with the ninth bit of its display setting set",
                    "the title refuses to start without it and the console it "
                    "was recorded from is not here");
        } else {
            assumed("a console with no other settings turned on",
                    "what one was set to is not a property of the title and is "
                    "not in the container");
        }

        if (into != 0 && room >= 4) {
            xenolith_store32(base, into, answer);
        }
        if (needed_at != 0) {
            xenolith_store32(base, needed_at, 4);
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 189:
    case 220:
        /* Giving memory back. Nothing here takes it back, which is written
         * down where the memory is handed out. Reporting a failure would be a
         * different lie and a louder one. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 196:
        /* What a page allows. Every guest page is mapped readable and writable
         * before a title starts and nothing here changes that, so that is the
         * answer for any address, in the kernel's own flag for it. */
        ctx->r[3] = XENOLITH_PAGE_READ_WRITE;
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
         * where it wants nothing done. Doing nothing is the whole of it, and
         * saying it succeeded is the rest of it. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
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
         * it, so nothing is lost by it being nothing.
         *
         * A table with no room left says so rather than carrying on. Carrying on
         * is what lets two threads into the same critical section: each is told
         * it holds a lock that was never taken, and what the section was
         * protecting is now written by both at once. */
        pthread_mutex_t *lock = section_of((uint32_t)ctx->r[3]);
        if (lock == NULL) {
            say_a_report("no room left to name critical section %#x",
                         (unsigned)ctx->r[3]);
            ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
            return 1;
        }
        pthread_mutex_lock(lock);
        ctx->r[3] = 0;
        return 1;
    }
    case 180: {
        pthread_mutex_t *lock = section_of((uint32_t)ctx->r[3]);
        if (lock == NULL) {
            say_a_report("no room left to name critical section %#x",
                         (unsigned)ctx->r[3]);
            ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
            return 1;
        }
        pthread_mutex_unlock(lock);
        return 1;
    }
    case 77: {
        /* Taking a lock the console holds with interrupts already off. There
         * are no interrupts here and the threads are real, so what it protects
         * against is another thread, and a lock is what does that. */
        pthread_mutex_t *lock = section_of((uint32_t)ctx->r[3]);
        if (lock == NULL) {
            say_a_report("no room left to name critical section %#x",
                         (unsigned)ctx->r[3]);
            ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
            return 1;
        }
        pthread_mutex_lock(lock);
        return 1;
    }
    case 137: {
        pthread_mutex_t *lock = section_of((uint32_t)ctx->r[3]);
        if (lock == NULL) {
            say_a_report("no room left to name critical section %#x",
                         (unsigned)ctx->r[3]);
            ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
            return 1;
        }
        pthread_mutex_unlock(lock);
        return 1;
    }
    case 95:
    case 125:
        /* Holding off the kernel's own interruptions around a stretch of work.
         * Nothing here interrupts a thread that way, so there is nothing to
         * hold off and nothing to let back in. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 454:
        /* Whether the link to the display trained. There is no link, and a
         * title told it failed goes looking for a fault to report rather than
         * carrying on, so this reports the state a working console is in. */
        ctx->r[3] = 1;
        return 1;
    case 476:
        /* Shutting the graphics hardware down.
         *
         * Nothing here is hardware, so there is nothing to shut down, and a
         * title calling this is finishing with the display rather than starting
         * it. Answering failure would be the one wrong answer: a title that
         * shuts the display down and is told it could not goes looking for a
         * fault to report, which is not what happened. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    default:
        return 0;
    }
}

/* The code a C++ throw raises, and the word its record carries to say so. */
#define XENOLITH_CXX_EXCEPTION 0xe06d7363u
#define XENOLITH_CXX_MAGIC 0x19930520u

/* Names the C++ type a raised exception carries, when it carries one.
 *
 * A throw hands over three words after the record's count: a magic number, the
 * object thrown, and a description of its type. The description leads through a
 * list of the types the object can be caught as to each type's mangled name,
 * and the first one is the type that was thrown. */
static void say_what_was_thrown(uint8_t *base, uint32_t record) {
    if (record == 0 || xenolith_load32(base, record) != XENOLITH_CXX_EXCEPTION ||
        xenolith_load32(base, record + 16) < 3 ||
        xenolith_load32(base, record + 20) != XENOLITH_CXX_MAGIC) {
        return;
    }
    uint32_t object = xenolith_load32(base, record + 24);
    uint32_t described = xenolith_load32(base, record + 28);
    uint32_t types = described != 0 ? xenolith_load32(base, described + 12) : 0;
    uint32_t first = types != 0 && xenolith_load32(base, types) > 0
                         ? xenolith_load32(base, types + 4)
                         : 0;
    uint32_t type = first != 0 ? xenolith_load32(base, first + 4) : 0;
    if (type == 0) {
        return;
    }

    char name[128];
    size_t used = 0;
    for (; used + 1 < sizeof name; used++) {
        char letter = (char)xenolith_load8(base, type + 8 + (uint32_t)used);
        if (letter == '\0') {
            break;
        }
        name[used] = letter >= 0x20 && letter < 0x7f ? letter : '?';
    }
    name[used] = '\0';
    fprintf(stderr, "xenolith: what was thrown is a %s, held at %#010x\n", name, object);
}

/* Says where the calls that led here came from, by walking the title's stack.
 *
 * Each frame's first word leads to the frame of the function that called it, and
 * the return address that function saved sits just below that frame. The walk
 * stops at the first frame that does not lead further up the stack, or that
 * saved no return address. */
static void say_who_called(uint8_t *base, uint32_t frame) {
    fprintf(stderr, "xenolith: called by");
    for (int count = 0; count < 24; count++) {
        uint32_t above = xenolith_load32(base, frame);
        if (above <= frame || above > XENOLITH_STACK_TOP) {
            break;
        }
        uint32_t returned_to = xenolith_load32(base, above - 8);
        if (returned_to == 0) {
            break;
        }
        fprintf(stderr, " %#010x", returned_to);
        frame = above;
    }
    fprintf(stderr, "\n");
}

/* What a title asks of the kernel about what it can see.
 *
 * There is no graphics hardware here, so most of these say what they cannot
 * rather than pretending they did. */
static int serviced_about_what_it_sees(xenolith_context *ctx, uint8_t *base, uint32_t ordinal) {
    switch (ordinal) {
    case 210:
        open_file(ctx, base, 1);
        return 1;
    case 223:
        open_file(ctx, base, 0);
        return 1;
    case 240:
        read_file(ctx, base);
        return 1;
    case 255:
        write_file(ctx, base);
        return 1;
    case 219: {
        /* Making sure what was written has gone. Every write here goes the
         * moment it is asked for, so there is nothing waiting. */
        struct object *found = object_of((uint32_t)ctx->r[3]);
        if (found != NULL && found->file != NULL) {
            fflush(found->file);
        }
        report_io(base, (uint32_t)ctx->r[4], XENOLITH_STATUS_SUCCESS, 0);
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 232:
        query_file(ctx, base);
        return 1;
    case 247:
        set_file(ctx, base);
        return 1;
    case 228:
        query_directory(ctx, base);
        return 1;
    case 548: {
        /* Making a context for the hardware that decodes compressed sound.
         *
         * A context is a block of memory the title fills in with where its
         * compressed sound is and where the decoded sound should go, and the
         * hardware reads and updates it. Nothing here decodes that kind of
         * sound, so the block is made and handed over, the title fills it in,
         * and nothing ever plays: that is said once rather than left for a
         * title to fall silent without a word. */
        static int said;
        if (!said) {
            said = 1;
            say_a_report("compressed sound is not decoded here, so a title's music and "
                         "voices are not heard");
        }
        uint32_t into = (uint32_t)ctx->r[3];
        uint32_t context = take_guest_memory(XENOLITH_SOUND_CONTEXT, 0);
        if (into == 0 || context == 0) {
            ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
            return 1;
        }
        xenolith_store32(base, into, context);
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 550:
        /* Letting a context go. Memory here is never given back. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 499: {
        /* Registering a client that will hand sound over.
         *
         * The title passes a small record holding the callback the hardware
         * calls to ask for the next frame and what to hand it, in that order,
         * and somewhere to write the number the client will be known by. The
         * asking starts here, on a thread of its own, and runs at the
         * console's rate from then on. */
        uint32_t at = (uint32_t)ctx->r[3];
        uint32_t known_as = (uint32_t)ctx->r[4];
        if (at != 0) {
            __atomic_store_n(&sound_client, xenolith_load32(base, at + 4), __ATOMIC_RELEASE);
            __atomic_store_n(&sound_client_callback, xenolith_load32(base, at),
                             __ATOMIC_RELEASE);
        }
        if (known_as != 0) {
            xenolith_store32(base, known_as, XENOLITH_SOUND_DRIVER);
        }
        static int asking;
        if (!asking) {
            asking = 1;
            pthread_t thread;
            if (pthread_create(&thread, NULL, ask_for_sound, base) == 0) {
                pthread_detach(thread);
            }
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 500:
        /* Unregistering one, which stops it being asked. */
        __atomic_store_n(&sound_client_callback, 0, __ATOMIC_RELEASE);
        __atomic_store_n(&sound_client, 0, __ATOMIC_RELEASE);
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 501: {
        /* Handing over a frame of sound. The frame is six channels of the
         * console's own floats, and what a device here takes is two, so it is
         * folded on its way in and played at whatever rate the device runs.
         * Saying it was taken is what lets a title carry on to the next one,
         * and the taking itself is counted, because that count is the clock the
         * title paces by. A title handing over nothing is one asking for
         * silence to be played in its place, which a starved device is given
         * anyway. */
        uint32_t given = (uint32_t)ctx->r[4];
        if (given != 0) {
            hand_a_frame_over(base, given);
        } else {
            start_the_output();
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 844:
        /* How far playback has got, in samples the device has taken. */
        ctx->r[3] = how_far_playback_has_got();
        return 1;
    case 504:
        /* How loud a kind of sound should be, written where a title points.
         *
         * A console that has had nothing turned down reads as full volume, and
         * a title multiplying its mix by what it reads here has to read
         * something: left holding whatever the word held, it multiplied by
         * nothing and mixed silence. */
        if ((uint32_t)ctx->r[4] != 0) {
            xenolith_store32(base, (uint32_t)ctx->r[4], 0x3f800000u);
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 617:
    case 618:
        /* Retraining the memory the graphics hardware draws into. There is
         * none, so there is nothing to retrain. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 143: {
        struct object *found = object_at(base, (uint32_t)ctx->r[3]);
        if (found != NULL) {
            signal_object(found, 0);
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 157: {
        /* Setting an event named by where it lives. What comes back is
         * whether it was already set. */
        struct object *found = object_at(base, (uint32_t)ctx->r[3]);
        uint32_t was = 0;
        if (found != NULL) {
            pthread_mutex_lock(&found->lock);
            was = found->signalled || found->releases > 0;
            pthread_mutex_unlock(&found->lock);
            signal_object(found, 1);
        }
        ctx->r[3] = was;
        return 1;
    }
    case 254: {
        /* Waiting on several objects named by handle: any of them, or all of
         * them in turn, for as long as the sixth argument says. */
        uint32_t count = (uint32_t)ctx->r[3];
        uint32_t list = (uint32_t)ctx->r[4];
        struct object *waited[64];
        if (count == 0 || count > 64) {
            ctx->r[3] = XENOLITH_STATUS_INVALID_PARAMETER;
            return 1;
        }
        for (uint32_t at = 0; at < count; at++) {
            waited[at] = object_of(xenolith_load32(base, list + at * 4));
        }
        struct timespec until;
        int limited = give_up_at(base, (uint32_t)ctx->r[8], &until);
        if ((uint32_t)ctx->r[5] == 1) {
            ctx->r[3] = wait_for_any(waited, count, limited ? &until : NULL);
            return 1;
        }
        uint32_t status = XENOLITH_STATUS_SUCCESS;
        for (uint32_t at = 0; at < count && status == XENOLITH_STATUS_SUCCESS; at++) {
            if (waited[at] != NULL) {
                status = wait_until(waited[at], limited ? &until : NULL);
            }
        }
        ctx->r[3] = status;
        return 1;
    }
    case 175: {
        /* Waiting on several objects named by where they live.
         *
         * Waiting for any of them takes the first that can be taken. Waiting
         * for all of them takes each in turn, which ends in the same place but
         * not atomically: the console takes none until every one can be taken,
         * and this may hold one while waiting on the next. */
        uint32_t count = (uint32_t)ctx->r[3];
        uint32_t list = (uint32_t)ctx->r[4];
        int for_any = (uint32_t)ctx->r[5] == 1;
        struct object *waited[64];
        if (count == 0 || count > 64) {
            ctx->r[3] = XENOLITH_STATUS_INVALID_PARAMETER;
            return 1;
        }
        for (uint32_t at = 0; at < count; at++) {
            waited[at] = object_at(base, xenolith_load32(base, list + at * 4));
        }
        struct timespec until;
        int limited = give_up_at(base, (uint32_t)ctx->r[9], &until);
        if (for_any) {
            ctx->r[3] = wait_for_any(waited, count, limited ? &until : NULL);
            return 1;
        }
        uint32_t status = XENOLITH_STATUS_SUCCESS;
        for (uint32_t at = 0; at < count && status == XENOLITH_STATUS_SUCCESS; at++) {
            if (waited[at] != NULL) {
                status = wait_until(waited[at], limited ? &until : NULL);
            }
        }
        ctx->r[3] = status;
        return 1;
    }
    case 176: {
        /* Waiting on an object named by where it lives, for as long as the
         * fifth argument says. */
        struct object *found = object_at(base, (uint32_t)ctx->r[3]);
        struct timespec until;
        int limited = give_up_at(base, (uint32_t)ctx->r[7], &until);
        ctx->r[3] = found == NULL ? XENOLITH_STATUS_SUCCESS
                                  : wait_until(found, limited ? &until : NULL);
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
        /* Asking for what was drawn to be shown.
         *
         * A title reaching this has written a frame's commands, which is not
         * the same as their having been read. So nothing is shown here: the
         * commands that show the frame are written where the title left room
         * for them among its own, and the frame is shown when the reading
         * reaches them.
         *
         * The first argument is that room, sixty four words of a buffer the
         * title has already handed to the hardware or is about to. The kernel
         * fills it with one packet that covers all of it, saying which picture
         * to show, and the hardware reads past it to what comes next. Left
         * alone, it holds whatever that memory held the last time round, which
         * is commands from an older frame, and reading those as these puts
         * every command after them out of step.
         *
         * The picture is the copy the title made out of the hardware into its
         * front buffer. The second argument describes that buffer the way a
         * picture is described to a shader, and the address is the top of its
         * second word. */
        graphics.frames++;
        {
            uint32_t described = (uint32_t)ctx->r[4];
            uint32_t front = described != 0 ? xenolith_load32(base, described + 4) & ~0xfffu : 0;
            uint32_t room = (uint32_t)ctx->r[3];
            if (room != 0) {
                uint32_t size = described != 0 ? xenolith_load32(base, described + 8) : 0;
                xenolith_store32(base, room,
                                 (3u << 30) | ((XENOLITH_SWAP_WORDS - 2u) << 16)
                                     | (XENOLITH_PACKET_SWAP << 8));
                xenolith_store32(base, room + 4, XENOLITH_SWAP_TAG);
                xenolith_store32(base, room + 8, front & 0x1fffffffu);
                xenolith_store32(base, room + 12, (size & 0x1fffu) + 1u);
                xenolith_store32(base, room + 16, ((size >> 13) & 0x1fffu) + 1u);
                for (uint32_t word = 5; word < XENOLITH_SWAP_WORDS; word++) {
                    xenolith_store32(base, room + word * 4u, 0);
                }
            }
        }
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
        /* Starting the command processor, of which there is none.
         *
         * Answering an import without saying it succeeded leaves the status
         * holding whatever the title passed in, which for these is an address.
         * A title comparing it against success is then told the call failed and
         * does nothing further, so every one of these says so. */
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 451:
        /* Where the commands are written, which starts them again from the
         * front. */
        graphics.ring_at = (uint32_t)ctx->r[3];
        /* The size is given as a power of two of eight byte units. */
        graphics.ring_words = 2u << ((uint32_t)ctx->r[4] & 0x1f);
        consumed = 0;
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    case 438: {
        /* Where to say how far the commands have been read, and the start of
         * the reading. A title says where more than once, as it sets the
         * hardware up again, and the reading is started only the first time:
         * a second reader walks the same commands as the first, from the same
         * count of how far both have got, so every draw and every copy is
         * carried out twice and the count races between them. */
        graphics.read_back_at = (uint32_t)ctx->r[3];
        static int reading;
        if (!reading) {
            reading = 1;
            pthread_t draining;
            if (pthread_create(&draining, NULL, drain_the_ring, base) == 0) {
                pthread_detach(draining);
            }
            pthread_t counting;
            if (pthread_create(&counting, NULL, count_the_frames, base) == 0) {
                pthread_detach(counting);
            }
        }
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 469:
        graphics.callback = (uint32_t)ctx->r[3];
        graphics.callback_argument = (uint32_t)ctx->r[4];
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
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
        if (lock == NULL) {
            say_a_report("no room left to name critical section %#x",
                         (unsigned)ctx->r[3]);
            ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
            return 1;
        }
        pthread_mutex_lock(lock);
        ctx->r[3] = XENOLITH_STATUS_SUCCESS;
        return 1;
    }
    case 304: {
        pthread_mutex_t *lock = section_of((uint32_t)ctx->r[3]);
        if (lock == NULL) {
            say_a_report("no room left to name critical section %#x",
                         (unsigned)ctx->r[3]);
            ctx->r[3] = XENOLITH_STATUS_NO_MEMORY;
            return 1;
        }
        pthread_mutex_unlock(lock);
        return 1;
    }
    case 310: {
        /* Raising a structured exception.
         *
         * A title reaching this has decided something is wrong and is handing
         * over a record saying what. The record's first word is the code, and
         * the two after it are the flags and the address. Nothing here can
         * unwind a title's stack the way the console's dispatcher would, so the
         * exception is not delivered: what is done instead is reporting what was
         * raised, which is the only thing that says why the title gave up.
         *
         * A title that raises one is not in a state to carry on from, so this
         * does not pretend the exception was handled. It says what happened and
         * stops, which is what a fault means. The word after the flags is a
         * nested record, and the address is the one after that. */
        uint32_t record = (uint32_t)ctx->r[3];
        uint32_t code = record != 0 ? xenolith_load32(base, record) : 0;
        uint32_t flags = record != 0 ? xenolith_load32(base, record + 4) : 0;
        uint32_t at = record != 0 ? xenolith_load32(base, record + 12) : 0;
        fprintf(stderr,
                "xenolith: the title raised an exception %#010x with flags %#x at "
                "%#010x, from %#010x\n",
                code, flags, at, (uint32_t)ctx->lr);
        say_what_was_thrown(base, record);
        say_who_called(base, (uint32_t)ctx->r[1]);
        stop_the_run(3);
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
        /* Where it was called from, because which import a title wants says
         * what it was trying to do and where it asked says which part of the
         * title that was. Without the second, a report of an unimplemented
         * entry point sends a reader looking through a title with nothing to
         * start from. */
        fprintf(stderr,
                "xenolith: %s ordinal %u (%s) is not implemented, and was wanted "
                "from %#010llx\n",
                library, ordinal, name ? name : "unnamed",
                (unsigned long long)ctx->lr);
        say_who_called(base, (uint32_t)ctx->r[1]);
        stop_the_run(3);
    }

    if (reached >= XENOLITH_TRACE_LIMIT) {
        fprintf(stderr, "xenolith: trace: stopping after %lu calls\n", reached);
        stop_the_run(3);
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

/* The time base, counting at the rate a title is told it counts at.
 *
 * A title divides what it reads here by that rate to know how long something
 * took. A count that went up by one at each reading made a title's seconds last
 * as many readings as the rate says, so a wait for a fifth of a second that
 * reads once a frame lasted days, and the screen asking for it never left. */
uint64_t xenolith_timebase(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return (uint64_t)now.tv_sec * XENOLITH_TIMEBASE_HZ
           + (uint64_t)now.tv_nsec / (1000000000u / XENOLITH_TIMEBASE_HZ);
}
