//! Running a real function out of a real title on both sides.
//!
//! Every other check here runs code this project wrote. The instruction
//! differential runs encodings chosen to be awkward, and the control flow
//! differential runs sequences assembled by hand. Both are answers to questions
//! someone thought to ask.
//!
//! This one runs whatever a compiler emitted years ago into a shipped game: the
//! register allocation it happened to pick, the blocks in the order it happened
//! to lay them out, the constants it happened to fold. Nothing here chose the
//! code, which is the point.
//!
//! No game data is committed. The title, the toolchain, and the emulator all
//! come from the environment and every test skips when they are absent.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use xenolith_lift::{Imports, RUNTIME_HEADER, RUNTIME_SOURCE, lift, name_of};
use xenolith_xex::{Container, Image, KeyMaterial};

/// How many functions to run.
///
/// Every one costs an emulator start and a host process, so this is a sample
/// rather than a sweep. What makes it worth having is that the sample is drawn
/// from code nobody here wrote.
const SAMPLE: usize = 2000;

/// The registers seeded before a call and read back after it.
///
/// These are the ones the calling convention lets a function use without saving,
/// so they are where a leaf function keeps its working state.
const WATCHED: [u32; 8] = [3, 4, 5, 6, 7, 8, 9, 10];

/// Where the scratch the seeded pointers aim at lives.
///
/// Inside the guest's address space and outside the image, so that a function
/// handed a pointer writes somewhere both sides have and neither has anything
/// else in.
///
/// Clear of where a big endian ppc64 program loads itself, which is the same
/// sixteen megabytes this first used: mapping over it replaced the running
/// program's own code and every run died before reaching the function.
const SCRATCH: u32 = 0x3000_0000;

/// How much of that is seeded and compared.
const SCRATCH_SIZE: usize = 256;

/// How far into the scratch the compared window sits.
///
/// In the middle rather than at the start, so that a function indexing
/// backwards off a pointer it was handed lands somewhere mapped.
const SCRATCH_SPAN: usize = 64 << 20;

/// Where the stack the called function is given starts.
///
/// A real function spills to its stack and reads back what it spilled, and one
/// given no stack at all reads whatever the two harnesses happened to leave
/// where it looked. That is not a disagreement about the instruction set: the
/// hardware side was reading its own real stack while the model read guest
/// address zero. Both are pointed at the same swept guest address instead.
///
/// High enough that a function indexing below it stays mapped, clear of the
/// scratch and of the image, and with its top half below the sign bit so that
/// the two instructions loading it need no more than that.
const STACK: u32 = 0x4000_0000;

/// The lowest guest address mapped on the hardware side.
///
/// Not zero, since the kernel refuses to map the first pages, and a null
/// dereference should fault on both sides rather than quietly read something.
const GUEST_LOW: u64 = 0x1_0000;

/// How much of the guest address space is mapped on the hardware side.
///
/// All of it. The model maps four gigabytes and reaches memory as a base plus a
/// truncated thirty two bit address, so anything the hardware side leaves
/// unmapped is a difference between the two harnesses rather than between the
/// two models of the instruction set.
const GUEST_SPAN: u64 = (1 << 32) - GUEST_LOW;

/// What one run left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    registers: [u64; WATCHED.len()],
    condition: u32,
    exception: u64,
    scratch: Vec<u8>,
}

/// How many shapes of starting state each function is tried under.
const SHAPES: usize = 4;

/// Returns the values a run starts its watched registers at, per shape.
///
/// One shape is not enough. A real function does not say which of its arguments
/// are pointers and which are counts, and the two want opposite seeds: give a
/// count a pointer and the function scales it out of the mapping, give a pointer
/// a count and it dereferences nothing. Each was tried alone first and each left
/// about five of every six functions dying before they had done anything.
///
/// So every function is tried under all of them, and every shape that finishes
/// on both sides is compared. The shapes run from all pointers to all small,
/// with the middle two splitting the registers between the two readings.
///
/// What discriminates here is not the seeds but the memory they lead to, which
/// holds a pattern, and the function's own control flow, which nobody here
/// wrote.
fn seeds(shape: usize) -> [u64; WATCHED.len()] {
    let middle = u64::from(SCRATCH) + (SCRATCH_SPAN as u64 / 2);
    let pointers = WATCHED.len() - shape * (WATCHED.len() / SHAPES);
    let mut values = [0u64; WATCHED.len()];
    for (at, slot) in values.iter_mut().enumerate() {
        *slot = if at < pointers {
            middle + (at as u64) * 64
        } else {
            (at as u64) % 5
        };
    }
    values
}

/// Returns the bytes the scratch starts holding.
fn scratch_seed() -> Vec<u8> {
    (0..SCRATCH_SIZE)
        .map(|at| u8::try_from((at * 7 + 3) & 0xff).unwrap_or(0))
        .collect()
}

/// Returns the title named by the environment, decoded.
fn title() -> Option<Image> {
    let path = std::env::var_os("XENOLITH_ANALYSIS_XEX")?;
    let bytes = std::fs::read(path).ok()?;
    let container = Container::parse(&bytes).ok()?;
    let key = std::env::var("XENOLITH_XEX_KEY")
        .ok()
        .and_then(|text| KeyMaterial::from_hex(&text).ok());
    container.load(key.as_ref()).ok()
}

/// Returns the cross toolchain prefix, if it works.
fn cross_prefix() -> Option<String> {
    let prefix = std::env::var("XENOLITH_PPC_TOOLCHAIN").ok()?;
    Command::new(format!("{prefix}gcc"))
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|_| prefix)
}

/// Returns the emulator, if one is installed.
fn emulator() -> Option<String> {
    let name = std::env::var("XENOLITH_PPC_EMULATOR").unwrap_or_else(|_| "qemu-ppc64".to_owned());
    Command::new(&name)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|_| name)
}

/// Returns a host C compiler, if one is installed.
fn host_compiler() -> Option<&'static str> {
    ["clang", "cc", "gcc"].into_iter().find(|candidate| {
        Command::new(candidate)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    })
}

/// Returns a directory to build in, made fresh.
fn workspace(name: &str) -> PathBuf {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&directory);
    let _ = std::fs::create_dir_all(&directory);
    directory
}

/// Returns functions safe to run on both sides, with the C emitted for each.
///
/// Safe means it returns without calling anything. A function that calls would
/// drag in whatever it calls, and one that branches through a register or
/// reaches an import would leave the part of the program this can account for.
/// The point is not to run the hardest functions but to run ones nobody here
/// wrote.
fn candidates(image: &Image, wanted: usize) -> Vec<(u32, String)> {
    let program = xenolith_analysis::analyze(image, &[]);
    let mut found = Vec::new();

    for function in program.functions() {
        // Small enough that a disagreement can be read, and large enough to do
        // something.
        let instructions: u32 = function
            .blocks
            .iter()
            .map(|block| (block.end - block.start) / 4)
            .sum();
        if !(4..=48).contains(&instructions) {
            continue;
        }
        let Ok(lifted) = lift(image, function, &Imports::new()) else {
            continue;
        };
        if !lifted.calls.is_empty() {
            continue;
        }
        if ["xenolith_dispatch", "xenolith_import", "xenolith_trap"]
            .iter()
            .any(|escape| lifted.code.contains(escape))
        {
            continue;
        }
        found.push((function.start, lifted.code));
    }

    // Spread over the whole code section rather than taken from the front of
    // it. Functions near each other were written together and do the same
    // kinds of thing, so the first hundred are a far narrower test than a
    // hundred drawn from end to end.
    if found.len() <= wanted {
        return found;
    }
    let stride = found.len() / wanted;
    found.into_iter().step_by(stride).take(wanted).collect()
}

/// The assembly that seeds the registers, calls a guest function, and stores
/// what it left.
///
/// The seeds are loaded from a table held in a callee saved register, because
/// the registers being seeded are the ones the loads would otherwise use.
fn guest_assembly() -> String {
    let mut out = String::from("\t.globl run\n\t.type run,@function\nrun:\n");
    // r3 the target, r4 the output, r5 the seeds.
    out.push_str("\tmflr 0\n\tstd 0, 16(1)\n\tstdu 1, -160(1)\n");
    out.push_str("\tstd 28, 96(1)\n\tstd 29, 104(1)\n\tstd 30, 112(1)\n\tstd 31, 120(1)\n");
    out.push_str("\tmr 31, 3\n\tmr 30, 4\n\tmr 29, 5\n");

    // Everything the calling convention lets a function use without saving is
    // cleared, not just the registers being watched. The model starts from a
    // context of zeroes, and any register left holding whatever this wrapper
    // happened to put there is a difference between the two harnesses. A stack
    // probe out of one title read the count register scratch and disagreed for
    // exactly that reason.
    // The second is this program's own table pointer, which the driver needs
    // back before it can so much as print, so it goes on the real stack until
    // the real stack pointer is back too.
    out.push_str("\tstd 2, 88(1)\n");
    for register in [0, 2, 11, 12] {
        let _ = writeln!(out, "\tli {register}, 0");
    }

    // The real stack pointer is kept aside and swapped for the guest one, so
    // that what the function spills lands somewhere the model has too. The
    // callee restores it, since every register from fourteen up is its to save.
    let _ = writeln!(out, "\tmr 28, 1");
    let _ = writeln!(out, "\tlis 1, {}", STACK >> 16);
    let _ = writeln!(out, "\tori 1, 1, {}", STACK & 0xffff);

    for (at, register) in WATCHED.iter().enumerate() {
        let _ = writeln!(out, "\tld {register}, {}(29)", at * 8);
    }
    out.push_str("\tli 0, 0\n\tmtxer 0\n\tmtcr 0\n");
    out.push_str("\tmtctr 31\n\tbctrl\n");
    out.push_str("\tmr 1, 28\n\tld 2, 88(1)\n");

    for (at, register) in WATCHED.iter().enumerate() {
        let _ = writeln!(out, "\tstd {register}, {}(30)", at * 8);
    }
    let condition = WATCHED.len() * 8;
    let _ = writeln!(out, "\tmfcr 0\n\tstw 0, {condition}(30)");
    let _ = writeln!(out, "\tmfxer 0\n\tstd 0, {}(30)", condition + 8);

    out.push_str("\tld 28, 96(1)\n\tld 29, 104(1)\n\tld 30, 112(1)\n\tld 31, 120(1)\n");
    out.push_str("\taddi 1, 1, 160\n\tld 0, 16(1)\n\tmtlr 0\n\tblr\n");
    out
}

/// The driver that maps the image where the guest expects it and runs one
/// function.
///
/// One function per process. A function that faults takes the process with it,
/// and running them one at a time means the rest are still comparable.
const GUEST_DRIVER: &str = r#"
#define _GNU_SOURCE
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

/* Replacing a mapping that is already there is how this went wrong once, so a
 * request that would has to fail rather than succeed quietly. */
#ifndef MAP_FIXED_NOREPLACE
#define MAP_FIXED_NOREPLACE MAP_FIXED
#endif

void run(uint64_t target, uint64_t *out, const uint64_t *seeds);

int main(int count, char **arguments) {
    if (count < 3) { return 1; }
    uint32_t address = (uint32_t)strtoul(arguments[1], 0, 0);

    FILE *file = fopen(arguments[2], "rb");
    if (!file) { return 1; }
    /* The whole console address space, the way the model has it. A function
     * out of a real title computes addresses from what it was handed, and one
     * given only the pages it was expected to touch dies on the first it was
     * not.
     *
     * Reserved rather than committed, so this costs nothing until it is
     * written to. */
    void *space = mmap((void *)(uintptr_t)GUEST_LOW, GUEST_SPAN,
                       PROT_READ | PROT_WRITE | PROT_EXEC,
                       MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE |
                           MAP_NORESERVE,
                       -1, 0);
    if (space != (void *)(uintptr_t)GUEST_LOW) { return 1; }

    void *image = (void *)(uintptr_t)LOAD_ADDRESS;
    if (fread(image, 1, IMAGE_SPAN, file) == 0) { return 1; }
    fclose(file);

    static const unsigned char pattern[SCRATCH_SIZE] = SCRATCH_PATTERN;
    memcpy((unsigned char *)(uintptr_t)SCRATCH_BASE + SCRATCH_SPAN / 2,
           pattern, SCRATCH_SIZE);

    static const uint64_t seeds[SHAPE_COUNT][WATCHED_COUNT] = SEED_VALUES;
    int shape = count > 3 ? atoi(arguments[3]) : 0;
    if (shape < 0 || shape >= SHAPE_COUNT) { return 1; }
    static uint64_t out[WATCHED_COUNT + 2];
    run((uint64_t)address, out, seeds[shape]);

    for (int i = 0; i < WATCHED_COUNT; i++) {
        printf("%016llx ", (unsigned long long)out[i]);
    }
    printf("%08llx %016llx ", (unsigned long long)(out[WATCHED_COUNT] >> 32),
           (unsigned long long)out[WATCHED_COUNT + 1]);
    const unsigned char *m = (const unsigned char *)(uintptr_t)(SCRATCH_BASE + SCRATCH_SPAN / 2);
    for (int b = 0; b < SCRATCH_SIZE; b++) { printf("%02x", m[b]); }
    printf("\n");
    return 0;
}
"#;

/// The driver that runs the lifted C from the same state.
const MODEL_DRIVER: &str = r#"
#include "xenolith.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

xenolith_function xenolith_lookup(uint32_t address) { (void)address; return 0; }
xenolith_function chosen(uint32_t address);

int main(int count, char **arguments) {
    if (count < 3) { return 1; }
    uint32_t address = (uint32_t)strtoul(arguments[1], 0, 0);
    uint8_t *base = xenolith_boot(arguments[2], LOAD_ADDRESS);
    if (!base) { return 1; }

    static const unsigned char pattern[SCRATCH_SIZE] = SCRATCH_PATTERN;
    memcpy(base + SCRATCH_BASE + SCRATCH_SPAN / 2, pattern, SCRATCH_SIZE);

    static const uint64_t seeds[SHAPE_COUNT][WATCHED_COUNT] = SEED_VALUES;
    int shape = count > 3 ? atoi(arguments[3]) : 0;
    if (shape < 0 || shape >= SHAPE_COUNT) { return 1; }
    static xenolith_context state;
    state.r[1] = STACK_TOP;
    for (int i = 0; i < WATCHED_COUNT; i++) {
        state.r[WATCHED_LIST[i]] = seeds[shape][i];
    }

    xenolith_function entered = chosen(address);
    if (!entered) { return 1; }
    entered(&state, base);

    for (int i = 0; i < WATCHED_COUNT; i++) {
        printf("%016llx ", (unsigned long long)state.r[WATCHED_LIST[i]]);
    }
    printf("%08llx %016llx ", (unsigned long long)xenolith_condition_pack(state.cr),
           (unsigned long long)state.xer);
    for (int b = 0; b < SCRATCH_SIZE; b++) {
        printf("%02x", base[SCRATCH_BASE + SCRATCH_SPAN / 2 + b]);
    }
    printf("\n");
    return 0;
}
"#;

/// Fills the placeholders both drivers share.
fn filled(template: &str, load: u32, span: usize) -> String {
    let pattern: Vec<String> = scratch_seed().iter().map(u8::to_string).collect();
    let sets: Vec<String> = (0..SHAPES)
        .map(|shape| {
            let row: Vec<String> = seeds(shape)
                .iter()
                .map(|value| format!("{value}ull"))
                .collect();
            format!("{{{}}}", row.join(", "))
        })
        .collect();
    let watched: Vec<String> = WATCHED.iter().map(u32::to_string).collect();

    template
        .replace("LOAD_ADDRESS", &format!("0x{load:x}"))
        .replace("IMAGE_SPAN", &span.to_string())
        .replace("SCRATCH_BASE", &format!("0x{SCRATCH:x}"))
        .replace("SCRATCH_SPAN", &SCRATCH_SPAN.to_string())
        .replace("GUEST_LOW", &format!("0x{GUEST_LOW:x}"))
        .replace("GUEST_SPAN", &format!("0x{GUEST_SPAN:x}"))
        .replace("SCRATCH_SIZE", &SCRATCH_SIZE.to_string())
        .replace("STACK_TOP", &format!("0x{STACK:x}"))
        .replace("SCRATCH_PATTERN", &format!("{{{}}}", pattern.join(", ")))
        .replace("SEED_VALUES", &format!("{{{}}}", sets.join(", ")))
        .replace("SHAPE_COUNT", &SHAPES.to_string())
        .replace(
            "WATCHED_LIST",
            &format!("(const int[]){{{}}}", watched.join(", ")),
        )
        .replace("WATCHED_COUNT", &WATCHED.len().to_string())
}

/// Reads a state one side printed.
fn parse(text: &str) -> Option<State> {
    let fields: Vec<&str> = text.split_whitespace().collect();
    if fields.len() != WATCHED.len() + 3 {
        return None;
    }
    let mut registers = [0u64; WATCHED.len()];
    for (at, slot) in registers.iter_mut().enumerate() {
        *slot = u64::from_str_radix(fields.get(at)?, 16).ok()?;
    }
    let condition = u32::from_str_radix(fields.get(WATCHED.len())?, 16).ok()?;
    let exception = u64::from_str_radix(fields.get(WATCHED.len() + 1)?, 16).ok()?;
    let hex = fields.get(WATCHED.len() + 2)?;
    let scratch = (0..SCRATCH_SIZE)
        .map(|at| u8::from_str_radix(hex.get(at * 2..at * 2 + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;

    Some(State {
        registers,
        condition,
        exception: exception & 0xe000_0000,
        scratch,
    })
}

/// Builds the guest side and returns the program that runs one function.
fn build_guest(directory: &Path, image: &Image) -> Option<PathBuf> {
    let prefix = cross_prefix()?;
    let _ = std::fs::write(directory.join("run.s"), guest_assembly());
    let _ = std::fs::write(
        directory.join("driver.c"),
        filled(GUEST_DRIVER, image.base_address(), image.size()),
    );

    let built = Command::new(format!("{prefix}gcc"))
        .args([
            "-static",
            "-no-pie",
            // Out of the low four gigabytes entirely, so that the whole of the
            // console's address space is free to be mapped the way the model
            // maps it. Linked where it lands by default, the program sits in
            // the middle of that space and every mapping has to route around
            // it.
            "-Wl,-Ttext-segment=0x400000000",
            "-O1",
            "-o",
        ])
        .arg(directory.join("guest"))
        .arg(directory.join("driver.c"))
        .arg(directory.join("run.s"))
        .output()
        .ok()?;
    assert!(
        built.status.success(),
        "the guest did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    Some(directory.join("guest"))
}

/// Builds the model side over the functions chosen.
fn build_model(directory: &Path, image: &Image, chosen: &[(u32, String)]) -> Option<PathBuf> {
    let compiler = host_compiler()?;
    let _ = std::fs::write(directory.join("xenolith.h"), RUNTIME_HEADER);
    let _ = std::fs::write(directory.join("xenolith.c"), RUNTIME_SOURCE);

    let mut lifted = String::from("#include \"xenolith.h\"\n\n");
    for (address, _) in chosen {
        let _ = writeln!(
            lifted,
            "void {}(xenolith_context *ctx, uint8_t *base);",
            name_of(*address)
        );
    }
    lifted.push('\n');
    for (_, code) in chosen {
        lifted.push_str(code);
        lifted.push('\n');
    }
    // The driver reaches a function by address, over the ones lifted here.
    lifted.push_str("xenolith_function chosen(uint32_t address) {\n    switch (address) {\n");
    for (address, _) in chosen {
        let _ = writeln!(
            lifted,
            "    case {address:#010x}u: return {};",
            name_of(*address)
        );
    }
    lifted.push_str("    default: return 0;\n    }\n}\n");
    let _ = std::fs::write(directory.join("lifted.c"), lifted);

    let driver = filled(MODEL_DRIVER, image.base_address(), image.size());
    let _ = std::fs::write(directory.join("driver.c"), driver);

    let built = Command::new(compiler)
        .args(["-std=c17", "-O1", "-Wno-infinite-recursion", "-o"])
        .arg(directory.join("model"))
        .arg(directory.join("driver.c"))
        .arg(directory.join("lifted.c"))
        .arg(directory.join("xenolith.c"))
        .arg("-lm")
        .output()
        .ok()?;
    assert!(
        built.status.success(),
        "the model did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    Some(directory.join("model"))
}

/// How long one run is given before it counts as not having finished.
///
/// A function handed a state it was never called with can loop forever waiting
/// on a count that never reaches its end, and one that does has to be given up
/// on rather than allowed to stop the whole comparison.
///
/// Generous, because a real countdown seeded with a pointer runs to hundreds of
/// millions of turns, and the compiled model is slower at that than the
/// emulator is. Cutting it short there would report a defect where there is
/// only a slow loop.
const PATIENCE: &str = "25";

/// Runs one address on one side, or nothing when that side could not.
fn once(
    program: &Path,
    address: u32,
    shape: usize,
    image: &Path,
    emulator: Option<&str>,
) -> Option<State> {
    let mut command = Command::new("timeout");
    command.arg(PATIENCE);
    if let Some(emulator) = emulator {
        command.arg(emulator);
    }
    command.arg(program);
    let ran = command
        .arg(format!("{address:#010x}"))
        .arg(image)
        .arg(shape.to_string())
        .output()
        .ok()?;
    if !ran.status.success() {
        return None;
    }
    parse(&String::from_utf8_lossy(&ran.stdout))
}

/// Runs functions out of a shipped title on hardware and through the model.
///
/// A function that faults on the emulator is skipped rather than counted, since
/// the two sides cannot be compared when one of them did not finish. One that
/// finishes on hardware and not through the model is a disagreement, and is
/// reported as one.
#[test]
fn the_model_runs_what_the_title_holds() {
    let Some(image) = title() else {
        eprintln!("skipping: XENOLITH_ANALYSIS_XEX names no title this can decode");
        return;
    };
    if cross_prefix().is_none() || emulator().is_none() || host_compiler().is_none() {
        eprintln!("skipping: the toolchain, the emulator, or a host compiler is missing");
        return;
    }

    let chosen = candidates(&image, SAMPLE);
    assert!(
        chosen.len() >= SAMPLE / 2,
        "only {} functions were safe to run, which is too few to say anything",
        chosen.len()
    );

    // Two directories, because both sides write a driver and a build that
    // overwrote the other's would compile one of them twice.
    let shared = workspace("title-differential");
    let image_path = shared.join("image.bin");
    let _ = std::fs::write(&image_path, image.bytes());

    let Some(guest) = build_guest(&workspace("title-guest"), &image) else {
        return;
    };
    let Some(model) = build_model(&workspace("title-model"), &image, &chosen) else {
        return;
    };
    let emulator = emulator().expect("checked above");

    // One process per side per shape, which at this many functions is thousands
    // of them. Run across the machine rather than one at a time.
    let jobs: Vec<(u32, usize)> = chosen
        .iter()
        .flat_map(|(address, _)| (0..SHAPES).map(move |shape| (*address, shape)))
        .collect();
    let lanes = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let size = jobs.len().div_ceil(lanes);

    let outcomes: Vec<(u32, Option<String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .chunks(size)
            .map(|chunk| {
                let (guest, model, image_path, emulator) =
                    (&guest, &model, &image_path, &emulator);
                scope.spawn(move || {
                    let mut out = Vec::new();
                    for (address, shape) in chunk {
                        let Some(theirs) =
                            once(guest, *address, *shape, image_path, Some(emulator))
                        else {
                            continue;
                        };
                        let Some(ours) = once(model, *address, *shape, image_path, None) else {
                            out.push((*address, Some(format!(
                                "{address:#010x} shape {shape} finished on hardware and not through the model"
                            ))));
                            continue;
                        };
                        if theirs == ours {
                            out.push((*address, None));
                            continue;
                        }
                        out.push((*address, Some(describe(*address, *shape, &theirs, &ours))));
                    }
                    out
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok())
            .flatten()
            .collect()
    });

    let reached: std::collections::BTreeSet<u32> =
        outcomes.iter().map(|(address, _)| *address).collect();
    let agreed = outcomes.iter().filter(|(_, note)| note.is_none()).count();
    let skipped = jobs.len() - outcomes.len();
    let disagreements: Vec<String> = outcomes.into_iter().filter_map(|(_, note)| note).collect();
    let reached = reached.len();

    println!(
        "functions {reached} of {}, runs compared {agreed}, skipped {skipped}, disagreeing {}",
        chosen.len(),
        disagreements.len()
    );
    assert!(
        agreed > 0,
        "no function ran on both sides, so nothing was compared"
    );
    assert!(
        disagreements.is_empty(),
        "the model and the hardware disagree about {} functions out of {}\n{}",
        disagreements.len(),
        agreed as usize + disagreements.len(),
        disagreements.join("\n")
    );
}

/// Returns what differed, naming only the places that did.
fn describe(address: u32, shape: usize, theirs: &State, ours: &State) -> String {
    let mut out = format!("{address:#010x} shape {shape}");
    let both = theirs.registers.iter().zip(ours.registers.iter());
    for (register, (theirs, ours)) in WATCHED.iter().zip(both) {
        if theirs != ours {
            let _ = write!(
                out,
                "\n      r{register}  hardware {theirs:016x}  model {ours:016x}"
            );
        }
    }
    if theirs.condition != ours.condition {
        let _ = write!(
            out,
            "\n      cr   hardware {:08x}  model {:08x}",
            theirs.condition, ours.condition
        );
    }
    if theirs.exception != ours.exception {
        let _ = write!(
            out,
            "\n      xer  hardware {:016x}  model {:016x}",
            theirs.exception, ours.exception
        );
    }
    let differing: BTreeSet<usize> = (0..SCRATCH_SIZE)
        .filter(|at| theirs.scratch.get(*at) != ours.scratch.get(*at))
        .collect();
    if !differing.is_empty() {
        let _ = write!(out, "\n      memory differs at {} bytes", differing.len());
    }
    out
}
