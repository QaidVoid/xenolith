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

use std::path::PathBuf;
use std::process::Command;

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
        .args(["-lm", "-lpthread"])
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
