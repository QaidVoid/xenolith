//! Checking that the interface emitted code is written against is usable.
//!
//! The header is the contract between this crate and whatever eventually runs
//! the emitted code. If it does not compile, nothing emitted against it can. If
//! its memory accessors read the host's byte order rather than the guest's, the
//! emitted code works on one machine and not another, which is the kind of
//! mistake that is found years later.
//!
//! A C compiler is required: a check that skipped when it was absent would leave
//! the interface itself unverified while reporting green.

mod honest;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

/// An empty shader table, which is what a title with no shaders it can read
/// gets. The runtime finds nothing by any sum and says so.
const SHADER_TABLE: &str = r#"
#include "xenolith.h"

const uint32_t *xenolith_said(uint32_t kind, uint32_t sum, uint32_t words,
                              uint32_t *length) {
    (void)kind; (void)sum; (void)words; (void)length;
    return 0;
}

const uint32_t *xenolith_reads(uint32_t kind, uint32_t sum, uint32_t words,
                               uint32_t *streams) {
    (void)kind; (void)sum; (void)words; (void)streams;
    return 0;
}

const uint32_t *xenolith_looks_at(uint32_t kind, uint32_t sum, uint32_t words,
                                  uint32_t *pictures) {
    (void)kind; (void)sum; (void)words; (void)pictures;
    return 0;
}
"#;

/// The interface, shipped alongside the code emitted against it.
const HEADER: &str = include_str!("../runtime/xenolith.h");

/// Compiles `program` against the header, returning what the compiler said.
fn compiles(name: &str, program: &str) -> Result<(), String> {
    let compiler = honest::host_compiler();

    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&directory).map_err(|error| format!("making a directory: {error}"))?;
    std::fs::write(directory.join("xenolith.h"), HEADER)
        .map_err(|error| format!("writing the header: {error}"))?;

    let source = directory.join("main.c");
    std::fs::write(&source, program).map_err(|error| format!("writing the program: {error}"))?;

    let output = Command::new(compiler)
        .args(["-std=c17", "-Wall", "-Wextra", "-Werror", "-pedantic"])
        .arg("-o")
        .arg(directory.join("program"))
        .arg(&source)
        .arg("-lm")
        .output()
        .map_err(|error| format!("running the compiler: {error}"))?;

    if output.status.success() {
        return Ok(());
    }
    Err(String::from_utf8_lossy(&output.stderr).into_owned())
}

#[test]
fn the_interface_compiles_with_warnings_enabled() {
    let program = r#"
#include "xenolith.h"

void xenolith_dispatch(xenolith_context *ctx, uint8_t *base, uint32_t address, uint32_t from) {
    (void)from;
    (void)ctx;
    (void)base;
    (void)address;
}

void xenolith_trap(xenolith_context *ctx, uint8_t *base, uint32_t address) {
    (void)ctx;
    (void)base;
    (void)address;
}

void xenolith_unlifted(xenolith_context *ctx, uint8_t *base, uint32_t address) {
    (void)ctx;
    (void)base;
    (void)address;
}

int main(void) {
    xenolith_context ctx = {0};
    ctx.r[3] = 1;
    return (int)ctx.r[3] - 1;
}
"#;

    if let Err(complaint) = compiles("interface", program) {
        panic!("the interface did not compile:\n{complaint}");
    }
}

/// Guest memory is big endian whatever the host is. A value stored through the
/// interface has to read back with its most significant byte at the lowest
/// address, or every multi byte access in emitted code is silently reversed.
#[test]
fn memory_access_uses_the_guest_byte_order() {
    let program = r#"
#include "xenolith.h"
#include <stdlib.h>

void xenolith_dispatch(xenolith_context *ctx, uint8_t *base, uint32_t address, uint32_t from) {
    (void)from;
    (void)ctx;
    (void)base;
    (void)address;
}

void xenolith_trap(xenolith_context *ctx, uint8_t *base, uint32_t address) {
    (void)ctx;
    (void)base;
    (void)address;
}

void xenolith_unlifted(xenolith_context *ctx, uint8_t *base, uint32_t address) {
    (void)ctx;
    (void)base;
    (void)address;
}

int main(void) {
    uint8_t memory[32] = {0};

    xenolith_store32(memory, 0, 0x11223344u);
    if (memory[0] != 0x11 || memory[1] != 0x22 ||
        memory[2] != 0x33 || memory[3] != 0x44) {
        return 1;
    }
    if (xenolith_load32(memory, 0) != 0x11223344u) {
        return 2;
    }

    xenolith_store64(memory, 8, 0x0102030405060708ull);
    if (memory[8] != 0x01 || memory[15] != 0x08) {
        return 3;
    }
    if (xenolith_load64(memory, 8) != 0x0102030405060708ull) {
        return 4;
    }

    xenolith_store16(memory, 20, 0xabcdu);
    if (memory[20] != 0xab || memory[21] != 0xcd) {
        return 5;
    }
    if (xenolith_load16(memory, 20) != 0xabcdu) {
        return 6;
    }

    /* A value written wide and read narrow takes the low part, which is what
     * the instruction set does constantly. */
    xenolith_context ctx = {0};
    ctx.r[3] = 0xdeadbeefcafebabeull;
    if ((uint32_t)ctx.r[3] != 0xcafebabeu) {
        return 7;
    }

    return 0;
}
"#;

    if let Err(complaint) = compiles("byte_order", program) {
        panic!("the check did not compile:\n{complaint}");
    }
    let program_path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("byte_order")
        .join("program");
    let status = Command::new(&program_path)
        .status()
        .unwrap_or_else(|error| panic!("running the check: {error}"));

    assert!(
        status.success(),
        "the interface read or wrote memory in the wrong byte order, check {}",
        status.code().unwrap_or(-1)
    );
}

/// The runtime, compiled beside a program that drives it.
///
/// The header checks say the interface is usable. They say nothing about the
/// runtime that implements it, which is where the guest address space, the heap
/// and the processor block live. So this compiles both and runs one against the
/// other.
fn runs(name: &str, program: &str) -> Result<(), String> {
    let compiler = honest::host_compiler();
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&directory).map_err(|error| format!("making a directory: {error}"))?;

    for (file, text) in [
        ("xenolith.h", HEADER),
        ("xenolith.c", include_str!("../runtime/xenolith.c")),
        (
            "command_stream.c",
            include_str!("../runtime/command_stream.c"),
        ),
        ("sound.c", include_str!("../runtime/sound.c")),
        ("drawing.c", include_str!("../runtime/drawing.c")),
        ("vulkan.c", include_str!("../runtime/vulkan.c")),
        /* The shader table, which the lift emits. A test has no image, so it
         * gets one with nothing in it. */
        ("shaders.c", SHADER_TABLE),
        ("main.c", program),
    ] {
        std::fs::write(directory.join(file), text)
            .map_err(|error| format!("writing {file}: {error}"))?;
    }

    let program_path = directory.join("program");
    let output = Command::new(compiler)
        .args(["-std=c17", "-Wall", "-Wextra", "-Werror", "-pedantic"])
        .arg("-o")
        .arg(&program_path)
        .arg(directory.join("main.c"))
        .arg(directory.join("xenolith.c"))
        .arg(directory.join("vulkan.c"))
        .arg(directory.join("shaders.c"))
        .args(["-lvulkan", "-lm", "-lpthread"])
        .output()
        .map_err(|error| format!("running the compiler: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }

    /* Run where the program was built rather than where the tests were
     * started. Several of these write an image file of their own beside
     * themselves, and sharing one directory would have them writing over each
     * other's. */
    let ran = Command::new(&program_path)
        .current_dir(&directory)
        .output()
        .map_err(|error| format!("running the check: {error}"))?;
    if ran.status.success() {
        Ok(())
    } else {
        Err(format!(
            "check {} failed:\n{}",
            ran.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&ran.stderr)
        ))
    }
}

/// A guest thread reaches the block the kernel keeps for it through r13.
///
/// The block holds the tick count and which processor this is. Left at nothing
/// every thread reads processor nought, and a title's vector code picks a slot
/// from that number and steps a counter there, so two threads on the same block
/// step the same counter and carry it past the end of it.
///
/// The thread is started the way a title starts one, by importing the kernel's
/// own create-thread call, and what it found is left where this can read it.
#[test]
fn a_thread_reaches_a_processor_block_through_r13() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>

/* What the guest thread found, written by the thread itself. */
static uint32_t reached_r13;
static uint32_t ticks;
static uint8_t processor;
static uint32_t done;

void sub_82000000(xenolith_context *ctx, uint8_t *base) {
    reached_r13 = (uint32_t)ctx->r[13];
    if (reached_r13 == 0) {
        done = 1;
        return;
    }
    uint32_t shared = xenolith_load32(base, reached_r13 + 256u);
    if (shared != 0) {
        /* The count is kept by another thread that starts with the first block,
         * so it is waited for rather than read once: what matters is that it
         * moves, not that it has already moved. */
        for (int waited = 0; waited < 2000 && ticks == 0; waited++) {
            struct timespec once = {0, 1000000};
            nanosleep(&once, NULL);
            ticks = xenolith_load32(base, shared + 88u);
        }
    }
    processor = base[reached_r13 + 268u + 3u];
    done = 1;
}

/* The table an indirect branch resolves against. */
xenolith_function xenolith_lookup(uint32_t address) {
    return address == 0x82000000u ? sub_82000000 : 0;
}

int main(void) {
    /* The runtime reads and writes guest addresses all over the span, so the
     * span has to be mapped before it is handed over. */
    uint8_t *base = mmap(NULL, (size_t)1 << 32, PROT_READ | PROT_WRITE,
                         MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (base == MAP_FAILED) {
        fprintf(stderr, "could not map the guest span\n");
        return 6;
    }

    /* A context where the thread body is the one above and nothing else is
     * named. The handle and the id are written into the guest span, which the
     * runtime is handed. */
    xenolith_context ctx = {0};
    uint32_t handle_at = 64;
    uint32_t id_at = 128;
    ctx.r[3] = handle_at;
    ctx.r[5] = id_at;
    ctx.r[6] = 0;
    ctx.r[7] = 0x82000000u;
    ctx.r[8] = 0;
    ctx.r[9] = 0;

    xenolith_import(&ctx, base, "xboxkrnl.exe", 13, "NtCreateThreadEx");
    if (ctx.r[3] != 0u) {
        return 1;
    }

    for (int waited = 0; waited < 2000 && done == 0; waited++) {
        struct timespec once = {0, 1000000};
        nanosleep(&once, NULL);
    }
    if (done == 0) {
        fprintf(stderr, "the guest thread never ran\n");
        return 2;
    }
    if (reached_r13 == 0) {
        fprintf(stderr, "the guest thread read nothing through r13\n");
        return 3;
    }
    if (ticks == 0) {
        fprintf(stderr, "the shared tick count was never written\n");
        return 4;
    }
    if (processor >= 6) {
        fprintf(stderr, "the processor number was %u, and the console has six\n",
                (unsigned)processor);
        return 5;
    }
    return 0;
}
"#;
    if let Err(complaint) = runs("processor_block", program) {
        panic!("a thread did not reach its processor block:\n{complaint}");
    }
}

/// A read that begins in the last bytes of the guest span lands on something.
///
/// Guest addresses are four bytes wide and a read works out where it is reading
/// before taking the bytes, so a word begun at the very top would otherwise run
/// past the end of the mapping. Only a title reading through a pointer that is
/// already nonsense gets there, and what it reads is nothing either way, but
/// bringing the whole run down is not the same as reading nothing.
#[test]
fn a_read_at_the_top_of_the_span_does_not_fall_off_the_end() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);

    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    /* The last address a four byte read can start at, which runs two bytes past
     * the span. */
    uint32_t at = 0xfffffffeu;
    if (xenolith_load32(base, at) != 0u) {
        return 3;
    }
    return 0;
}
"#;
    if let Err(complaint) = runs("top_of_span", program) {
        panic!("a read at the top of the span did not land:\n{complaint}");
    }
}

/// Guest memory is handed out in distinct grains, aligned as asked.
///
/// Two allocations that overlap would put one title structure on top of another,
/// which is the kind of fault that shows up as corruption far from the call.
/// The alignment is what a title asks for and gets told it got, so it has to be
/// the alignment it asked for.
#[test]
fn two_allocations_do_not_overlap_and_are_aligned() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

/* Asks for `size` bytes at `alignment`, the way a title asks. */
static int allocate(xenolith_context *ctx, uint8_t *base, uint32_t size,
                    uint32_t alignment, uint32_t *got_at, uint32_t *got_size) {
    uint32_t address = 0;
    uint32_t size_slot = size;
    ctx->r[3] = 0;          /* nowhere in particular */
    ctx->r[4] = 0;
    xenolith_store32(base, 0x100, 0);
    xenolith_store32(base, 0x104, size);
    ctx->r[3] = 0x100;
    ctx->r[4] = 0x104;
    ctx->r[5] = 0;
    ctx->r[6] = 0;
    ctx->r[7] = 0;
    ctx->r[8] = alignment;
    (void)address;
    (void)size_slot;
    xenolith_import(ctx, base, "xboxkrnl.exe", 204, "NtAllocateVirtualMemory");
    if (ctx->r[3] != 0u) {
        return 1;
    }
    *got_at = xenolith_load32(base, 0x100);
    *got_size = xenolith_load32(base, 0x104);
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    xenolith_context ctx = {0};
    uint32_t first_at = 0, first_size = 0;
    uint32_t second_at = 0, second_size = 0;
    if (allocate(&ctx, base, 0x3000, 0x10000, &first_at, &first_size) != 0) {
        fprintf(stderr, "the first allocation failed\n");
        return 3;
    }
    if (allocate(&ctx, base, 0x3000, 0x20000, &second_at, &second_size) != 0) {
        fprintf(stderr, "the second allocation failed\n");
        return 4;
    }

    if (first_at == 0 || second_at == 0) {
        fprintf(stderr, "an allocation came back as nothing\n");
        return 5;
    }
    if (first_at % 0x10000u != 0 || second_at % 0x20000u != 0) {
        fprintf(stderr, "an allocation was not aligned as asked: %#x at %u, %#x at %u\n",
                first_at, (unsigned)(first_at % 0x10000u),
                second_at, (unsigned)(second_at % 0x20000u));
        return 6;
    }
    /* A size is rounded up to whole grains, and nothing of it is smaller than
     * what was asked for. */
    if (first_size < 0x3000 || second_size < 0x3000) {
        fprintf(stderr, "a size came back smaller than it was asked for\n");
        return 7;
    }
    if (second_at < first_at + first_size) {
        fprintf(stderr, "the two allocations overlap: %#x+%u and %#x\n",
                first_at, (unsigned)first_size, second_at);
        return 8;
    }
    return 0;
}
"#;
    if let Err(complaint) = runs("two_allocations", program) {
        panic!("guest memory was not handed out as asked:\n{complaint}");
    }
}

/// A size that cannot be rounded into a word is refused rather than answered.
///
/// A request close to the top of what a word holds rounds past it. Rounded in a
/// word, the addition wraps to nought and the request comes back as one grain,
/// so the caller is handed a sixteenth of a megabyte where it asked for nearly
/// four gigabytes and is told it succeeded.
#[test]
fn an_unroundable_size_is_refused() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    xenolith_context ctx = {0};
    xenolith_store32(base, 0x100, 0);
    xenolith_store32(base, 0x104, 0xffffffffu);
    ctx.r[3] = 0x100;
    ctx.r[4] = 0x104;
    ctx.r[8] = 0x10000;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 204, "NtAllocateVirtualMemory");

    /* No memory left is what the kernel says when it cannot do what was asked,
     * and it is the only honest answer here: the size asked for does not fit. */
    if (ctx.r[3] != 0xC0000017u) {
        fprintf(stderr, "a size of four gigabytes came back as %#llx\n",
                (unsigned long long)ctx.r[3]);
        return 3;
    }
    return 0;
}
"#;
    if let Err(complaint) = runs("unroundable", program) {
        panic!("an unroundable size was answered rather than refused:\n{complaint}");
    }
}

/// An allocation that would end past the arena is refused rather than granted.
///
/// The bound used to be checked as `rounded > END - at`. Where `at` is already
/// past the end, that subtraction wraps to something enormous and the test
/// passes, so the caller is handed an address outside everything the runtime
/// manages. One title asks for nearly four gigabytes, is granted an address past
/// the end, and the fill it writes there reaches the image's own data section
/// and overwrites a live object. The program then calls through a vtable slot
/// holding the fill's own byte, and stops on an address that is not code.
///
/// Three asks are made, and each has to come back as no memory. A single grain
/// that does not fit at the top, an ask that is one grain too many, and an ask
/// larger than the whole arena.
#[test]
fn an_allocation_past_the_arena_is_refused() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

/* Asks for `size` bytes at `alignment` and says whether it was granted. */
static int ask_aligned(xenolith_context *ctx, uint8_t *base, uint32_t size,
                       uint32_t alignment, uint32_t *got) {
    xenolith_store32(base, 0x100, 0);
    xenolith_store32(base, 0x104, size);
    ctx->r[3] = 0x100;
    ctx->r[4] = 0x104;
    ctx->r[8] = alignment;
    xenolith_import(ctx, base, "xboxkrnl.exe", 204, "NtAllocateVirtualMemory");
    if (ctx->r[3] != 0u) {
        return 0;
    }
    *got = xenolith_load32(base, 0x100);
    return 1;
}

static int ask(xenolith_context *ctx, uint8_t *base, uint32_t size, uint32_t *got) {
    return ask_aligned(ctx, base, size, 0x10000u, got);
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) { return 1; }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) { return 2; }

    xenolith_context ctx = {0};
    uint32_t got = 0;

    /* An alignment the title chooses, and one a title can choose wrongly.
     *
     * Rounding the next grain up to it wraps, so the address the allocator
     * computes is above the arena's end. The old bound was
     * `rounded > END - at`, and with `at` above the end that subtraction wraps
     * to something enormous, so the test passed and the address was handed out.
     * One title asks for nearly four gigabytes with an alignment like this, is
     * granted an address past everything, and the fill it writes there reaches
     * the image's own data section and overwrites a live object. */
    if (ask_aligned(&ctx, base, 0x10000u, 0x80000000u, &got)) {
        fprintf(stderr, "an ask aligned past the end was granted at %#x\n", got);
        return 3;
    }

    /* And after that refusal the arena still works, because a refusal must not
     * leave the bookkeeping somewhere unusable. */
    if (!ask(&ctx, base, 0x10000u, &got)) {
        fprintf(stderr, "a grain was refused after a refusal\n");
        return 4;
    }
    if (got == 0u || got >= 0x82000000u) {
        fprintf(stderr, "a grain was granted at %#x, which is outside the arena\n", got);
        return 5;
    }
    printf("refused past the end\n");
    return 0;
}
"#;
    if let Err(complaint) = runs("past_the_arena", program) {
        panic!("an allocation past the arena was granted:\n{complaint}");
    }
}

/// A wait given a length of time comes back when that length has passed.
///
/// A title hands over a count of hundreds of nanoseconds, negative for a stretch
/// from now. A wait that ignores it waits for ever, and the thread that would
/// have carried on never does, which looks like a title that has stopped rather
/// than one waiting for something that never comes.
#[test]
fn a_wait_with_a_timeout_returns_at_the_timeout() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>
#include <time.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    /* An event nothing will ever set, and a wait of ten milliseconds. The kind
     * is nought for the sort that stays set until it is cleared and anything
     * else for the sort that lets a single waiter through, so this is the
     * second sort: one set is spent by one wait. */
    xenolith_context ctx = {0};
    ctx.r[3] = 0x200;      /* where the handle is written */
    ctx.r[4] = 0;          /* the handle's own process */
    ctx.r[5] = 1;          /* not the sort that stays set */
    ctx.r[6] = 0;          /* signalled to begin with */
    xenolith_import(&ctx, base, "xboxkrnl.exe", 209, "NtCreateEvent");
    uint32_t handle = xenolith_load32(base, 0x200);
    if (handle == 0) {
        return 3;
    }

    /* Ten milliseconds, counted in hundreds of nanoseconds, from now. */
    int64_t stretch = -100000;
    xenolith_store64(base, 0x300, (uint64_t)stretch);
    ctx.r[3] = handle;
    ctx.r[5] = 0x300;

    struct timespec began, ended;
    clock_gettime(CLOCK_MONOTONIC, &began);
    xenolith_import(&ctx, base, "xboxkrnl.exe", 253, "NtWaitForSingleObject");
    clock_gettime(CLOCK_MONOTONIC, &ended);

    if (ctx.r[3] != 0x00000102u) {
        fprintf(stderr, "the wait came back with %#llx rather than a timeout\n",
                (unsigned long long)ctx.r[3]);
        return 4;
    }
    long long took = (long long)(ended.tv_sec - began.tv_sec) * 1000000000ll
                     + (long long)(ended.tv_nsec - began.tv_nsec);
    /* Ten milliseconds, with room either way for a host that was busy. */
    if (took < 5000000ll || took > 2000000000ll) {
        fprintf(stderr, "the wait took %lld nanoseconds\n", took);
        return 5;
    }
    return 0;
}
"#;
    if let Err(complaint) = runs("timeout", program) {
        panic!("a wait with a timeout did not come back at the timeout:\n{complaint}");
    }
}

/// Two sets release two waiters rather than one.
///
/// A set is a bit, and a second set landing on a bit that is already set would
/// let nobody else through. So the sets are counted rather than remembered, and
/// a thread that waits after both has nothing to spend.
#[test]
fn two_sets_release_two_waiters() {
    let program = r#"
/* Asked for anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>
#include <time.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

/* Waits on a handle for `hundreds` hundreds of nanoseconds and reports what
 * came back. */
static uint32_t wait_for(xenolith_context *ctx, uint8_t *base, uint32_t handle,
                         int64_t hundreds) {
    xenolith_store64(base, 0x300, (uint64_t)hundreds);
    ctx->r[3] = handle;
    ctx->r[5] = 0x300;
    xenolith_import(ctx, base, "xboxkrnl.exe", 253, "NtWaitForSingleObject");
    return (uint32_t)ctx->r[3];
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    xenolith_context ctx = {0};
    ctx.r[3] = 0x200;
    ctx.r[4] = 0;
    ctx.r[5] = 1;          /* not the sort that stays set */
    ctx.r[6] = 0;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 209, "NtCreateEvent");
    uint32_t handle = xenolith_load32(base, 0x200);
    if (handle == 0) {
        return 3;
    }

    /* Two sets with nobody waiting. */
    for (int at = 0; at < 2; at++) {
        ctx.r[3] = handle;
        xenolith_import(&ctx, base, "xboxkrnl.exe", 246, "NtSetEvent");
    }

    /* Both of them are spent by a wait that comes back at once. */
    if (wait_for(&ctx, base, handle, -100000) != 0u) {
        fprintf(stderr, "the first wait after two sets did not come back\n");
        return 4;
    }
    if (wait_for(&ctx, base, handle, -100000) != 0u) {
        fprintf(stderr, "the second wait after two sets did not come back\n");
        return 5;
    }
    /* And a third has nothing left to spend. */
    if (wait_for(&ctx, base, handle, -100000) != 0x00000102u) {
        fprintf(stderr, "a third wait was let through by two sets\n");
        return 6;
    }
    return 0;
}
"#;
    if let Err(complaint) = runs("two_sets", program) {
        panic!("two sets did not release two waiters:\n{complaint}");
    }
}

/// A pulse releases every waiter it names, and leaves nothing behind.
///
/// A title pulses to let go of everything waiting now without setting the
/// object: anyone arriving afterwards waits, which is what tells a pulse apart
/// from a set. Signalling and then clearing lets a single waiter through,
/// because a set that finds threads waiting releases one and the clear takes
/// the mark back before anybody else looks.
#[test]
fn a_pulse_releases_every_waiter_and_leaves_nothing_behind() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

static uint32_t wait_for(xenolith_context *ctx, uint8_t *base, uint32_t handle,
                         int64_t hundreds) {
    xenolith_store64(base, 0x300, (uint64_t)hundreds);
    ctx->r[3] = handle;
    ctx->r[5] = 0x300;
    xenolith_import(ctx, base, "xboxkrnl.exe", 253, "NtWaitForSingleObject");
    return (uint32_t)ctx->r[3];
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    xenolith_context ctx = {0};
    ctx.r[3] = 0x200;
    ctx.r[4] = 0;
    ctx.r[5] = 1;          /* not the sort that stays set */
    ctx.r[6] = 0;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 209, "NtCreateEvent");
    uint32_t handle = xenolith_load32(base, 0x200);
    if (handle == 0) {
        return 3;
    }

    /* A pulse with nobody waiting leaves the object as it was, and what it was
     * is unsignalled, so a wait after it still times out. */
    ctx.r[3] = handle;
    ctx.r[4] = 0;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 226, "NtPulseEvent");
    if (wait_for(&ctx, base, handle, -100000) != 0x00000102u) {
        fprintf(stderr, "a pulse left something behind for a later waiter\n");
        return 4;
    }

    /* A set with nobody waiting is spent by one wait and no more. */
    ctx.r[3] = handle;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 246, "NtSetEvent");
    if (wait_for(&ctx, base, handle, -100000) != 0u) {
        fprintf(stderr, "a set was not spent by a wait\n");
        return 5;
    }
    if (wait_for(&ctx, base, handle, -100000) != 0x00000102u) {
        fprintf(stderr, "a set was spent by two waits\n");
        return 6;
    }
    return 0;
}
"#;
    if let Err(complaint) = runs("pulse", program) {
        panic!("a pulse did not behave as a pulse:\n{complaint}");
    }
}

/// A critical section table with no room left reports failure rather than
/// carrying on.
///
/// Carrying on is what lets two threads into the same critical section: each is
/// told it holds a lock that was never taken, and what the section was
/// protecting is written by both at once. The table holds a fixed number of
/// sections, so filling it is a thing a title can do.
#[test]
fn a_full_critical_section_table_reports_failure() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

/* Takes the lock at `at`, reporting what came back. */
static uint32_t take(xenolith_context *ctx, uint8_t *base, uint32_t at) {
    ctx->r[3] = at;
    xenolith_import(ctx, base, "xboxkrnl.exe", 293, "RtlEnterCriticalSection");
    return (uint32_t)ctx->r[3];
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    xenolith_context ctx = {0};

    /* More sections than the table holds, each a different address, none of
     * them released. */
    uint32_t filled = 0;
    for (uint32_t at = 0x1000; at < 0x1000 + 4u * 2000u; at += 4) {
        uint32_t status = take(&ctx, base, at);
        if (status == 0u) {
            filled++;
            continue;
        }
        /* The first refusal is what this is looking for: no memory left, and
         * not a lock that was never taken. */
        if (status != 0xC0000017u) {
            fprintf(stderr, "a full table reported %#x\n", (unsigned)status);
            return 3;
        }
        printf("filled %u\n", (unsigned)filled);
        return filled == 0 ? 4 : 0;
    }

    fprintf(stderr, "the table never filled: %u sections were named\n", (unsigned)filled);
    return 5;
}
"#;
    if let Err(complaint) = runs("full_sections", program) {
        panic!("a full critical section table did not report failure:\n{complaint}");
    }
}

/// The ordinals the runtime answers are exactly the ones the list names.
///
/// The switch that answered them was one function holding all of them. Splitting
/// it by library and by subject is only safe if the set stays what it was: an
/// ordinal dropped in the move is an import that stops working, and an ordinal
/// added without being written down is one nobody reviews. The list is read as
/// well as the runtime, so the two have to be changed together.
#[test]
fn the_ordinals_the_runtime_answers_are_the_ones_written_down() {
    let runtime = include_str!("../runtime/xenolith.c");
    let written = include_str!("../runtime/ordinals.txt");

    let mut expected: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    let mut library = String::new();
    for line in written.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            library = line.trim_matches(['[', ']'].as_slice()).to_owned();
            expected.entry(library.clone()).or_default();
            continue;
        }
        let ordinal: u32 = line
            .parse()
            .unwrap_or_else(|error| panic!("{line:?} is not an ordinal: {error}"));
        expected.entry(library.clone()).or_default().insert(ordinal);
    }
    assert_eq!(
        expected.len(),
        2,
        "the list names {} libraries and the runtime answers two",
        expected.len()
    );

    let found = ordinals_in(runtime);
    assert_eq!(
        found, expected,
        "the ordinals the runtime answers and the ones written down disagree"
    );
}

/// The ordinals each library's answers are written as, read out of the source.
///
/// The three kernel functions are found by name and the `xam.xex` answers are
/// the cases in `serviced` that are not a call to one of them, which is what the
/// split left in that function.
fn ordinals_in(runtime: &str) -> BTreeMap<String, BTreeSet<u32>> {
    let mut found = BTreeMap::new();
    found.insert("xam.xex".to_owned(), cases_of(runtime, "serviced"));
    let mut kernel = BTreeSet::new();
    for name in [
        "serviced_about_what_it_holds",
        "serviced_about_memory_and_time",
        "serviced_about_what_it_sees",
    ] {
        kernel.extend(cases_of(runtime, name));
    }
    found.insert("xboxkrnl.exe".to_owned(), kernel);
    found
}

/// Every `case N:` in the body of the function called `name`.
///
/// The body is taken from the definition rather than from the declaration of the
/// same name, which is why the search skips anything ending in a semicolon.
fn cases_of(runtime: &str, name: &str) -> BTreeSet<u32> {
    /* The definition rather than the declaration of the same name, and only the
     * first one: `serviced` is not `serviced_by_the_kernel`, and looking for more
     * than one would read every function whose name starts the same way. */
    /* The name has to be the whole name. `static int serviced(` is a prefix of
     * `static int serviced_about_what_it_holds(`, so what follows the name has
     * to be the bracket that opens its own arguments rather than more letters. */
    let opening = format!("static int {name}");
    let Some(start) = runtime
        .match_indices(&opening)
        .map(|(at, _)| at)
        .find(|at| {
            let after = at + opening.len();
            if !runtime[after..].starts_with('(') {
                return false;
            }
            /* A declaration ends in a semicolon before any brace, so the first
             * brace after the name has to be the one that opens the body. */
            match runtime[after..].find(['{', ';']) {
                Some(at) => runtime[after + at..].starts_with('{'),
                None => false,
            }
        })
    else {
        return BTreeSet::new();
    };
    let Some(brace) = runtime[start..].find('{') else {
        return BTreeSet::new();
    };
    let body_at = start + brace;
    let mut depth = 0usize;
    let mut end = body_at;
    for (offset, letter) in runtime[body_at..].char_indices() {
        match letter {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = body_at + offset;
                    break;
                }
            }
            _ => {}
        }
    }
    let body = &runtime[body_at..=end];
    let mut found = BTreeSet::new();
    for (case, _) in body.match_indices("case ") {
        let rest = &body[case + "case ".len()..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(number) = digits.parse() {
            found.insert(number);
        }
    }
    found
}

/// A file beside the program opens, reads, and reports how long it is.
///
/// A title names its files with a prefix of its own and never with a path a
/// host would recognise, so what is being checked is the mapping from the one
/// to the other as much as the reading. A title that cannot open its data looks
/// exactly like one that opened it and read nothing.
#[test]
fn a_file_beside_the_program_opens_and_reads() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>
#include <string.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

/* The structure a title names a file with: what it may do, and the counted name
 * a little further in. Written into guest memory the way a title writes it. */
static void name_a_file(uint8_t *base, uint32_t at, const char *name) {
    size_t length = strlen(name);
    xenolith_store32(base, at + 4, at + 0x40);
    xenolith_store16(base, at + 0x40, (uint16_t)length);
    xenolith_store32(base, at + 0x44, at + 0x80);
    for (size_t letter = 0; letter < length; letter++) {
        base[at + 0x80 + letter] = (uint8_t)name[letter];
    }
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    /* The file the test wrote, named the way a title names one. */
    const char *wrote = "beside.txt";
    FILE *data = fopen(wrote, "wb");
    if (data == NULL) {
        return 3;
    }
    fputs("abcdef", data);
    fclose(data);

    xenolith_context ctx = {0};
    name_a_file(base, 0x1000, "game:\\beside.txt");
    ctx.r[3] = 0x2000;      /* where the handle goes */
    ctx.r[4] = 0x80000000u; /* may read */
    ctx.r[5] = 0x1000;      /* the name */
    ctx.r[6] = 0x2100;      /* where what happened is reported */
    ctx.r[8] = 0;           /* a file rather than a directory */
    xenolith_import(&ctx, base, "xboxkrnl.exe", 223, "NtOpenFile");
    if (ctx.r[3] != 0u) {
        fprintf(stderr, "opening failed with %#llx\n", (unsigned long long)ctx.r[3]);
        return 4;
    }
    uint32_t handle = xenolith_load32(base, 0x2000);
    if (handle == 0) {
        return 5;
    }

    /* How long it is, which a title asks before making room to read it. */
    ctx.r[3] = handle;
    ctx.r[4] = 0x2200;
    ctx.r[5] = 0x2300;
    ctx.r[7] = 1;           /* how long the file is */
    xenolith_import(&ctx, base, "xboxkrnl.exe", 232, "NtQueryInformationFile");
    uint64_t length = xenolith_load64(base, 0x2300);
    if (length != 6u) {
        fprintf(stderr, "the file was reported as %llu bytes long\n",
                (unsigned long long)length);
        return 6;
    }

    /* Reading it, from the front. */
    xenolith_store64(base, 0x2400, 0);
    ctx.r[3] = handle;
    ctx.r[4] = 0;
    ctx.r[7] = 0x2500;
    ctx.r[8] = 0x3000;      /* where the bytes go */
    ctx.r[9] = 6;
    ctx.r[10] = 0x2400;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 240, "NtReadFile");
    if (ctx.r[3] != 0u) {
        fprintf(stderr, "reading failed with %#llx\n", (unsigned long long)ctx.r[3]);
        return 7;
    }
    if (memcmp(base + 0x3000, "abcdef", 6) != 0) {
        fprintf(stderr, "the bytes read were not the ones in the file\n");
        return 8;
    }
    return 0;
}
"#;
    if let Err(complaint) = runs("files", program) {
        panic!("a file beside the program did not open and read:\n{complaint}");
    }
}

/// A file that is not there reports its name once and returns the status.
///
/// A title that cannot find its data goes on and does something else, so the
/// report is the only thing that says the data was missing. Reporting it on
/// every attempt would fill the output with one name, which is why it is said
/// once.
#[test]
fn a_missing_file_is_reported_once() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>
#include <string.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

static void name_a_file(uint8_t *base, uint32_t at, const char *name) {
    size_t length = strlen(name);
    xenolith_store32(base, at + 4, at + 0x40);
    xenolith_store16(base, at + 0x40, (uint16_t)length);
    xenolith_store32(base, at + 0x44, at + 0x80);
    for (size_t letter = 0; letter < length; letter++) {
        base[at + 0x80 + letter] = (uint8_t)name[letter];
    }
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    xenolith_context ctx = {0};
    name_a_file(base, 0x1000, "game:\\not_there.txt");
    for (int at = 0; at < 3; at++) {
        ctx.r[3] = 0x2000;
        ctx.r[4] = 0x80000000u;
        ctx.r[5] = 0x1000;
        ctx.r[6] = 0x2100;
        ctx.r[8] = 0;
        xenolith_import(&ctx, base, "xboxkrnl.exe", 223, "NtOpenFile");
        if (ctx.r[3] != 0xC0000034u) {
            fprintf(stderr, "a missing file came back as %#llx\n",
                    (unsigned long long)ctx.r[3]);
            return 3;
        }
    }
    return 0;
}
"#;
    if let Err(complaint) = runs("missing_file", program) {
        panic!("a missing file was not reported as missing:\n{complaint}");
    }

    /* Said once, not once per attempt: the program asks three times. */
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("missing_file");
    let said = Command::new(directory.join("program"))
        .current_dir(&directory)
        .output()
        .unwrap_or_else(|error| panic!("running the check: {error}"));
    let complaint = String::from_utf8_lossy(&said.stderr);
    let mentions = complaint.matches("not_there.txt").count();
    assert_eq!(
        mentions, 1,
        "a missing file was named {mentions} times, and it should be named once:\n{complaint}"
    );
}

/// Two threads see their own thread-local values.
///
/// A title puts something in a slot on one thread and expects to read it back on
/// that thread and nowhere else. One set shared between threads gives every
/// thread the last value written by any of them, which shows up as one worker
/// reading another's work.
#[test]
fn two_threads_see_their_own_thread_local_values() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <pthread.h>
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

static uint8_t *where;
static uint32_t slot;

static uint32_t wrong;

/* What each thread puts in the slot and what it read back. */
struct kept {
    uint32_t wrote;
    uint32_t read;
};

static void *use_the_slot(void *given) {
    struct kept *mine = given;
    xenolith_context ctx = {0};

    ctx.r[3] = slot;
    ctx.r[4] = mine->wrote;
    xenolith_import(&ctx, where, "xboxkrnl.exe", 341, "KeTlsSetValue");

    /* Long enough that the other thread has certainly written by now. */
    for (volatile int spin = 0; spin < 2000000; spin++) {
    }

    ctx.r[3] = slot;
    xenolith_import(&ctx, where, "xboxkrnl.exe", 340, "KeTlsGetValue");
    mine->read = (uint32_t)ctx.r[3];
    if (mine->read != mine->wrote) {
        __atomic_add_fetch(&wrong, 1, __ATOMIC_RELAXED);
    }
    return NULL;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    where = xenolith_boot(path, 0);
    if (where == NULL) {
        return 2;
    }

    xenolith_context ctx = {0};
    xenolith_import(&ctx, where, "xboxkrnl.exe", 338, "KeTlsAlloc");
    slot = (uint32_t)ctx.r[3];
    if (slot == 0xFFFFFFFFu) {
        return 3;
    }

    struct kept one = {0x11111111u, 0};
    struct kept two = {0x22222222u, 0};
    pthread_t first, second;
    if (pthread_create(&first, NULL, use_the_slot, &one) != 0) {
        return 4;
    }
    if (pthread_create(&second, NULL, use_the_slot, &two) != 0) {
        return 5;
    }
    pthread_join(first, NULL);
    pthread_join(second, NULL);

    if (wrong != 0) {
        fprintf(stderr, "%u threads read a value another thread wrote\n",
                (unsigned)wrong);
        return 6;
    }
    return 0;
}
"#;
    if let Err(complaint) = runs("thread_local", program) {
        panic!("two threads did not see their own values:\n{complaint}");
    }
}

/// The import service answers what a title asks the kernel about itself.
///
/// This is the one path every title goes through, and it is the one where
/// answering nothing looks exactly like answering correctly: a title told
/// nothing carries on and goes wrong somewhere else. What is checked is that an
/// answered import is answered and an unanswered one is not, which is what says
/// the switch is reached at all.
#[test]
fn the_import_service_answers_the_kernel_about_itself() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    /* What kind of process this is. A title is not the kernel and not the idle
     * process, so the answer is the user one, which is one. */
    xenolith_context ctx = {0};
    xenolith_import(&ctx, base, "xboxkrnl.exe", 102, "PsGetCurrentProcessType");
    if (ctx.r[3] != 1u) {
        fprintf(stderr, "the process type came back as %llu\n",
                (unsigned long long)ctx.r[3]);
        return 3;
    }

    /* And the same for the answer that comes from the processor block, which is
     * a different part of the same service. */
    xenolith_import(&ctx, base, "xboxkrnl.exe", 338, "KeTlsAlloc");
    if (ctx.r[3] == 0xFFFFFFFFu) {
        fprintf(stderr, "no thread local slot could be taken\n");
        return 4;
    }
    return 0;
}
"#;
    if let Err(complaint) = runs("imports", program) {
        panic!("the import service did not answer the kernel:\n{complaint}");
    }
}

/// Writing to a file and reading it back.
///
/// A title that saves has to be able to read what it saved, and the two paths
/// are separate enough that one working says nothing about the other. What is
/// checked is the bytes on disk, which is what a later run reads.
#[test]
fn a_file_written_is_a_file_read_back() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>
#include <string.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

static void name_a_file(uint8_t *base, uint32_t at, const char *name) {
    size_t length = strlen(name);
    xenolith_store32(base, at + 4, at + 0x40);
    xenolith_store16(base, at + 0x40, (uint16_t)length);
    xenolith_store32(base, at + 0x44, at + 0x80);
    for (size_t letter = 0; letter < length; letter++) {
        base[at + 0x80 + letter] = (uint8_t)name[letter];
    }
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    xenolith_context ctx = {0};
    name_a_file(base, 0x1000, "game:\\written.txt");
    ctx.r[3] = 0x2000;
    ctx.r[4] = 0x40000000u;  /* may write */
    ctx.r[5] = 0x1000;
    ctx.r[6] = 0x2100;
    ctx.r[8] = 0;
    ctx.r[10] = 3;           /* open or make */
    xenolith_import(&ctx, base, "xboxkrnl.exe", 210, "NtCreateFile");
    if (ctx.r[3] != 0u) {
        fprintf(stderr, "making failed with %#llx\n", (unsigned long long)ctx.r[3]);
        return 3;
    }
    uint32_t handle = xenolith_load32(base, 0x2000);
    if (handle == 0) {
        return 4;
    }

    const char *wrote = "written by the test";
    memcpy(base + 0x3000, wrote, strlen(wrote));
    xenolith_store64(base, 0x2400, 0);
    ctx.r[3] = handle;
    ctx.r[4] = 0;
    ctx.r[7] = 0x2500;
    ctx.r[8] = 0x3000;
    ctx.r[9] = (uint32_t)strlen(wrote);
    ctx.r[10] = 0x2400;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 255, "NtWriteFile");
    if (ctx.r[3] != 0u) {
        fprintf(stderr, "writing failed with %#llx\n", (unsigned long long)ctx.r[3]);
        return 5;
    }
    if (xenolith_load32(base, 0x2500 + 4) != (uint32_t)strlen(wrote)) {
        fprintf(stderr, "the write reported %u bytes\n",
                (unsigned) xenolith_load32(base, 0x2500 + 4));
        return 6;
    }
    return 0;
}
"#;
    if let Err(complaint) = runs("writing", program) {
        panic!("writing a file did not work:\n{complaint}");
    }

    /* The bytes on disk, which is what a later run reads. */
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("writing");
    let written = std::fs::read(directory.join("written.txt"))
        .unwrap_or_else(|error| panic!("the file was not written: {error}"));
    assert_eq!(
        String::from_utf8_lossy(&written),
        "written by the test",
        "the file holds something other than what was written to it"
    );
}

/// A directory is read one thing at a time until it says there are no more.
///
/// A title reads what it has saved this way, so a directory query that answers
/// nothing makes every save look missing. What is checked is that the things in
/// it are named, and that the run ends with the console's own number for the
/// end rather than by going round for ever.
#[test]
#[allow(clippy::too_many_lines)]
fn a_directory_is_read_one_thing_at_a_time() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

static void name_a_file(uint8_t *base, uint32_t at, const char *name) {
    size_t length = strlen(name);
    xenolith_store32(base, at + 4, at + 0x40);
    xenolith_store16(base, at + 0x40, (uint16_t)length);
    xenolith_store32(base, at + 0x44, at + 0x80);
    for (size_t letter = 0; letter < length; letter++) {
        base[at + 0x80 + letter] = (uint8_t)name[letter];
    }
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    /* A directory beside the program, with two things in it. */
    mkdir("saved", 0777);
    FILE *one = fopen("saved/one.txt", "wb");
    if (one == NULL) {
        return 3;
    }
    fputs("one", one);
    fclose(one);
    FILE *two = fopen("saved/two.txt", "wb");
    if (two == NULL) {
        return 4;
    }
    fputs("two", two);
    fclose(two);

    xenolith_context ctx = {0};
    name_a_file(base, 0x1000, "game:\\saved");
    ctx.r[3] = 0x2000;
    ctx.r[4] = 0x80000000u;
    ctx.r[5] = 0x1000;
    ctx.r[6] = 0x2100;
    ctx.r[8] = 1;            /* a directory rather than a file */
    xenolith_import(&ctx, base, "xboxkrnl.exe", 223, "NtOpenFile");
    if (ctx.r[3] != 0u) {
        fprintf(stderr, "opening the directory failed with %#llx\n",
                (unsigned long long)ctx.r[3]);
        return 5;
    }
    uint32_t handle = xenolith_load32(base, 0x2000);
    if (handle == 0) {
        return 6;
    }

    /* Reading it out, one at a time, until it says there are no more. */
    int seen = 0;
    int saw_one = 0;
    int saw_two = 0;
    for (int at = 0; at < 10; at++) {
        xenolith_store32(base, 0x2300, at == 0 ? 1 : 0);
        ctx.r[3] = handle;
        ctx.r[7] = 0x2200;
        ctx.r[8] = 0x2400;
        ctx.r[9] = 0x200;
        xenolith_import(&ctx, base, "xboxkrnl.exe", 228, "NtQueryDirectoryFile");
        if (ctx.r[3] == 0x80000006u) {
            break;
        }
        if (ctx.r[3] != 0u) {
            fprintf(stderr, "the directory read failed with %#llx\n",
                    (unsigned long long)ctx.r[3]);
            return 7;
        }
        uint32_t how_long = xenolith_load32(base, 0x2400 + 0x3c);
        char name[64];
        for (uint32_t letter = 0; letter < how_long && letter < sizeof name - 1; letter++) {
            name[letter] = (char)base[0x2400 + 0x40 + letter];
        }
        name[how_long < sizeof name - 1 ? how_long : sizeof name - 1] = '\0';
        if (strcmp(name, "one.txt") == 0) {
            saw_one = 1;
        }
        if (strcmp(name, "two.txt") == 0) {
            saw_two = 1;
        }
        seen++;
    }

    if (!saw_one || !saw_two) {
        fprintf(stderr, "the directory held %d things, one.txt %d, two.txt %d\n", seen,
                saw_one, saw_two);
        return 8;
    }
    return 0;
}
"#;
    if let Err(complaint) = runs("directory", program) {
        panic!("a directory was not read one thing at a time:\n{complaint}");
    }
}

/// The runtime, run under the sanitizers that find what a test cannot.
///
/// Address finds what reads or writes outside what it owns, and the runtime
/// indexes a base pointer with a guest address and has no check anywhere, so the
/// whole class is one no test can catch. Undefined finds what the compiler was
/// entitled to assume and was not. Thread finds what two threads did to one thing
/// at once, and the wait queue, the object table and the thread-local slots are
/// all shared.
///
/// A compiler without these is not a failure: `gcc` on this host has none. What
/// would be a failure is reporting a pass from a run that never happened, so a
/// host with no such compiler reports the test as ignored, naming what is
/// missing.
#[test]
#[ignore = "needs a compiler with sanitizers"]
fn the_runtime_is_clean_under_the_sanitizers() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <pthread.h>
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

static uint8_t *where;
static uint32_t slot;

/* Asks the one question whose answer is said once, so that several threads
 * asking at once all race for that one report. */
static void *say_it_once(void *given) {
    (void)given;
    xenolith_context ctx = {0};
    xenolith_import(&ctx, where, "xboxkrnl.exe", 102, "PsGetCurrentProcessType");
    return NULL;
}

/* Two threads putting something in one thread-local slot and reading it back,
 * which is where a shared set shows up. */
static void *use_the_slot(void *given) {
    xenolith_context ctx = {0};
    ctx.r[3] = slot;
    ctx.r[4] = (uint32_t)(uintptr_t)given;
    xenolith_import(&ctx, where, "xboxkrnl.exe", 341, "KeTlsSetValue");
    ctx.r[3] = slot;
    xenolith_import(&ctx, where, "xboxkrnl.exe", 340, "KeTlsGetValue");
    return NULL;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    where = xenolith_boot(path, 0);
    if (where == NULL) {
        return 2;
    }

    xenolith_context ctx = {0};

    /* Guest memory, which indexes the base pointer by a guest address. */
    xenolith_store32(where, 0x100, 0);
    xenolith_store32(where, 0x104, 0x3000);
    ctx.r[3] = 0x100;
    ctx.r[4] = 0x104;
    ctx.r[8] = 0x20000;
    xenolith_import(&ctx, where, "xboxkrnl.exe", 204, "NtAllocateVirtualMemory");

    /* An object set, waited on with a timeout, and entered as a critical
     * section, which is the queue and the section table. */
    ctx.r[3] = 0x200;
    ctx.r[5] = 1;
    ctx.r[6] = 0;
    xenolith_import(&ctx, where, "xboxkrnl.exe", 209, "NtCreateEvent");
    uint32_t handle = xenolith_load32(where, 0x200);
    ctx.r[3] = handle;
    xenolith_import(&ctx, where, "xboxkrnl.exe", 246, "NtSetEvent");
    xenolith_store64(where, 0x300, (uint64_t)(int64_t)-100000);
    ctx.r[3] = handle;
    ctx.r[5] = 0x300;
    xenolith_import(&ctx, where, "xboxkrnl.exe", 253, "NtWaitForSingleObject");
    ctx.r[3] = 0x400;
    xenolith_import(&ctx, where, "xboxkrnl.exe", 293, "RtlEnterCriticalSection");
    ctx.r[3] = 0x400;
    xenolith_import(&ctx, where, "xboxkrnl.exe", 304, "RtlLeaveCriticalSection");

    /* The reporter, from two threads at once. Both ask the kernel what kind of
     * process this is, which is answered once and said once, so both race for
     * the one line it writes and for the bookkeeping that decides whether to
     * write it at all. */
    pthread_t saying[8];
    for (int at = 0; at < 8; at++) {
        if (pthread_create(&saying[at], NULL, say_it_once, NULL) != 0) {
            return 5;
        }
    }
    for (int at = 0; at < 8; at++) {
        pthread_join(saying[at], NULL);
    }

    /* The thread-local slots, from two threads at once. */
    xenolith_import(&ctx, where, "xboxkrnl.exe", 338, "KeTlsAlloc");
    slot = (uint32_t)ctx.r[3];
    pthread_t first, second;
    if (pthread_create(&first, NULL, use_the_slot, (void *)1) != 0) {
        return 3;
    }
    if (pthread_create(&second, NULL, use_the_slot, (void *)2) != 0) {
        return 4;
    }
    pthread_join(first, NULL);
    pthread_join(second, NULL);
    return 0;
}
"#;

    for sanitizer in ["address,undefined", "thread"] {
        if let Err(complaint) = runs_sanitized(sanitizer, program) {
            panic!("the runtime under {sanitizer}:\n{complaint}");
        }
    }
}

/// Builds and runs the program with one sanitizer, or says the host cannot.
fn runs_sanitized(sanitizer: &str, program: &str) -> Result<(), String> {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "sanitized_{}",
        sanitizer.replace(['-', '+', ','], "_")
    ));
    std::fs::create_dir_all(&directory).map_err(|error| format!("making a directory: {error}"))?;
    for (file, text) in [
        ("xenolith.h", HEADER),
        ("xenolith.c", include_str!("../runtime/xenolith.c")),
        /* The runtime includes this, so it has to be beside it. */
        (
            "command_stream.c",
            include_str!("../runtime/command_stream.c"),
        ),
        ("sound.c", include_str!("../runtime/sound.c")),
        ("drawing.c", include_str!("../runtime/drawing.c")),
        ("vulkan.c", include_str!("../runtime/vulkan.c")),
        /* The shader table, which the lift emits. A test has no image, so it
         * gets one with nothing in it. */
        ("shaders.c", SHADER_TABLE),
        ("main.c", program),
    ] {
        std::fs::write(directory.join(file), text)
            .map_err(|error| format!("writing {file}: {error}"))?;
    }

    /* A compiler that has the sanitizer. The host's own may not: `gcc` on this
     * machine has neither of them. */
    let flag = format!("-fsanitize={sanitizer}");
    /* A whole program, because the flag alone says nothing: linking is where a
     * sanitizer's runtime is found or not found. */
    let probe = directory.join("probe.c");
    std::fs::write(&probe, "int main(void) { return 0; }")
        .map_err(|error| format!("writing the probe: {error}"))?;
    let compiler = ["clang", "cc", "gcc"].into_iter().find(|candidate| {
        Command::new(candidate)
            .args(["-std=c17", &flag])
            .arg("-o")
            .arg(directory.join("probe"))
            .arg(&probe)
            .output()
            .is_ok_and(|output| output.status.success())
    });
    let Some(compiler) = compiler else {
        return Err(format!(
            "no compiler on this host can build with {flag}, so this check could not run"
        ));
    };

    let program_path = directory.join("program");
    let output = Command::new(compiler)
        .args(["-std=c17", "-g", &flag, "-fno-omit-frame-pointer"])
        .arg("-o")
        .arg(&program_path)
        .arg(directory.join("main.c"))
        .arg(directory.join("xenolith.c"))
        .arg(directory.join("vulkan.c"))
        .arg(directory.join("shaders.c"))
        .args(["-lvulkan", "-lm", "-lpthread"])
        .output()
        .map_err(|error| format!("running the compiler: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }

    let ran = Command::new(&program_path)
        .current_dir(&directory)
        .output()
        .map_err(|error| format!("running the check: {error}"))?;
    let complaint = String::from_utf8_lossy(&ran.stderr).into_owned();
    if ran.status.success() && !complaint.contains("ERROR: ") {
        return Ok(());
    }
    Err(format!(
        "check {} failed:\n{complaint}",
        ran.status.code().unwrap_or(-1)
    ))
}

/// The commands a title writes are read, and a register they set holds what
/// they set it to.
///
/// A title does not call the hardware: it writes packets into a ring buffer and
/// waits for a pointer to move. A runtime that does not read the buffer draws
/// nothing and, worse, leaves the registers a title set holding whatever was
/// there before, so a title reading one back gets an answer nobody gave.
#[test]
fn a_packet_in_the_ring_sets_a_register() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>
#include <time.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    xenolith_context ctx = {0};

    /* A ring buffer with one packet in it: a run of one register, index 0x20,
     * set to 0xdeadbeef. The header's top two bits are nought for a run of
     * registers, the next fourteen say how many words follow, and the low
     * fifteen are the register it starts at. */
    uint32_t ring = 0x01000000u;
    /* The field is one less than how many words follow the header, so nought
     * is one word after it. */
    uint32_t header = (0u << 30) | (0u << 16) | 0x20u;
    xenolith_store32(base, ring, header);
    xenolith_store32(base, ring + 4, 0xdeadbeefu);

    /* The title says where the ring is, and how big, as a power of two. */
    ctx.r[3] = ring;
    ctx.r[4] = 16;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 451, "NtSetCommandBuffer");
    if (ctx.r[3] != 0u) {
        return 3;
    }

    /* Where it reads progress from. The block begins sixty bytes before the
     * address it names, and the word at its front is how far the hardware has
     * worked through what it was given. */
    uint32_t read_back = 0x02000000u + 60u;
    ctx.r[3] = read_back;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 438, "NtSetProgressAddress");
    if (ctx.r[3] != 0u) {
        return 4;
    }

    /* Publishing the write pointer is what tells the hardware there is
     * something to read: two words, the header and its one word. */
    xenolith_store32(base, 0x7fc80714u, 2u);

    /* The register is written by a thread that wakes a hundred times a
     * millisecond, so it is waited for rather than read once. */
    for (int waited = 0; waited < 2000; waited++) {
        if (xenolith_load32(base, 0x7fc80000u + 0x20u * 4u) == 0xdeadbeefu) {
            printf("read\n");
            return 0;
        }
        struct timespec once = {0, 1000000};
        nanosleep(&once, NULL);
    }
    fprintf(stderr, "the register still holds %#x\n",
            xenolith_load32(base, 0x7fc80000u + 0x20u * 4u));
    return 5;
}
"#;
    if let Err(complaint) = runs("ring", program) {
        panic!("a packet in the ring did not set a register:\n{complaint}");
    }
}

/// An import the runtime answers says it succeeded.
///
/// The status is `r[3]`, which is also where the first argument arrives. An
/// import that answers without writing a status therefore leaves the title
/// looking at whatever it passed in, which for most of these is an address. A
/// title comparing that against success is told the call failed and does
/// nothing further, so a title stops without anything having gone wrong.
#[test]
fn an_answered_import_says_it_succeeded() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

/* Asks for one import with an argument that is not nought, and reports what
 * came back. */
static uint32_t ask(xenolith_context *ctx, uint8_t *base, uint32_t ordinal,
                    const char *name, uint32_t argument) {
    ctx->r[3] = argument;
    xenolith_import(ctx, base, "xboxkrnl.exe", ordinal, name);
    return (uint32_t)ctx->r[3];
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    xenolith_context ctx = {0};

    /* The command processor, told where the ring buffer is and where progress
     * goes. Both take an address in the first argument. */
    if (ask(&ctx, base, 450, "VdStartCommandProcessor", 0) != 0u) {
        fprintf(stderr, "starting the command processor answered %#llx\n",
                (unsigned long long)ctx.r[3]);
        return 3;
    }
    if (ask(&ctx, base, 451, "VdInitializeRingBuffer", 0x01000000u) != 0u) {
        fprintf(stderr, "setting the ring buffer answered %#llx\n",
                (unsigned long long)ctx.r[3]);
        return 4;
    }
    if (ask(&ctx, base, 438, "VdEnableRingBufferRPtrWriteBack", 0x02000000u) != 0u) {
        fprintf(stderr, "setting the progress address answered %#llx\n",
                (unsigned long long)ctx.r[3]);
        return 5;
    }

    /* The one that does nothing at all, which is still an answer. */
    if (ask(&ctx, base, 479, "the one that does nothing", 0x40u) != 0u) {
        fprintf(stderr, "doing nothing answered %#llx\n",
                (unsigned long long)ctx.r[3]);
        return 6;
    }
    return 0;
}
"#;
    if let Err(complaint) = runs("answered", program) {
        panic!("an answered import did not say it succeeded:\n{complaint}");
    }
}

/// A load packet fills the block it names, not the front of the register file.
///
/// A title hands its shaders their constants by writing them where it likes and
/// then saying to load them into the constant block. The number it names is
/// counted from the front of that block rather than from the front of the
/// register file, so a load written without the block's own start puts every
/// constant eight thousand registers short of where the shader looks for it, and
/// the shader computes from nought.
#[test]
fn a_load_fills_the_block_it_names() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>
#include <time.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) { return 1; }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) { return 2; }

    /* A value to load, and a ring holding one load packet: opcode 0x2f, four
     * words after the header. The third word names constant 3 of the constant
     * block, which is register 0x4003. */
    uint32_t value_at = 0x01000000u;
    xenolith_store32(base, value_at, 0x3f800000u);

    uint32_t ring = 0x01100000u;
    /* Three words after the header, and the field is one less than that. */
    xenolith_store32(base, ring, (3u << 30) | (2u << 16) | (0x2fu << 8));
    xenolith_store32(base, ring + 4, value_at);
    xenolith_store32(base, ring + 8, 3u);          /* constant 3 */
    xenolith_store32(base, ring + 12, 1u);         /* one word */

    xenolith_context ctx = {0};
    ctx.r[3] = ring;
    ctx.r[4] = 16;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 451, "VdInitializeRingBuffer");
    ctx.r[3] = 0x02000000u + 60u;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 438, "VdEnableRingBufferRPtrWriteBack");

    xenolith_store32(base, 0x7fc80714u, 4u);

    for (int waited = 0; waited < 2000; waited++) {
        if (xenolith_load32(base, 0x7fc80000u + 0x4003u * 4u) == 0x3f800000u) {
            /* And nothing was written to the front of the file, which is where
             * a load written without the block's start would have put it. */
            if (xenolith_load32(base, 0x7fc80000u + 3u * 4u) != 0u) {
                fprintf(stderr, "the load landed at the front of the file\n");
                return 3;
            }
            printf("loaded\n");
            return 0;
        }
        struct timespec once = {0, 1000000};
        nanosleep(&once, NULL);
    }
    fprintf(stderr, "constant 3 holds %#x, and the front holds %#x\n",
            xenolith_load32(base, 0x7fc80000u + 0x4003u * 4u),
            xenolith_load32(base, 0x7fc80000u + 3u * 4u));
    return 4;
}
"#;
    if let Err(complaint) = runs("load_block", program) {
        panic!("a load did not fill the block it names:\n{complaint}");
    }
}

/// A report packet writes its value where it was asked to.
///
/// A title reports that the hardware hung and waits for an answer at a fixed
/// address. Without the answer it waits for ever, which is indistinguishable
/// from a hang. The value is written as it was given, and a byte order this
/// does not know is reported rather than written in the wrong way round, since
/// a number a title compares against and never matches is the same as no
/// number at all except that it looks like one.
#[test]
fn a_report_packet_writes_its_value() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>
#include <time.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    xenolith_context ctx = {0};

    /* Two report packets: one asking for the byte order a console's own reads
     * want, and one asking for another. */
    uint32_t ring = 0x01000000u;
    uint32_t at = ring;
    uint32_t answers = 0x04000000u;

    /* The first: opcode 0x58, three words following. The field is one less than
     * that, which is what the walk reads it as. */
    xenolith_store32(base, at, (3u << 30) | (2u << 16) | (0x58u << 8));
    xenolith_store32(base, at + 4, 0u);
    xenolith_store32(base, at + 8, answers | 2u);
    xenolith_store32(base, at + 12, 0xcafef00du);
    at += 16;

    /* The second, asking for byte order 1. */
    xenolith_store32(base, at, (3u << 30) | (2u << 16) | (0x58u << 8));
    xenolith_store32(base, at + 4, 0u);
    xenolith_store32(base, at + 8, (answers + 4u) | 1u);
    xenolith_store32(base, at + 12, 0xcafef00du);
    at += 16;

    ctx.r[3] = ring;
    ctx.r[4] = 16;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 451, "VdInitializeRingBuffer");
    uint32_t read_back = 0x02000000u + 60u;
    ctx.r[3] = read_back;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 438, "VdEnableRingBufferRPtrWriteBack");

    /* Two packets of four words each. */
    xenolith_store32(base, 0x7fc80714u, 8u);

    /* Both packets are walked in one pass, so the first is waited for and the
     * second has been reached by the time it appears. */
    for (int waited = 0; waited < 2000; waited++) {
        if (xenolith_load32(base, answers) == 0xcafef00du) {
            break;
        }
        struct timespec once = {0, 1000000};
        nanosleep(&once, NULL);
    }
    if (xenolith_load32(base, answers) != 0xcafef00du) {
        fprintf(stderr, "the report was not written, the word holds %#x\n",
                xenolith_load32(base, answers));
        return 4;
    }

    /* The one asking for another order must not have written. */
    if (xenolith_load32(base, answers + 4u) != 0u) {
        fprintf(stderr, "a report in an unknown byte order was written\n");
        return 3;
    }
    printf("read\n");
    return 0;
}
"#;
    if let Err(complaint) = runs("report_packet", program) {
        panic!("a report packet did not write its value:\n{complaint}");
    }

    /* The unknown byte order is named once in the output. The run is given a
     * moment to reach the second packet, which is the one that asks for it. */
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("report_packet");
    let said = Command::new(directory.join("program"))
        .current_dir(&directory)
        .output()
        .unwrap_or_else(|error| panic!("running the check: {error}"));
    let complaint = String::from_utf8_lossy(&said.stderr);
    assert!(
        complaint.contains("only 2 is written"),
        "an unknown byte order was not reported:\n{complaint}"
    );
}

/// A privilege is answered from the flags the container declares.
///
/// A title asks whether it holds a numbered privilege before it does the thing,
/// and the answer is a bit in the system flags word its own executable carries.
/// Answering no to everything is not a safe default: the title tried here asks,
/// is told no, and shuts itself down before it draws anything.
///
/// Both halves are checked. A bit the container sets is reported as held, and a
/// bit it does not is reported as not held, so this cannot pass by answering yes
/// to everything either.
#[test]
fn a_privilege_is_answered_from_the_declared_flags() {
    let program = r#"
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

static uint8_t *base_kept;

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) { return 1; }
    fputc('x', made);
    fclose(made);
    base_kept = xenolith_boot(path, 0);
    if (base_kept == NULL) { return 2; }

    /* Bits nine and ten set, which is what one title's container declares. */
    xenolith_privileges(0x600u);

    xenolith_context ctx = {0};
    for (uint32_t bit = 0; bit < 32; bit++) {
        ctx.r[3] = bit;
        xenolith_import(&ctx, base_kept, "xboxkrnl.exe", 404,
                        "XexCheckExecutablePrivilege");
        uint64_t wanted = (bit == 9 || bit == 10) ? 1u : 0u;
        if (ctx.r[3] != wanted) {
            fprintf(stderr, "privilege %u was answered %llu where %llu was declared\n",
                    bit, (unsigned long long)ctx.r[3], (unsigned long long)wanted);
            return 3;
        }
    }
    printf("every bit answered\n");
    return 0;
}
"#;
    if let Err(complaint) = runs("privileges", program) {
        panic!("a privilege was not answered from the declared flags:\n{complaint}");
    }
}

/// The two console settings a title's startup checks are answered with what it
/// requires, and every other setting is still answered with nothing.
///
/// A setting is a property of the console rather than of the title, so the
/// runtime has to choose an answer. Choosing nothing is not neutral: one title's
/// startup reads two of them, and refuses to start for any answer but the one it
/// compares against. Those two numbers were read out of that check.
#[test]
fn the_settings_a_startup_checks_are_answered_with_what_it_requires() {
    let program = r#"
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

static uint8_t *base_kept;

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

static int ask(xenolith_context *ctx, uint32_t category, uint32_t setting,
               uint32_t *answer) {
    uint32_t into = 0x1000u;
    uint32_t needed = 0x2000u;
    xenolith_store32(base_kept, into, 0xdeadbeefu);
    ctx->r[3] = category;
    ctx->r[4] = setting;
    ctx->r[5] = into;
    ctx->r[6] = 4;
    ctx->r[7] = needed;
    xenolith_import(ctx, base_kept, "xboxkrnl.exe", 16, "ExGetXConfigSetting");
    if (ctx->r[3] != 0u) {
        return 0;
    }
    *answer = xenolith_load32(base_kept, into);
    return 1;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) { return 1; }
    fputc('x', made);
    fclose(made);
    base_kept = xenolith_boot(path, 0);
    if (base_kept == NULL) { return 2; }

    xenolith_context ctx = {0};
    uint32_t answer = 0;

    /* The display setting: bits eight to fifteen have to be three, which is
     * what the title's own mask reads. */
    if (!ask(&ctx, 2, 2, &answer)) { return 3; }
    if (((answer >> 8) & 0xffu) != 3u) {
        fprintf(stderr, "the display setting answered %#x, and the title wants 3 "
                        "in bits eight to fifteen\n", answer);
        return 4;
    }

    /* The setting whose ninth bit the title requires. */
    if (!ask(&ctx, 3, 10, &answer)) { return 5; }
    if (((answer >> 8) & 0x2u) == 0u) {
        fprintf(stderr, "the ninth bit of the display answer is clear: %#x\n", answer);
        return 6;
    }

    /* And anything else is still nothing, rather than the answer above. */
    if (!ask(&ctx, 9, 9, &answer)) { return 7; }
    if (answer != 0u) {
        fprintf(stderr, "an unasked setting was answered %#x\n", answer);
        return 8;
    }

    printf("settings answered\n");
    return 0;
}
"#;
    if let Err(complaint) = runs("settings", program) {
        panic!("the settings a startup checks were not answered:\n{complaint}");
    }
}

/// An unimplemented import names where it was wanted from.
///
/// Which import a title wants says what it was trying to do; where it asked says
/// which part of the title that was. Without the second, a report of an
/// unimplemented entry point sends a reader through a title with nothing to
/// start from, and the title's own address is the only handle there is.
#[test]
fn an_unimplemented_import_names_where_it_was_wanted_from() {
    let program = r#"
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) { return 1; }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) { return 2; }

    /* An ordinal nothing answers, called from an address of the title's own. */
    xenolith_context ctx = {0};
    ctx.lr = 0x82119448u;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 82, "KeBugCheck");

    fprintf(stderr, "the run carried on past an import nothing answers\n");
    return 3;
}
"#;
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("wanted_from");
    std::fs::create_dir_all(&directory).expect("making a directory");
    for (file, text) in [
        ("xenolith.h", HEADER),
        ("xenolith.c", include_str!("../runtime/xenolith.c")),
        (
            "command_stream.c",
            include_str!("../runtime/command_stream.c"),
        ),
        ("sound.c", include_str!("../runtime/sound.c")),
        ("drawing.c", include_str!("../runtime/drawing.c")),
        ("vulkan.c", include_str!("../runtime/vulkan.c")),
        ("shaders.c", SHADER_TABLE),
        ("main.c", program),
    ] {
        std::fs::write(directory.join(file), text).expect("writing a file");
    }
    let program_path = directory.join("program");
    let built = Command::new(honest::host_compiler())
        .args(["-std=c17", "-O1"])
        .arg("-o")
        .arg(&program_path)
        .arg(directory.join("main.c"))
        .arg(directory.join("xenolith.c"))
        .arg(directory.join("vulkan.c"))
        .arg(directory.join("shaders.c"))
        .args(["-lvulkan", "-lm", "-lpthread"])
        .output()
        .expect("running the compiler");
    assert!(
        built.status.success(),
        "the wanted-from test did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let ran = Command::new(&program_path)
        .current_dir(&directory)
        .output()
        .expect("running the check");
    let complaint = String::from_utf8_lossy(&ran.stderr);

    assert!(
        complaint.contains("KeBugCheck"),
        "the import should be named: {complaint}"
    );
    assert!(
        complaint.contains("0x82119448"),
        "the address it was wanted from should be named: {complaint}"
    );
}

/// A wait that is given a length actually waits.
///
/// The console takes the length in the fifth argument. Reading it from the
/// fourth instead returns at once, because the fourth holds the reason for
/// waiting rather than a length. A title pacing itself then spins as fast as the
/// host allows, and one that polls a worker through this spins with it, which
/// looks like a title doing nothing at all rather than one waiting.
#[test]
fn a_wait_given_a_length_actually_waits() {
    let program = r#"
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>
#include <time.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

/* A length of `hundreds` hundreds of nanoseconds, negative for a stretch from
 * now, written where the console reads it. */
static void ask_to_wait(xenolith_context *ctx, uint8_t *base, int64_t hundreds) {
    uint32_t at = 0x300;
    xenolith_store32(base, at, (uint32_t)((uint64_t)hundreds >> 32));
    xenolith_store32(base, at + 4, (uint32_t)(uint64_t)hundreds);
    ctx->r[3] = 1;
    ctx->r[4] = 0;
    ctx->r[5] = at;
    xenolith_import(ctx, base, "xboxkrnl.exe", 90, "KeDelayExecutionThread");
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) { return 1; }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) { return 2; }

    xenolith_context ctx = {0};

    /* A tenth of a second, which is ten million hundreds of nanoseconds. */
    struct timespec before, after;
    clock_gettime(CLOCK_MONOTONIC, &before);
    ask_to_wait(&ctx, base, -10000000);
    clock_gettime(CLOCK_MONOTONIC, &after);

    long long waited = (long long)(after.tv_sec - before.tv_sec) * 1000
                       + (after.tv_nsec - before.tv_nsec) / 1000000;
    if (waited < 50) {
        fprintf(stderr, "a wait of a tenth of a second took %lld milliseconds\n",
                waited);
        return 3;
    }
    printf("waited %lld milliseconds\n", waited);
    return 0;
}
"#;
    if let Err(complaint) = runs("wait_length", program) {
        panic!("a wait given a length did not wait:\n{complaint}");
    }
}

/// A run that is stopped from outside says what it read before it goes.
///
/// The run this matters for is the one that never ends: a title spinning in its
/// own code is stopped by whatever started it, and the counts a reader most wants
/// are the ones that would otherwise never be printed. `atexit` does not run on a
/// signal, so the handler is registered at boot, where every run goes through,
/// rather than from the thread that reads the command stream, which a spinning
/// title may never have started.
#[test]
fn a_run_stopped_from_outside_says_what_it_read() {
    let program = r#"
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <signal.h>
#include <stdio.h>
#include <unistd.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) { return 1; }
    fputc('x', made);
    fclose(made);
    if (xenolith_boot(path, 0) == NULL) { return 2; }

    /* Spin here, as a title in a loop of its own would, and let the signal
     * arrive. */
    for (;;) {
        pause();
    }
}
"#;
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("stopped");
    std::fs::create_dir_all(&directory).expect("making a directory");
    for (file, text) in [
        ("xenolith.h", HEADER),
        ("xenolith.c", include_str!("../runtime/xenolith.c")),
        (
            "command_stream.c",
            include_str!("../runtime/command_stream.c"),
        ),
        ("sound.c", include_str!("../runtime/sound.c")),
        ("drawing.c", include_str!("../runtime/drawing.c")),
        ("vulkan.c", include_str!("../runtime/vulkan.c")),
        ("shaders.c", SHADER_TABLE),
        ("main.c", program),
    ] {
        std::fs::write(directory.join(file), text).expect("writing a file");
    }
    let program_path = directory.join("program");
    let built = Command::new(honest::host_compiler())
        .args(["-std=c17", "-O1"])
        .arg("-o")
        .arg(&program_path)
        .arg(directory.join("main.c"))
        .arg(directory.join("xenolith.c"))
        .arg(directory.join("vulkan.c"))
        .arg(directory.join("shaders.c"))
        .args(["-lvulkan", "-lm", "-lpthread"])
        .output()
        .expect("running the compiler");
    assert!(
        built.status.success(),
        "the stopped test did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let child = Command::new(&program_path)
        .current_dir(&directory)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("starting the program");
    std::thread::sleep(std::time::Duration::from_millis(400));
    let killed = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("sending the signal");
    assert!(killed.success(), "the signal should have been sent");

    let ended = child.wait_with_output().expect("waiting for the program");
    let complaint = String::from_utf8_lossy(&ended.stderr);

    assert!(
        complaint.contains("in total, read"),
        "a run stopped from outside should say what it read: {complaint}"
    );
}

/// A raised exception is reported with what was raised, and the run stops.
///
/// A title reaching this has decided something is wrong and is handing over a
/// record saying what. Nothing here can unwind a title's stack the way the
/// console's dispatcher would, so the exception is not delivered. Pretending it
/// was would leave the title running code written for a state it is not in,
/// which is worse than stopping. What is done instead is reporting the code and
/// the address, because that is the only thing that says why the title gave up.
#[test]
fn a_raised_exception_is_reported_and_the_run_stops() {
    let program = r#"
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) { return 1; }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) { return 2; }

    /* A record of three words: the code a C++ throw uses, the flags, and the
     * address. */
    xenolith_store32(base, 0x200, 0xe06d7363u);
    xenolith_store32(base, 0x204, 1);
    xenolith_store32(base, 0x208, 0x82123456u);

    xenolith_context ctx = {0};
    ctx.r[3] = 0x200;
    ctx.lr = 0x82392c98u;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 310, "RtlRaiseException");

    /* Reaching here means the exception was swallowed rather than reported. */
    fprintf(stderr, "the run carried on past a raised exception\n");
    return 3;
}
"#;
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("raised");
    std::fs::create_dir_all(&directory).expect("making a directory");
    for (file, text) in [
        ("xenolith.h", HEADER),
        ("xenolith.c", include_str!("../runtime/xenolith.c")),
        (
            "command_stream.c",
            include_str!("../runtime/command_stream.c"),
        ),
        ("sound.c", include_str!("../runtime/sound.c")),
        ("drawing.c", include_str!("../runtime/drawing.c")),
        ("vulkan.c", include_str!("../runtime/vulkan.c")),
        ("shaders.c", SHADER_TABLE),
        ("main.c", program),
    ] {
        std::fs::write(directory.join(file), text).expect("writing a file");
    }
    let program_path = directory.join("program");
    let built = Command::new(honest::host_compiler())
        .args(["-std=c17", "-O1"])
        .arg("-o")
        .arg(&program_path)
        .arg(directory.join("main.c"))
        .arg(directory.join("xenolith.c"))
        .arg(directory.join("vulkan.c"))
        .arg(directory.join("shaders.c"))
        .args(["-lvulkan", "-lm", "-lpthread"])
        .output()
        .expect("running the compiler");
    assert!(
        built.status.success(),
        "the raised exception test did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let ran = Command::new(&program_path)
        .current_dir(&directory)
        .output()
        .expect("running the check");
    let complaint = String::from_utf8_lossy(&ran.stderr);

    assert!(
        complaint.contains("raised an exception 0xe06d7363"),
        "the code that was raised should be named: {complaint}"
    );
    assert!(
        complaint.contains("0x82123456"),
        "the address in the record should be named: {complaint}"
    );
    assert_eq!(
        ran.status.code(),
        Some(3),
        "a raised exception should stop the run: {complaint}"
    );
}

/// A run with no sound device says so once and carries on.
///
/// A title hands over a frame and expects to be told it was taken, because that
/// answer is the clock it paces itself by. A runtime that stops at the first
/// frame draws nothing further, and one that says nothing leaves a run looking
/// as though it had sound in it. So the frame is taken, the count moves, and the
/// one line says what is not happening.
#[test]
fn a_run_with_no_sound_device_says_so_once() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>
#include <time.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) {
        return 1;
    }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) {
        return 2;
    }

    xenolith_context ctx = {0};

    /* A frame of six channels, every sample a half, which is a sound a device
     * would play. */
    uint32_t frame = 0x01000000u;
    for (unsigned channel = 0; channel < 6; channel++) {
        for (unsigned sample = 0; sample < 256; sample++) {
            uint32_t bits = 0x3f000000u;
            bits = __builtin_bswap32(bits);
            xenolith_store32(base, frame + (channel * 256u + sample) * 4u, bits);
        }
    }

    /* Handed over three times, and taken every time. */
    for (int at = 0; at < 3; at++) {
        ctx.r[3] = 0;
        ctx.r[4] = frame;
        xenolith_import(&ctx, base, "xboxkrnl.exe", 501, "XAudioSubmitRenderFrame");
        if (ctx.r[3] != 0u) {
            fprintf(stderr, "a frame was not taken: %#llx\n",
                    (unsigned long long)ctx.r[3]);
            return 3;
        }
    }

    /* And how far playback has got is readable, whether or not there is a
     * device to have played anything. */
    ctx.r[3] = 0;
    xenolith_import(&ctx, base, "xboxkrnl.exe", 844, "XAudioGetRenderingFrame");
    if (ctx.r[3] > 0xffffffffu) {
        return 4;
    }
    return 0;
}
"#;
    if let Err(complaint) = runs("no_device", program) {
        panic!("a run with no sound device did not carry on:\n{complaint}");
    }

    /* The one line names what is missing, and says it once. */
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no_device");
    let said = Command::new(directory.join("program"))
        .current_dir(&directory)
        .output()
        .unwrap_or_else(|error| panic!("running the check: {error}"));
    let complaint = String::from_utf8_lossy(&said.stderr);
    let mentions = complaint.matches("no sound will be heard").count();
    assert_eq!(
        mentions, 1,
        "a run with no device said so {mentions} times and should say it once:\n{complaint}"
    );
}

/// A run with no window can be told what is held.
///
/// A run that is not shown has nothing to read keys from, and a title that only
/// ever sees nothing held cannot be taken past the screen that waits for a
/// button. `XENOLITH_HOLD` is that: the number a title would be given, held for
/// the length of the run.
#[test]
fn a_run_with_no_window_can_be_told_what_is_held() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>
#include <stdlib.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) { return 1; }
    fputc('x', made);
    fclose(made);
    if (xenolith_boot(path, 0) == NULL) { return 2; }

    /* What the run was told to hold, as the number it was told in. Nought
     * means nothing was told, which is a truthful answer rather than a
     * failure: a port that answers with nothing pressed. */
    uint16_t buttons = 0xffff;
    int16_t thumbs[4] = {9, 9, 9, 9};
    if (!xenolith_what_is_held(&buttons, thumbs)) {
        fprintf(stderr, "a controller that is there was said not to be\n");
        return 3;
    }
    uint16_t wanted = (uint16_t)strtoul(getenv("WANTED") == NULL ? "0" : getenv("WANTED"), 0, 0);
    if (buttons != wanted) {
        fprintf(stderr, "held %#x where %#x was asked for\n", buttons, wanted);
        return 4;
    }
    printf("held %#x\n", buttons);
    return 0;
}
"#;

    for wanted in ["0x0000", "0x1000", "0x0003"] {
        let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("held_{wanted}"));
        std::fs::create_dir_all(&directory).expect("making a directory");
        for (file, text) in [
            ("xenolith.h", HEADER),
            ("xenolith.c", include_str!("../runtime/xenolith.c")),
            (
                "command_stream.c",
                include_str!("../runtime/command_stream.c"),
            ),
            ("drawing.c", include_str!("../runtime/drawing.c")),
            ("sound.c", include_str!("../runtime/sound.c")),
            ("vulkan.c", include_str!("../runtime/vulkan.c")),
            ("shaders.c", SHADER_TABLE),
            ("main.c", program),
        ] {
            std::fs::write(directory.join(file), text).expect("writing a file");
        }
        let program_path = directory.join("program");
        let built = Command::new(honest::host_compiler())
            .args([
                "-std=c17",
                "-O1",
                "-DXENOLITH_WITH_VULKAN",
                "-DXENOLITH_WITH_WINDOW",
                "-DXENOLITH_WITH_X11",
            ])
            .arg("-o")
            .arg(&program_path)
            .arg(directory.join("main.c"))
            .arg(directory.join("xenolith.c"))
            .arg(directory.join("vulkan.c"))
            .arg(directory.join("shaders.c"))
            .args(["-lvulkan", "-lX11", "-lm", "-lpthread"])
            .output()
            .expect("running the compiler");
        assert!(
            built.status.success(),
            "the held test did not build:\n{}",
            String::from_utf8_lossy(&built.stderr)
        );

        let ran = Command::new(&program_path)
            .current_dir(&directory)
            .env("XENOLITH_HOLD", wanted)
            .env("WANTED", wanted)
            .output()
            .expect("running the check");
        assert!(
            ran.status.success(),
            "a run told to hold {wanted} went wrong, {}:\n{}",
            ran.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&ran.stderr)
        );
    }
}

/// A run with no way to show a frame says so once and carries on.
///
/// A machine with no display, or one asked to show nothing, is a limit of the
/// host and not a fault in the title. A line that only says what is missing
/// leaves a reader unable to tell whether the program gave up, so the line says
/// both, and it is said once however many frames the run goes on to make.
#[test]
fn a_run_with_no_window_says_so_once_and_carries_on() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) { return 1; }
    fputc('x', made);
    fclose(made);
    if (xenolith_boot(path, 0) == NULL) { return 2; }

    /* Asked for several times, because the line is about the host and saying it
     * once per frame would bury it. */
    for (int frame = 0; frame < 3; frame++) {
        if (!xenolith_show_the_frame()) {
            fprintf(stderr, "showing a frame reported failure\n");
            return 3;
        }
    }
    printf("still running\n");
    return 0;
}
"#;

    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no_frame");
    std::fs::create_dir_all(&directory).expect("making a directory");
    for (file, text) in [
        ("xenolith.h", HEADER),
        ("xenolith.c", include_str!("../runtime/xenolith.c")),
        (
            "command_stream.c",
            include_str!("../runtime/command_stream.c"),
        ),
        ("drawing.c", include_str!("../runtime/drawing.c")),
        ("sound.c", include_str!("../runtime/sound.c")),
        ("vulkan.c", include_str!("../runtime/vulkan.c")),
        ("shaders.c", SHADER_TABLE),
        ("main.c", program),
    ] {
        std::fs::write(directory.join(file), text).expect("writing a file");
    }

    let program_path = directory.join("program");
    let built = Command::new(honest::host_compiler())
        .args([
            "-std=c17",
            "-O1",
            "-DXENOLITH_WITH_VULKAN",
            "-DXENOLITH_WITH_WINDOW",
            "-DXENOLITH_WITH_X11",
        ])
        .arg("-o")
        .arg(&program_path)
        .arg(directory.join("main.c"))
        .arg(directory.join("xenolith.c"))
        .arg(directory.join("vulkan.c"))
        .arg(directory.join("shaders.c"))
        .args(["-lvulkan", "-lX11", "-lm", "-lpthread"])
        .output()
        .expect("running the compiler");
    assert!(
        built.status.success(),
        "the no frame test did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let ran = Command::new(&program_path)
        .current_dir(&directory)
        .env("XENOLITH_NO_WINDOW", "1")
        .output()
        .expect("running the check");
    assert!(
        ran.status.success(),
        "a run with no window went wrong, {}:\n{}",
        ran.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&ran.stderr)
    );

    let complaint = String::from_utf8_lossy(&ran.stderr);
    let mentions = complaint.matches("no frame will be shown").count();
    assert_eq!(
        mentions, 1,
        "a run with no window said so {mentions} times and should say it once:\n{complaint}"
    );
    assert!(
        complaint.contains("the run carries on"),
        "the line about the window should say the run carries on:\n{complaint}"
    );
    let said = String::from_utf8_lossy(&ran.stdout);
    assert!(
        said.contains("still running"),
        "the run should have carried on past the frames it could not show: {said}"
    );
}

/// A full vertex arena refuses the draw rather than wrapping to the front.
///
/// Wrapping would have the draw read what an earlier draw in the same frame
/// wrote: the geometry is there, the vertices are somebody else's, and what
/// appears is a shape nobody asked for. A draw left out is a gap that can be
/// seen.
#[test]
fn a_full_vertex_arena_refuses_rather_than_wraps() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) { return 1; }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) { return 2; }
    if (!xenolith_can_draw()) {
        printf("no device\n");
        return 0;
    }

    /* A frame's worth of draws, each taking a whole draw's room, until one does
     * not fit. */
    int refused = 0;
    for (int at = 0; at < 64; at++) {
        uint32_t room = 0;
        uint32_t *window = xenolith_room_for_vertices(&room);
        if (window == NULL) {
            refused = 1;
            break;
        }
        xenolith_done_with_vertices(room);
    }

    if (!refused) {
        fprintf(stderr, "the arena never filled, so nothing was refused\n");
        return 3;
    }

    /* And after a refusal, a new frame starts with the room again. */
    xenolith_begin_a_frame();
    uint32_t room = 0;
    if (xenolith_room_for_vertices(&room) == NULL || room == 0) {
        fprintf(stderr, "a new frame did not have the room again\n");
        return 4;
    }
    printf("refused\n");
    return 0;
}
"#;
    if let Err(complaint) = runs_with_the_renderer("arena", program) {
        panic!("a full arena did not refuse:\n{complaint}");
    }
}

/// The renderer opens a device where the machine has one, and says which.
///
/// A renderer that cannot open a device is not a failure: a machine without a
/// graphics driver is the ordinary case. What would be wrong is opening one and
/// not saying so, or claiming to draw where nothing was opened, so the answer
/// and the line have to agree.
#[test]
fn the_renderer_opens_a_device_where_there_is_one() {
    let program = r#"
/* Asked for before anything is included, because a strict C compiler hides the
 * declarations this needs otherwise. */
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    /* Asked twice, because opening a device is something a run does once and
     * the second answer has to be the first one. */
    int first = xenolith_can_draw();
    int again = xenolith_can_draw();
    if (first != again) {
        fprintf(stderr, "asking twice gave %d and then %d\n", first, again);
        return 1;
    }
    printf("drawing %d\n", first);
    return 0;
}
"#;
    if let Err(complaint) = runs_with_the_renderer("renderer", program) {
        panic!("the renderer did not answer about a device:\n{complaint}");
    }
}

/// Builds and runs a program beside the renderer, with it turned on.
fn runs_with_the_renderer(name: &str, program: &str) -> Result<(), String> {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&directory).map_err(|error| format!("making a directory: {error}"))?;
    /* The runtime and the renderer both, because a program that asks the
     * renderer for room asks through the runtime's own interface. */
    for (file, text) in [
        ("xenolith.h", HEADER),
        ("xenolith.c", include_str!("../runtime/xenolith.c")),
        (
            "command_stream.c",
            include_str!("../runtime/command_stream.c"),
        ),
        ("drawing.c", include_str!("../runtime/drawing.c")),
        ("sound.c", include_str!("../runtime/sound.c")),
        ("vulkan.c", include_str!("../runtime/vulkan.c")),
        ("shaders.c", SHADER_TABLE),
        ("main.c", program),
    ] {
        std::fs::write(directory.join(file), text)
            .map_err(|error| format!("writing {file}: {error}"))?;
    }

    let program_path = directory.join("program");
    let output = Command::new(honest::host_compiler())
        .args(["-std=c17", "-Wall", "-Wextra", "-Werror", "-pedantic"])
        .arg("-DXENOLITH_WITH_VULKAN")
        .arg("-o")
        .arg(&program_path)
        .arg(directory.join("main.c"))
        .arg(directory.join("xenolith.c"))
        .arg(directory.join("vulkan.c"))
        .arg(directory.join("shaders.c"))
        .args(["-lvulkan", "-lm", "-lpthread"])
        .output()
        .map_err(|error| format!("running the compiler: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }

    let ran = Command::new(&program_path)
        .current_dir(&directory)
        .output()
        .map_err(|error| format!("running the check: {error}"))?;
    if ran.status.success() {
        Ok(())
    } else {
        Err(format!(
            "check {} failed:\n{}",
            ran.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&ran.stderr)
        ))
    }
}

/// A draw with a real pair of shaders puts something on the surface.
///
/// Everything before this proves a piece works on its own: the renderer opens a
/// device, the command stream reads a packet, the shader table finds a shader.
/// None of that says a draw happens. What this checks is the whole chain: a
/// vertex shader and a pixel shader that a validator accepts, handed over the
/// way a title hands them over, and a draw that reads the frame back.
/// Needs a device to draw on, and says so rather than passing where there is
/// none: a machine without a graphics driver prints "no device" and the test
/// returns without claiming a picture was drawn.
#[test]
#[allow(clippy::too_many_lines)]
fn a_draw_with_real_shaders_puts_something_on_the_surface() {
    /* A vertex shader a title draws a scene with, taken from the crate's own
     * validated fixtures, and a pixel shader that moves a constant into the
     * colour. The scene's own pixel shader would be the better test of the
     * translator, but it reads pictures, and a shader reading pictures nothing
     * has filled puts out black whatever else is right. What this is testing is
     * the draw, so the colour is one the shader can make on its own. */
    /* Two modules written for this test rather than taken from a title.
     *
     * A title's own shaders are the better test of the translator, but this is
     * a test of the draw: a scene's vertex shader works its position out from a
     * transform a title sets up, and a scene's pixel shader reads pictures, so
     * neither puts anything visible on the surface without reproducing a
     * title's own state. These two put a triangle where the corners say and
     * fill it with a colour, which is what says a draw reached the surface.
     *
     * The shader table below hands them over the way the lift hands over a
     * title's: the translated words, the streams each reads, and the pictures
     * it looks at. */
    #[rustfmt::skip]
    const VERTEX: &[u32] = &[
        0x0723_0203,
        0x0001_0000,
        0x0008_000b,
        0x0000_0028,
        0x0000_0000,
        0x0002_0011,
        0x0000_0001,
        0x0006_000b,
        0x0000_0001,
        0x4c53_4c47,
        0x6474_732e,
        0x3035_342e,
        0x0000_0000,
        0x0003_000e,
        0x0000_0000,
        0x0000_0001,
        0x0007_000f,
        0x0000_0000,
        0x0000_0004,
        0x6e69_616d,
        0x0000_0000,
        0x0000_0019,
        0x0000_001d,
        0x0003_0003,
        0x0000_0002,
        0x0000_01c2,
        0x0004_0005,
        0x0000_0004,
        0x6e69_616d,
        0x0000_0000,
        0x0004_0005,
        0x0000_000c,
        0x6e72_6f63,
        0x0073_7265,
        0x0006_0005,
        0x0000_0017,
        0x505f_6c67,
        0x6556_7265,
        0x7865_7472,
        0x0000_0000,
        0x0006_0006,
        0x0000_0017,
        0x0000_0000,
        0x505f_6c67,
        0x7469_736f,
        0x006e_6f69,
        0x0007_0006,
        0x0000_0017,
        0x0000_0001,
        0x505f_6c67,
        0x746e_696f,
        0x657a_6953,
        0x0000_0000,
        0x0007_0006,
        0x0000_0017,
        0x0000_0002,
        0x435f_6c67,
        0x4470_696c,
        0x6174_7369,
        0x0065_636e,
        0x0007_0006,
        0x0000_0017,
        0x0000_0003,
        0x435f_6c67,
        0x446c_6c75,
        0x6174_7369,
        0x0065_636e,
        0x0003_0005,
        0x0000_0019,
        0x0000_0000,
        0x0006_0005,
        0x0000_001d,
        0x565f_6c67,
        0x6574_7265,
        0x646e_4978,
        0x0000_7865,
        0x0003_0047,
        0x0000_0017,
        0x0000_0002,
        0x0005_0048,
        0x0000_0017,
        0x0000_0000,
        0x0000_000b,
        0x0000_0000,
        0x0005_0048,
        0x0000_0017,
        0x0000_0001,
        0x0000_000b,
        0x0000_0001,
        0x0005_0048,
        0x0000_0017,
        0x0000_0002,
        0x0000_000b,
        0x0000_0003,
        0x0005_0048,
        0x0000_0017,
        0x0000_0003,
        0x0000_000b,
        0x0000_0004,
        0x0004_0047,
        0x0000_001d,
        0x0000_000b,
        0x0000_002a,
        0x0002_0013,
        0x0000_0002,
        0x0003_0021,
        0x0000_0003,
        0x0000_0002,
        0x0003_0016,
        0x0000_0006,
        0x0000_0020,
        0x0004_0017,
        0x0000_0007,
        0x0000_0006,
        0x0000_0002,
        0x0004_0015,
        0x0000_0008,
        0x0000_0020,
        0x0000_0000,
        0x0004_002b,
        0x0000_0008,
        0x0000_0009,
        0x0000_0003,
        0x0004_001c,
        0x0000_000a,
        0x0000_0007,
        0x0000_0009,
        0x0004_0020,
        0x0000_000b,
        0x0000_0007,
        0x0000_000a,
        0x0004_002b,
        0x0000_0006,
        0x0000_000d,
        0x0000_0000,
        0x0004_002b,
        0x0000_0006,
        0x0000_000e,
        0x3f00_0000,
        0x0005_002c,
        0x0000_0007,
        0x0000_000f,
        0x0000_000d,
        0x0000_000e,
        0x0004_002b,
        0x0000_0006,
        0x0000_0010,
        0xbf00_0000,
        0x0005_002c,
        0x0000_0007,
        0x0000_0011,
        0x0000_0010,
        0x0000_0010,
        0x0005_002c,
        0x0000_0007,
        0x0000_0012,
        0x0000_000e,
        0x0000_0010,
        0x0006_002c,
        0x0000_000a,
        0x0000_0013,
        0x0000_000f,
        0x0000_0011,
        0x0000_0012,
        0x0004_0017,
        0x0000_0014,
        0x0000_0006,
        0x0000_0004,
        0x0004_002b,
        0x0000_0008,
        0x0000_0015,
        0x0000_0001,
        0x0004_001c,
        0x0000_0016,
        0x0000_0006,
        0x0000_0015,
        0x0006_001e,
        0x0000_0017,
        0x0000_0014,
        0x0000_0006,
        0x0000_0016,
        0x0000_0016,
        0x0004_0020,
        0x0000_0018,
        0x0000_0003,
        0x0000_0017,
        0x0004_003b,
        0x0000_0018,
        0x0000_0019,
        0x0000_0003,
        0x0004_0015,
        0x0000_001a,
        0x0000_0020,
        0x0000_0001,
        0x0004_002b,
        0x0000_001a,
        0x0000_001b,
        0x0000_0000,
        0x0004_0020,
        0x0000_001c,
        0x0000_0001,
        0x0000_001a,
        0x0004_003b,
        0x0000_001c,
        0x0000_001d,
        0x0000_0001,
        0x0004_0020,
        0x0000_001f,
        0x0000_0007,
        0x0000_0007,
        0x0004_002b,
        0x0000_0006,
        0x0000_0022,
        0x3f80_0000,
        0x0004_0020,
        0x0000_0026,
        0x0000_0003,
        0x0000_0014,
        0x0005_0036,
        0x0000_0002,
        0x0000_0004,
        0x0000_0000,
        0x0000_0003,
        0x0002_00f8,
        0x0000_0005,
        0x0004_003b,
        0x0000_000b,
        0x0000_000c,
        0x0000_0007,
        0x0003_003e,
        0x0000_000c,
        0x0000_0013,
        0x0004_003d,
        0x0000_001a,
        0x0000_001e,
        0x0000_001d,
        0x0005_0041,
        0x0000_001f,
        0x0000_0020,
        0x0000_000c,
        0x0000_001e,
        0x0004_003d,
        0x0000_0007,
        0x0000_0021,
        0x0000_0020,
        0x0005_0051,
        0x0000_0006,
        0x0000_0023,
        0x0000_0021,
        0x0000_0000,
        0x0005_0051,
        0x0000_0006,
        0x0000_0024,
        0x0000_0021,
        0x0000_0001,
        0x0007_0050,
        0x0000_0014,
        0x0000_0025,
        0x0000_0023,
        0x0000_0024,
        0x0000_000d,
        0x0000_0022,
        0x0005_0041,
        0x0000_0026,
        0x0000_0027,
        0x0000_0019,
        0x0000_001b,
        0x0003_003e,
        0x0000_0027,
        0x0000_0025,
        0x0001_00fd,
        0x0001_0038,
    ];
    #[rustfmt::skip]
    const PIXEL: &[u32] = &[
        0x0723_0203,
        0x0001_0000,
        0x0008_000b,
        0x0000_000d,
        0x0000_0000,
        0x0002_0011,
        0x0000_0001,
        0x0006_000b,
        0x0000_0001,
        0x4c53_4c47,
        0x6474_732e,
        0x3035_342e,
        0x0000_0000,
        0x0003_000e,
        0x0000_0000,
        0x0000_0001,
        0x0006_000f,
        0x0000_0004,
        0x0000_0004,
        0x6e69_616d,
        0x0000_0000,
        0x0000_0009,
        0x0003_0010,
        0x0000_0004,
        0x0000_0007,
        0x0003_0003,
        0x0000_0002,
        0x0000_01c2,
        0x0004_0005,
        0x0000_0004,
        0x6e69_616d,
        0x0000_0000,
        0x0004_0005,
        0x0000_0009,
        0x6f6c_6f63,
        0x0000_7275,
        0x0004_0047,
        0x0000_0009,
        0x0000_001e,
        0x0000_0000,
        0x0002_0013,
        0x0000_0002,
        0x0003_0021,
        0x0000_0003,
        0x0000_0002,
        0x0003_0016,
        0x0000_0006,
        0x0000_0020,
        0x0004_0017,
        0x0000_0007,
        0x0000_0006,
        0x0000_0004,
        0x0004_0020,
        0x0000_0008,
        0x0000_0003,
        0x0000_0007,
        0x0004_003b,
        0x0000_0008,
        0x0000_0009,
        0x0000_0003,
        0x0004_002b,
        0x0000_0006,
        0x0000_000a,
        0x3f80_0000,
        0x0004_002b,
        0x0000_0006,
        0x0000_000b,
        0x0000_0000,
        0x0007_002c,
        0x0000_0007,
        0x0000_000c,
        0x0000_000a,
        0x0000_000b,
        0x0000_000b,
        0x0000_000a,
        0x0005_0036,
        0x0000_0002,
        0x0000_0004,
        0x0000_0000,
        0x0000_0003,
        0x0002_00f8,
        0x0000_0005,
        0x0003_003e,
        0x0000_0009,
        0x0000_000c,
        0x0001_00fd,
        0x0001_0038,
    ];

    /* The table the runtime finds them by, built the way the lift builds it. */
    let mut table = String::from("#include \"xenolith.h\"\n\n");
    for (which, words) in [(0usize, VERTEX), (1usize, PIXEL)] {
        let _ = writeln!(table, "static const uint32_t said_{which}[] = {{");
        for word in words {
            let _ = writeln!(table, "    {word:#010x}u,");
        }
        table.push_str("};\n\n");
    }
    /* One stream for the vertex shader: slot 0, two words per vertex, read from
     * the front. The pixel shader reads none. */
    table.push_str("static const uint32_t reads_0[] = { 0u, 2u, 0u, 2u };\n\n");
    table.push_str(concat!(
        "static const size_t everything_there = 2;\n\n",
        "const uint32_t *xenolith_said(uint32_t kind, uint32_t sum, uint32_t words,\n",
        "                              uint32_t *length) {\n",
        "    (void)sum; (void)words;\n",
        "    if (length != 0) { *length = kind == 0 ? ",
    ));
    let _ = writeln!(table, "{}u : {}u; }}", VERTEX.len(), PIXEL.len());
    table.push_str(concat!(
        "    return kind == 0 ? said_0 : said_1;\n",
        "}\n\n",
        "const uint32_t *xenolith_reads(uint32_t kind, uint32_t sum, uint32_t words,\n",
        "                               uint32_t *streams) {\n",
        "    (void)sum; (void)words;\n",
        "    if (kind != 0) { if (streams != 0) { *streams = 0; } return 0; }\n",
        "    if (streams != 0) { *streams = 1; }\n",
        "    return reads_0;\n",
        "}\n\n",
        "const uint32_t *xenolith_looks_at(uint32_t kind, uint32_t sum, uint32_t words,\n",
        "                                  uint32_t *pictures) {\n",
        "    (void)kind; (void)sum; (void)words;\n",
        "    if (pictures != 0) { *pictures = 0; }\n",
        "    return 0;\n",
        "}\n",
    ));

    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("a_draw");
    std::fs::create_dir_all(&directory).expect("making a directory");
    for (name, text) in [
        ("xenolith.h", HEADER),
        ("xenolith.c", include_str!("../runtime/xenolith.c")),
        (
            "command_stream.c",
            include_str!("../runtime/command_stream.c"),
        ),
        ("sound.c", include_str!("../runtime/sound.c")),
        ("drawing.c", include_str!("../runtime/drawing.c")),
        ("vulkan.c", include_str!("../runtime/vulkan.c")),
        ("shaders.c", table.as_str()),
    ] {
        std::fs::write(directory.join(name), text).expect("writing a file");
    }

    let program = r#"
#define _POSIX_C_SOURCE 200809L

#include "xenolith.h"
#include <stdio.h>

xenolith_function xenolith_lookup(uint32_t address) {
    (void)address;
    return 0;
}

int main(void) {
    const char *path = "image.bin";
    FILE *made = fopen(path, "wb");
    if (made == NULL) { return 1; }
    fputc('x', made);
    fclose(made);
    uint8_t *base = xenolith_boot(path, 0);
    if (base == NULL) { return 2; }
    if (!xenolith_can_draw()) {
        printf("no device\n");
        return 0;
    }

    /* Three corners of a triangle in the title's own memory, each a position of
     * two floats. */
    uint32_t at = 0x01000000u;
    static const float corners[3][2] = {{0.0f, 0.5f}, {-0.5f, -0.5f}, {0.5f, -0.5f}};
    for (unsigned one = 0; one < 3; one++) {
        uint32_t across, down;
        __builtin_memcpy(&across, &corners[one][0], 4);
        __builtin_memcpy(&down, &corners[one][1], 4);
        xenolith_store32(base, at + one * 8u, across);
        xenolith_store32(base, at + one * 8u + 4u, down);
    }

    /* Fetch slot 0: vertices, at that address, that many words. */
    xenolith_store32(base, 0x7fc80000u + 0x4800u * 4u, at | 3u);
    xenolith_store32(base, 0x7fc80000u + 0x4801u * 4u, (3u * 2u) << 2);

    /* Not mixing, and every part of a colour written. A blend of nought is not
     * "leave it alone", it is a factor of zero on both sides, which multiplies
     * the colour away. */
    xenolith_store32(base, 0x7fc80000u + 0x2201u * 4u, 0x00010001u);
    xenolith_store32(base, 0x7fc80000u + 0x2104u * 4u, 0xfu);

    if (!xenolith_begin_a_frame()) { return 3; }
    xenolith_look_at(NULL, NULL);
    xenolith_clip_to(0u, 0u, 1279u, 719u);
    xenolith_hand_over_constants(base + 0x7fc80000u + 0x4000u * 4u,
                                 base + 0x7fc80000u + 0x4900u * 4u,
                                 base + 0x7fc80000u + 0x4908u * 4u);

    /* The words the table holds, found the way the runtime finds them. */
    uint32_t vertex_length = 0;
    uint32_t pixel_length = 0;
    const uint32_t *vertex = xenolith_said(0u, 0u, 0u, &vertex_length);
    const uint32_t *pixel = xenolith_said(1u, 0u, 0u, &pixel_length);
    if (vertex == NULL || pixel == NULL) {
        printf("the table holds no shader\n");
        return 0;
    }

    int drawn = xenolith_draw_now(0x00010001u, 0xfu, 0u, XENOLITH_DRAWN_HOWEVER_SOLID, 0u,
                                  0u, vertex, vertex_length, 0u, pixel, pixel_length, 3u,
                                  XENOLITH_SHAPE_TRIANGLES);
    if (!drawn) { return 4; }

    if (!xenolith_end_a_frame()) { return 5; }
    if (!xenolith_keep_a_frame("frame.ppm")) { return 6; }
    printf("drawn\n");
    return 0;
}
"#;
    std::fs::write(directory.join("main.c"), program).expect("writing the program");

    let program_path = directory.join("program");
    let built = Command::new(honest::host_compiler())
        .args(["-std=c17", "-O1", "-DXENOLITH_WITH_VULKAN"])
        .arg("-o")
        .arg(&program_path)
        .arg(directory.join("main.c"))
        .arg(directory.join("xenolith.c"))
        .arg(directory.join("vulkan.c"))
        .arg(directory.join("shaders.c"))
        .args(["-lvulkan", "-lm", "-lpthread"])
        .output()
        .expect("running the compiler");
    assert!(
        built.status.success(),
        "the draw test did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let ran = Command::new(&program_path)
        .current_dir(&directory)
        .output()
        .expect("running the draw");
    assert!(
        ran.status.success(),
        "the draw went wrong, {}:\n{}",
        ran.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&ran.stderr)
    );

    let said = String::from_utf8_lossy(&ran.stdout);
    if said.contains("no device") {
        return;
    }
    let picture = std::fs::read(directory.join("frame.ppm")).expect("the frame was not written");
    assert!(picture.starts_with(b"P6\n"), "the frame is not a picture");
    /* Something other than black somewhere in it, which is what says a draw
     * reached the surface rather than an empty frame being written out. */
    let lit = picture
        .windows(3)
        .skip(picture.len() / 4)
        .any(|pixel| pixel != [0, 0, 0]);
    assert!(
        lit,
        "the frame is entirely black, so nothing was drawn into it"
    );
}

/// A translated vertex shader draws a triangle when run on its own.
///
/// This is the only check that separates the shader from the renderer. It builds
/// a small Vulkan program that hands a module one vertex buffer and one block of
/// constants, draws a triangle, and reads the pixels back. The same program with
/// a shader written by hand draws a red triangle, so a frame with nothing lit in
/// it says the module is at fault rather than the harness.
///
/// Ignored because it fails, and the failure is the finding: a vertex shader
/// AC6 hands over draws nothing, with the fetch the translator declares and the
/// constants the runtime supplies, while a hand-written shader draws. What it
/// needs is a vertex shader translated from a title's words that draws, which
/// means finding what the module does that the hand-written one does not.
#[test]
#[ignore = "a translated title shader draws nothing, which is the defect this is here to show"]
fn a_translated_vertex_shader_draws_a_triangle() {
    let harness = include_str!("shader_frame/render.c");

    /* A vertex shader a title hands over, from the crate's own validated
     * fixtures: the one a scene is drawn with. */
    let words: &[u32] = &[
        0x0000_0000,
        0x1001_c400,
        0x2200_0000,
        0xc80f_8000,
        0x0000_0000,
        0xc200_0000,
        0x0000_0000,
        0x0000_0000,
        0x0000_0000,
    ];
    let said = xenolith_shader::translate(xenolith_shader::Kind::Vertex, words)
        .expect("the shader translates");

    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("shader_frame");
    std::fs::create_dir_all(&directory).expect("making a directory");
    std::fs::write(directory.join("render.c"), harness).expect("writing the harness");

    /* The module, as the words a driver reads. */
    let mut module = Vec::new();
    for word in &said.words {
        module.extend_from_slice(&word.to_le_bytes());
    }
    std::fs::write(directory.join("vert.spv"), &module).expect("writing the module");

    /* A fragment shader that puts out a colour whatever it is handed, built
     * here rather than committed as a binary. */
    std::fs::write(
        directory.join("frag.frag"),
        "#version 450\nlayout(location = 0) out vec4 colour;\n\
         void main() { colour = vec4(1.0, 0.0, 0.0, 1.0); }\n",
    )
    .expect("writing the fragment shader");
    let fragment = Command::new("glslangValidator")
        .args(["-V", "-o"])
        .arg(directory.join("frag.spv"))
        .arg(directory.join("frag.frag"))
        .output();
    match fragment {
        Ok(done) if done.status.success() => {}
        _ => return,
    }

    let program = directory.join("render");
    let built = Command::new(honest::host_compiler())
        .args(["-std=c17", "-O1", "-o"])
        .arg(&program)
        .arg(directory.join("render.c"))
        .args(["-lvulkan", "-lm"])
        .output()
        .expect("running the compiler");
    assert!(
        built.status.success(),
        "the harness did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let ran = Command::new(&program)
        .current_dir(&directory)
        .output()
        .expect("running the harness");
    let said = String::from_utf8_lossy(&ran.stdout);
    let lit = said
        .lines()
        .find(|line| line.starts_with("lit pixels:"))
        .and_then(|line| line.split_whitespace().nth(2))
        .and_then(|number| number.parse::<u32>().ok())
        .unwrap_or(0);
    assert!(
        lit > 0,
        "the translated shader drew nothing, and a hand-written one draws:\n{said}"
    );
}
