//! Comparing what an instruction really does against what this project says.
//!
//! Every other check in this project compares against a description. The corpus
//! says which registers an instruction touches. A second disassembler says which
//! fields it has. Neither can say that subtracting reverses its operands or that
//! an arithmetic shift sets the carry when it drops a one bit, and those are the
//! mistakes that produce a recompiled game which starts, runs, and is wrong.
//!
//! This runs the instruction on emulated PowerPC hardware and runs the C this
//! project emits for it on the host, from the same starting state, and compares
//! what each left behind.
//!
//! Everything here skips when the tools are absent. It needs a big endian ppc64
//! assembler and linker, a user mode emulator, and a host C compiler.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

use xenolith_lift::RUNTIME_HEADER;
use xenolith_ppc::Instruction;

/// Registers the harness seeds and reads back.
///
/// Small and fixed, because one assembly template has to serve every
/// instruction and the operands are placed at chosen numbers to make that
/// possible.
/// How many registers are watched.
const WATCHED_COUNT: usize = 4;

const WATCHED: [u32; WATCHED_COUNT] = [3, 4, 5, 6];

/// The guest address the scratch memory lives at.
///
/// Fixed, and the same on both sides, because the model reaches memory through a
/// guest address added to a base. A buffer wherever the loader happened to put
/// it would leave the two sides addressing different things.
const MEMORY_BASE: u32 = 0x0001_0000;

/// How much of that memory is seeded and compared.
const MEMORY_SIZE: usize = 64;

/// Where the guest keeps the pointer to its output.
///
/// Callee saved and outside the watched set, so nothing under test can name it.
const OUTPUT_REGISTER: u32 = 31;

/// How many vector registers are watched.
const WATCHED_VECTOR_COUNT: usize = 3;

/// The vector registers under test, destination first.
const WATCHED_VECTORS: [u32; WATCHED_VECTOR_COUNT] = [1, 2, 3];

/// The three by name, so a subject reads as the operands it names.
const VECTOR_DESTINATION: u32 = 1;
const VECTOR_FIRST: u32 = 2;
const VECTOR_SECOND: u32 = 3;

/// Where the vector seeds sit in the scratch page.
///
/// Past the window that is compared, so seeding them cannot disturb what a
/// store is checked against, and sixteen byte aligned because the loads that
/// read them ignore the low bits of an address rather than faulting on them.
const VECTOR_SEED_OFFSET: u32 = 128;

/// How many floating point registers are watched.
///
/// More than the subjects strictly need. Each precision wants its own pair and
/// product, and each wants that product both ways round so that the adding and
/// the subtracting forms both cancel. The count also keeps the bytes each run
/// reports a multiple of sixteen, which the vector stores rely on.
const WATCHED_FLOAT_COUNT: usize = 10;

/// The floating point registers under test, destination first.
///
/// All of them are ones a called function is free to use without saving, so
/// the harness need not preserve them around the call.
const WATCHED_FLOATS: [u32; WATCHED_FLOAT_COUNT] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];

/// The destination every float subject writes.
const FLOAT_DESTINATION: u32 = 0;

/// A pair of doubles and the double their product rounds to.
///
/// Chosen so that a fused multiply and add of the three is exactly the error
/// the rounding threw away, which a multiply followed by an add computes as
/// zero. That makes the difference between one rounding and two the whole
/// result rather than a bit at the end of it.
const FLOAT_FIRST: u32 = 1;
const FLOAT_SECOND: u32 = 2;
const FLOAT_THIRD: u32 = 3;

/// The same three, held as values a single precision form can take exactly.
const SINGLE_FIRST: u32 = 4;
const SINGLE_SECOND: u32 = 5;
const SINGLE_THIRD: u32 = 6;

/// The two products again with their signs flipped, so that the forms which add
/// their third operand cancel the same way the ones that subtract it do.
const FLOAT_NEGATED: u32 = 7;
const SINGLE_NEGATED: u32 = 8;

/// Where the floating point seeds sit in the scratch page.
///
/// Past the vector seeds, which are themselves past the window that is
/// compared, so that seeding one disturbs neither the other nor what a store is
/// checked against.
const FLOAT_SEED_OFFSET: u32 = VECTOR_SEED_OFFSET + WATCHED_VECTOR_BYTES;

/// How many bytes the vector seeds take in the scratch page.
const WATCHED_VECTOR_BYTES: u32 = 48;

/// Bytes each run reports: the watched registers, the condition register, the
/// exception register, the watched vectors, the watched floats, and the scratch
/// memory.
///
/// A multiple of sixteen, so the vector slot of every run stays aligned for the
/// stores that write it.
const RESULT_BYTES: usize =
    (WATCHED_COUNT + 2) * 8 + WATCHED_VECTOR_COUNT * 16 + WATCHED_FLOAT_COUNT * 8 + MEMORY_SIZE;

/// Where the vectors sit within one run's report.
const VECTOR_RESULT_OFFSET: usize = (WATCHED_COUNT + 2) * 8;

/// Where the floats sit within one run's report.
const FLOAT_RESULT_OFFSET: usize = VECTOR_RESULT_OFFSET + WATCHED_VECTOR_COUNT * 16;

/// Where the scratch memory sits within one run's report.
const MEMORY_RESULT_OFFSET: usize = FLOAT_RESULT_OFFSET + WATCHED_FLOAT_COUNT * 8;

/// Bits of the exception register this project models.
///
/// Summary overflow, overflow, and carry. The rest holds state no lifted code
/// reads, and the emulator sets some of it for reasons of its own.
const EXCEPTION_MODELLED: u64 = 0xe000_0000;

/// The state an instruction started from or left behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State {
    /// The watched general purpose registers, in the order of `WATCHED`.
    pub registers: [u64; WATCHED_COUNT],
    /// The condition register, packed as the architecture packs it.
    pub condition: u32,
    /// The bits of the exception register this project models.
    pub exception: u64,
    /// The watched vector registers, in the order of `WATCHED_VECTORS`.
    pub vectors: [[u8; 16]; WATCHED_VECTOR_COUNT],
    /// The watched floating point registers, as the bits they hold.
    ///
    /// Compared as bits rather than as numbers, so that the two zeroes stay
    /// distinct and so that a result which is not a number has to match in the
    /// payload it carries and not merely in being one.
    pub floats: [u64; WATCHED_FLOAT_COUNT],
    /// The scratch memory afterwards, so a store can be seen.
    pub memory: [u8; MEMORY_SIZE],
}

/// Returns the tool prefix for the cross assembler and linker, if it works.
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
fn workspace(name: &str) -> Result<PathBuf, String> {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).map_err(|error| format!("making a directory: {error}"))?;
    Ok(directory)
}

/// Writes the assembly that executes each encoding on hardware.
///
/// Every register is named outright. Inline assembly with constraints does not
/// work here: the compiler allocates its own temporaries over the seeded
/// registers, and the first attempt at this reported that adding two numbers
/// returned one of them.
fn guest_assembly(words: &[u32], seeds: usize) -> String {
    let mut out = String::from("\t.globl run\n\t.type run,@function\nrun:\n");
    // The seed table, the output, and the scratch memory are kept in callee
    // saved registers no instruction under test can name.
    let _ = writeln!(out, "\tstd 29, -24(1)\n\tstd 30, -16(1)\n\tstd 31, -8(1)");
    let _ = writeln!(
        out,
        "\tmr 30, 3\n\tmr 31, 4\n\tlis 29, {}",
        MEMORY_BASE >> 16
    );

    for seed in 0..seeds {
        let base = seed * WATCHED.len() * 8;
        for word in words {
            for (at, register) in WATCHED.iter().enumerate() {
                let offset = base + at * 8;
                let _ = writeln!(out, "\tld {register}, {offset}(30)");
            }
            // The vector seeds live in the scratch page, so they are reached
            // the same way the scratch memory is.
            for (at, register) in WATCHED_VECTORS.iter().enumerate() {
                let offset = VECTOR_SEED_OFFSET as usize + at * 16;
                let _ = writeln!(out, "\tli 0, {offset}\n\tlvx {register}, 29, 0");
            }
            // The floating point seeds live in the scratch page too.
            for (at, register) in WATCHED_FLOATS.iter().enumerate() {
                let offset = FLOAT_SEED_OFFSET as usize + at * 8;
                let _ = writeln!(out, "\tlfd {register}, {offset}(29)");
            }
            // Start from a cleared condition and exception register, so that
            // what is read back afterwards was written by the instruction.
            let _ = writeln!(out, "\tli 0, 0\n\tmtxer 0\n\tmtcr 0");
            let _ = writeln!(out, "\t.long {word}");

            for (at, register) in WATCHED.iter().enumerate() {
                let offset = at * 8;
                let _ = writeln!(out, "\tstd {register}, {offset}({OUTPUT_REGISTER})");
            }
            let cr_at = WATCHED.len() * 8;
            let xer_at = cr_at + 8;
            let _ = writeln!(out, "\tmfcr 0\n\tstw 0, {cr_at}({OUTPUT_REGISTER})");
            let _ = writeln!(out, "\tmfxer 0\n\tstd 0, {xer_at}({OUTPUT_REGISTER})");

            for (at, register) in WATCHED_VECTORS.iter().enumerate() {
                let offset = VECTOR_RESULT_OFFSET + at * 16;
                let _ = writeln!(
                    out,
                    "\tli 0, {offset}\n\tstvx {register}, {OUTPUT_REGISTER}, 0"
                );
            }

            for (at, register) in WATCHED_FLOATS.iter().enumerate() {
                let offset = FLOAT_RESULT_OFFSET + at * 8;
                let _ = writeln!(out, "\tstfd {register}, {offset}({OUTPUT_REGISTER})");
            }

            // The scratch memory afterwards, so that a store can be seen.
            for at in (0..MEMORY_SIZE).step_by(8) {
                let to = MEMORY_RESULT_OFFSET + at;
                let _ = writeln!(out, "\tld 0, {at}(29)\n\tstd 0, {to}({OUTPUT_REGISTER})");
            }
            let _ = writeln!(
                out,
                "\taddi {OUTPUT_REGISTER}, {OUTPUT_REGISTER}, {RESULT_BYTES}"
            );
        }
    }

    let _ = writeln!(
        out,
        "\tld 29, -24(1)\n\tld 30, -16(1)\n\tld 31, -8(1)\n\tblr"
    );
    out
}

/// The driver that maps the scratch memory, calls the generated assembly, and
/// prints what it found.
///
/// The memory is mapped where both sides agree it is rather than wherever the
/// loader would have put it, since the model reaches memory by adding a guest
/// address to a base.
const GUEST_DRIVER: &str = r#"
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <sys/mman.h>

void run(const uint64_t *seeds, uint64_t *out);

int main(void) {
    static const uint64_t seeds[SEED_WORDS] = SEED_VALUES;
    static const unsigned char pattern[MEMORY_SIZE] = MEMORY_PATTERN;
    static const unsigned char vectors[VECTOR_SEED_BYTES] = VECTOR_SEEDS;
    static const uint64_t floats[FLOAT_COUNT] = FLOAT_SEEDS;
    static uint64_t out[RESULT_WORDS] __attribute__((aligned(16)));

    void *scratch = mmap((void *)(uintptr_t)MEMORY_BASE, 4096,
                         PROT_READ | PROT_WRITE,
                         MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
    if (scratch != (void *)(uintptr_t)MEMORY_BASE) {
        fprintf(stderr, "could not map the scratch memory\n");
        return 1;
    }
    memcpy(scratch, pattern, MEMORY_SIZE);
    memcpy((unsigned char *)scratch + VECTOR_SEED_OFFSET, vectors, VECTOR_SEED_BYTES);
    memcpy((unsigned char *)scratch + FLOAT_SEED_OFFSET, floats, FLOAT_COUNT * 8);

    run(seeds, out);

    for (int i = 0; i < RESULT_COUNT; i++) {
        const uint64_t *r = out + i * RESULT_SLOTS;
        printf("%016llx", (unsigned long long)r[0]);
        for (int w = 1; w < WATCHED_COUNT; w++) {
            printf(" %016llx", (unsigned long long)r[w]);
        }
        printf(" %08llx %016llx ",
               (unsigned long long)(r[WATCHED_COUNT] >> 32),
               (unsigned long long)r[WATCHED_COUNT + 1]);
        const unsigned char *v = (const unsigned char *)(r + WATCHED_COUNT + 2);
        for (int b = 0; b < VECTOR_SEED_BYTES; b++) {
            printf("%02x", v[b]);
        }
        printf(" ");
        const uint64_t *f = (const uint64_t *)(v + VECTOR_SEED_BYTES);
        for (int n = 0; n < FLOAT_COUNT; n++) {
            printf("%016llx", (unsigned long long)f[n]);
        }
        printf(" ");
        const unsigned char *m = (const unsigned char *)(f + FLOAT_COUNT);
        for (int b = 0; b < MEMORY_SIZE; b++) {
            printf("%02x", m[b]);
        }
        printf("\n");
    }
    return 0;
}
"#;

/// Fills in the sizes and values a driver needs.
fn driver_for(seeds: &[[u64; WATCHED_COUNT]], count: usize) -> String {
    let flat: Vec<String> = seeds
        .iter()
        .flat_map(|seed| seed.iter().map(|value| format!("{value}ull")))
        .collect();
    let pattern: Vec<String> = memory_seed().iter().map(u8::to_string).collect();

    GUEST_DRIVER
        .replace("SEED_WORDS", &flat.len().to_string())
        .replace("SEED_VALUES", &format!("{{{}}}", flat.join(", ")))
        .replace("MEMORY_PATTERN", &format!("{{{}}}", pattern.join(", ")))
        .replace("MEMORY_BASE", &format!("0x{MEMORY_BASE:x}"))
        .replace("MEMORY_SIZE", &MEMORY_SIZE.to_string())
        .replace("RESULT_WORDS", &(count * RESULT_BYTES / 8).to_string())
        .replace("RESULT_SLOTS", &(RESULT_BYTES / 8).to_string())
        .replace("RESULT_COUNT", &count.to_string())
        .replace("WATCHED_COUNT", &WATCHED.len().to_string())
        .replace("VECTOR_SEED_OFFSET", &VECTOR_SEED_OFFSET.to_string())
        .replace(
            "VECTOR_SEED_BYTES",
            &(WATCHED_VECTOR_COUNT * 16).to_string(),
        )
        .replace("VECTOR_SEEDS", &format!("{{{}}}", vector_seed().join(", ")))
        .replace("FLOAT_SEED_OFFSET", &FLOAT_SEED_OFFSET.to_string())
        .replace("FLOAT_COUNT", &WATCHED_FLOAT_COUNT.to_string())
        .replace(
            "FLOAT_SEEDS",
            &format!(
                "{{{}}}",
                float_seed()
                    .iter()
                    .map(|bits| format!("0x{bits:016x}ull"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
}

/// Returns the bits the watched floating point registers start each run with.
///
/// The two triples are a pair and the value their product rounds to, one at
/// each precision. A fused multiply and add over a triple gives the error the
/// rounding discarded; the same written as a multiply and then an add gives
/// zero, because the product it rounds is the value being subtracted. Nothing
/// else here separates one rounding from two so plainly.
fn float_seed() -> [u64; WATCHED_FLOAT_COUNT] {
    [
        // The destination, which no subject here reads.
        0x4008_0000_0000_0000,
        // Two doubles using every bit of their significands, and their product.
        0x3ff6_a09e_667f_3bcd,
        0x3ffb_b67a_e858_4caa,
        0x4003_988e_1409_212f,
        // The same shape at single precision, so the single forms are seeded
        // with values they can hold exactly rather than ones they would round
        // on the way in.
        0x3ff1_9999_a000_0000,
        0x3ff4_cccc_c000_0000,
        0x3ff6_e147_a000_0000,
        // Both products with the sign bit set, for the forms that add.
        0xc003_988e_1409_212f,
        0xbff6_e147_a000_0000,
        // A spare ordinary value, negative and not a power of two.
        0xbfe5_5555_5555_5555,
    ]
}

/// Returns the bytes the watched vector registers start each run holding.
///
/// Every lane differs from every other, and the three registers differ from
/// each other, so an operation that takes the wrong lane or the wrong register
/// brings back something visibly wrong rather than another copy of the same
/// number. The floating point families read these bits as floats, so the
/// patterns are chosen to be ordinary finite values rather than shapes that
/// would land on an infinity or a value that is not a number.
fn vector_seed() -> Vec<String> {
    let mut bytes = Vec::new();
    for register in 0..WATCHED_VECTOR_COUNT {
        for lane in 0..4u32 {
            let exponent = 0x3e + u32::try_from(register).unwrap_or(0);
            let word = (exponent << 24) | (0x10 << 16) | (lane << 12) | (0x0123 + lane);
            for byte in word.to_be_bytes() {
                bytes.push(byte.to_string());
            }
        }
    }
    bytes
}

/// Runs each encoding on emulated hardware, once per seed.
fn on_hardware(words: &[u32], seeds: &[[u64; WATCHED_COUNT]]) -> Result<Vec<State>, String> {
    let Some(prefix) = cross_prefix() else {
        return Err("skip: XENOLITH_PPC_TOOLCHAIN names no working toolchain".to_owned());
    };
    let Some(emulator) = emulator() else {
        return Err("skip: no emulator is installed".to_owned());
    };

    let directory = workspace("execution-guest")?;
    std::fs::write(directory.join("run.s"), guest_assembly(words, seeds.len()))
        .map_err(|error| format!("writing the assembly: {error}"))?;
    std::fs::write(
        directory.join("driver.c"),
        driver_for(seeds, words.len() * seeds.len()),
    )
    .map_err(|error| format!("writing the driver: {error}"))?;

    let built = Command::new(format!("{prefix}gcc"))
        .args(["-static", "-O1", "-o"])
        .arg(directory.join("guest"))
        .arg(directory.join("driver.c"))
        .arg(directory.join("run.s"))
        .output()
        .map_err(|error| format!("running the cross compiler: {error}"))?;
    if !built.status.success() {
        return Err(format!(
            "the guest did not build:\n{}",
            String::from_utf8_lossy(&built.stderr)
        ));
    }

    let ran = Command::new(&emulator)
        .arg(directory.join("guest"))
        .output()
        .map_err(|error| format!("running the emulator: {error}"))?;
    if !ran.status.success() {
        return Err(format!(
            "the guest did not run:\n{}",
            String::from_utf8_lossy(&ran.stderr)
        ));
    }

    parse(&String::from_utf8_lossy(&ran.stdout))
}

/// Reads the state each side printed.
fn parse(text: &str) -> Result<Vec<State>, String> {
    let mut states = Vec::new();

    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // The registers, the condition register, the exception register, the
        // vectors, the floats, and the memory.
        if fields.len() != WATCHED.len() + 5 {
            continue;
        }
        let number = |at: usize| {
            fields
                .get(at)
                .and_then(|field| u64::from_str_radix(field, 16).ok())
        };

        let mut registers = [0u64; WATCHED_COUNT];
        for (at, slot) in registers.iter_mut().enumerate() {
            *slot = number(at).ok_or_else(|| format!("could not read {line:?}"))?;
        }
        let condition = number(WATCHED.len()).ok_or_else(|| format!("no condition in {line:?}"))?;
        let exception =
            number(WATCHED.len() + 1).ok_or_else(|| format!("no exception in {line:?}"))?;

        let hex = fields
            .get(WATCHED.len() + 2)
            .ok_or_else(|| format!("no vectors in {line:?}"))?;
        let mut vectors = [[0u8; 16]; WATCHED_VECTOR_COUNT];
        for (at, register) in vectors.iter_mut().enumerate() {
            for (byte, slot) in register.iter_mut().enumerate() {
                let start = (at * 16 + byte) * 2;
                let pair = hex
                    .get(start..start + 2)
                    .ok_or_else(|| format!("short vectors in {line:?}"))?;
                *slot = u8::from_str_radix(pair, 16)
                    .map_err(|error| format!("bad vectors in {line:?}: {error}"))?;
            }
        }

        let hex = fields
            .get(WATCHED.len() + 3)
            .ok_or_else(|| format!("no floats in {line:?}"))?;
        let mut floats = [0u64; WATCHED_FLOAT_COUNT];
        for (at, slot) in floats.iter_mut().enumerate() {
            let text = hex
                .get(at * 16..at * 16 + 16)
                .ok_or_else(|| format!("short floats in {line:?}"))?;
            *slot = u64::from_str_radix(text, 16)
                .map_err(|error| format!("bad floats in {line:?}: {error}"))?;
        }

        let hex = fields
            .get(WATCHED.len() + 4)
            .ok_or_else(|| format!("no memory in {line:?}"))?;
        let mut memory = [0u8; MEMORY_SIZE];
        for (at, slot) in memory.iter_mut().enumerate() {
            let pair = hex
                .get(at * 2..at * 2 + 2)
                .ok_or_else(|| format!("short memory in {line:?}"))?;
            *slot = u8::from_str_radix(pair, 16)
                .map_err(|error| format!("bad memory in {line:?}: {error}"))?;
        }

        states.push(State {
            registers,
            condition: u32::try_from(condition).unwrap_or(0),
            exception: exception & EXCEPTION_MODELLED,
            vectors,
            floats,
            memory,
        });
    }

    Ok(states)
}

/// The runtime entry points the model side has to provide before it will link.
///
/// Nothing implements the interface, so the harness implements the least that
/// lets the emitted code run: a reservation that always succeeds, since there is
/// one thread and nothing to lose it to, and a time base that stands still,
/// since nothing here reads one.
const RUNTIME_STUBS: &str = "\
    void xenolith_dispatch(xenolith_context *c, uint8_t *b, uint32_t a) { (void)c; (void)b; (void)a; }\n\
    void xenolith_trap(xenolith_context *c, uint8_t *b, uint32_t a) { (void)c; (void)b; (void)a; exit(9); }\n\
    void xenolith_import(xenolith_context *c, uint8_t *b, const char *l, uint32_t o) { (void)c; (void)b; (void)l; (void)o; }\n\
    uint32_t xenolith_reserve32(const uint8_t *b, uint32_t a) { return xenolith_load32(b, a); }\n\
    uint64_t xenolith_reserve64(const uint8_t *b, uint32_t a) { return xenolith_load64(b, a); }\n\
    uint8_t xenolith_conditional32(uint8_t *b, uint32_t a, uint32_t v) { xenolith_store32(b, a, v); return 1; }\n\
    uint8_t xenolith_conditional64(uint8_t *b, uint32_t a, uint64_t v) { xenolith_store64(b, a, v); return 1; }\n\
    uint64_t xenolith_timebase(void) { return 0; }\n\n";

/// Writes the C that puts one starting state into the context.
///
/// Shared by the two model programs, which seed the same registers from the
/// same tables and differ only in what they run afterwards.
fn seed_state(out: &mut String, seed: &[u64; WATCHED_COUNT]) {
    for (register, value) in WATCHED.iter().zip(seed) {
        let _ = writeln!(out, "        state.r[{register}] = {value}ull;");
    }
    for (at, register) in WATCHED_VECTORS.iter().enumerate() {
        let _ = writeln!(
            out,
            "        for (unsigned b = 0; b < 16; b++) {{ xenolith_vector_set_u8(&state.v[{register}], b, vectors[{at} * 16 + b]); }}"
        );
    }
    for (at, register) in WATCHED_FLOATS.iter().enumerate() {
        let _ = writeln!(out, "        state.f[{register}].u64 = floats[{at}];");
    }
}

/// Writes the C that prints one finished state, in the order the parser reads.
fn report_state(out: &mut String) {
    out.push_str("        printf(\"%016llx\", (unsigned long long)state.r[3]);\n");
    for register in &WATCHED[1..] {
        let _ = writeln!(
            out,
            "        printf(\" %016llx\", (unsigned long long)state.r[{register}]);"
        );
    }
    out.push_str(
        "        printf(\" %08llx %016llx \", (unsigned long long)packed(&state), (unsigned long long)state.xer);\n",
    );
    for register in &WATCHED_VECTORS {
        let _ = writeln!(
            out,
            "        for (unsigned b = 0; b < 16; b++) {{ printf(\"%02x\", xenolith_vector_u8(&state.v[{register}], b)); }}"
        );
    }
    out.push_str("        printf(\" \");\n");
    for register in &WATCHED_FLOATS {
        let _ = writeln!(
            out,
            "        printf(\"%016llx\", (unsigned long long)state.f[{register}].u64);"
        );
    }
    out.push_str("        printf(\" \");\n");
    let _ = writeln!(
        out,
        "        for (int b = 0; b < {MEMORY_SIZE}; b++) {{ printf(\"%02x\", memory[0x{MEMORY_BASE:x} + b]); }}"
    );
    out.push_str("        printf(\"\\n\");\n    }\n");
}

/// Writes the host C that runs each encoding through this project's model.
fn model_program(words: &[u32], seeds: &[[u64; WATCHED_COUNT]]) -> String {
    let pattern: Vec<String> = memory_seed().iter().map(u8::to_string).collect();
    let mut out = String::from(
        "#include \"xenolith.h\"\n#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n\n",
    );
    out.push_str(RUNTIME_STUBS);
    // The condition register is packed the way the architecture packs it, so the
    // two sides can be compared as one number.
    out.push_str(
        "static uint32_t packed(const xenolith_context *ctx) {\n\
         \x20   uint32_t out = 0;\n\
         \x20   for (int i = 0; i < 8; i++) {\n\
         \x20       uint32_t f = (uint32_t)(ctx->cr[i].lt << 3 | ctx->cr[i].gt << 2 |\n\
         \x20                               ctx->cr[i].eq << 1 | ctx->cr[i].so);\n\
         \x20       out |= f << (28 - i * 4);\n\
         \x20   }\n\
         \x20   return out;\n\
         }\n\n",
    );

    let _ = writeln!(
        out,
        "static uint8_t memory[{}];",
        MEMORY_BASE as usize + 4096
    );
    let _ = writeln!(
        out,
        "static const unsigned char pattern[{}] = {{{}}};",
        MEMORY_SIZE,
        pattern.join(", ")
    );
    let seeded = vector_seed();
    let _ = writeln!(
        out,
        "static const unsigned char vectors[{}] = {{{}}};",
        seeded.len(),
        seeded.join(", ")
    );
    let _ = writeln!(
        out,
        "static const uint64_t floats[{}] = {{{}}};",
        WATCHED_FLOAT_COUNT,
        float_seed()
            .iter()
            .map(|bits| format!("0x{bits:016x}ull"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    out.push_str("\nint main(void) {\n");
    let _ = writeln!(
        out,
        "    memcpy(memory + 0x{MEMORY_BASE:x}, pattern, {MEMORY_SIZE});"
    );

    for seed in seeds {
        for word in words {
            let instruction = Instruction::decode(*word);
            let Some(code) = xenolith_lift::code_for(instruction, 0) else {
                continue;
            };

            // The emitted code writes through a pointer named ctx, so the state
            // is declared beside it and pointed at rather than the code being
            // rewritten to suit the test.
            out.push_str("    {\n        xenolith_context state = {0};\n");
            out.push_str("        xenolith_context *ctx = &state;\n");
            out.push_str("        uint8_t *base = memory;\n        (void)base;\n");
            out.push_str("        uint32_t address; (void)address;\n");
            seed_state(&mut out, seed);
            out.push_str(&code);

            report_state(&mut out);
        }
    }
    out.push_str("    return 0;\n}\n");
    out
}

/// Runs each encoding through the model on the host.
fn on_the_model(words: &[u32], seeds: &[[u64; WATCHED_COUNT]]) -> Result<Vec<State>, String> {
    let Some(compiler) = host_compiler() else {
        return Err("skip: no host C compiler is installed".to_owned());
    };

    let directory = workspace("execution-model")?;
    std::fs::write(directory.join("xenolith.h"), RUNTIME_HEADER)
        .map_err(|error| format!("writing the header: {error}"))?;
    std::fs::write(directory.join("model.c"), model_program(words, seeds))
        .map_err(|error| format!("writing the model program: {error}"))?;

    let built = Command::new(compiler)
        .args(["-std=c17", "-O1", "-lm", "-o"])
        .arg(directory.join("model"))
        .arg(directory.join("model.c"))
        .output()
        .map_err(|error| format!("running the host compiler: {error}"))?;
    if !built.status.success() {
        return Err(format!(
            "the model program did not build:\n{}",
            String::from_utf8_lossy(&built.stderr)
        ));
    }

    let ran = Command::new(directory.join("model"))
        .output()
        .map_err(|error| format!("running the model program: {error}"))?;

    parse(&String::from_utf8_lossy(&ran.stdout))
}

/// Inputs each instruction is run on.
///
/// One pair agrees by accident too often. Zero against zero agrees for nearly
/// every operation, two small positive numbers hide every question about sign,
/// and nothing carries out unless something is near the top of its range.
/// The last register always holds the scratch memory, so a load or a store has
/// somewhere to reach that both sides agree on.
const SEEDS: [[u64; WATCHED_COUNT]; 5] = [
    // The first register is the one most subjects write. It held zero in every
    // row once, which meant an instruction that reads what it writes was being
    // checked against a destination that was always already empty. Preserving
    // the high half and clearing it look identical from there, and an insert
    // that wrongly cleared it went unseen until a real title exercised one.
    [
        0xaaaa_aaaa_bbbb_bbbb,
        0xdead_beef_cafe_babe,
        0x0123_4567_89ab_cdef,
        MEMORY_BASE as u64,
    ],
    [
        0x0000_0001_0000_0002,
        0x0000_0000_0000_0007,
        0x0000_0000_0000_0003,
        MEMORY_BASE as u64,
    ],
    [
        0xffff_ffff_0000_0000,
        0xffff_ffff_ffff_ffff,
        0x0000_0000_0000_0001,
        MEMORY_BASE as u64,
    ],
    [
        0x0f0f_0f0f_f0f0_f0f0,
        0x8000_0000_8000_0000,
        0xffff_ffff_ffff_fffe,
        MEMORY_BASE as u64,
    ],
    // Both sources zero. Subtracting zero from zero carries, and the result is
    // zero too, so every way of reading the carry off the result alone gets it
    // wrong here and nowhere else. No other row has a zero source.
    [0x0000_0000_ffff_ffff, 0, 0, MEMORY_BASE as u64],
];

/// What the scratch memory holds before each run.
///
/// A pattern rather than zeroes, so a load that reads the wrong width or the
/// wrong byte order brings back something visibly wrong rather than another
/// zero.
fn memory_seed() -> [u8; MEMORY_SIZE] {
    let mut bytes = [0u8; MEMORY_SIZE];
    for (at, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::try_from(at)
            .unwrap_or(0)
            .wrapping_mul(7)
            .wrapping_add(0x11);
    }
    bytes
}

/// Builds an extended form instruction with its operands at the watched
/// registers.
///
/// The destination goes where the operation names it: most write the field the
/// first watched register occupies, and the logical and shift operations write
/// the second.
const fn extended(target: u32, first: u32, second: u32, code: u32, record: u32) -> u32 {
    (31 << 26) | (target << 21) | (first << 16) | (second << 11) | (code << 1) | record
}

/// The instructions this harness runs, paired with what they are.
fn subjects() -> Vec<(u32, &'static str)> {
    let (a, b, c, _d) = (WATCHED[0], WATCHED[1], WATCHED[2], WATCHED[3]);

    vec![
        // Nothing at all, which is the smallest thing the two sides can agree
        // on and the first thing that has to pass.
        (0x6000_0000, "nop"),
        (extended(a, b, c, 266, 0), "add"),
        (extended(a, b, c, 40, 0), "subf"),
        (extended(a, b, c, 235, 0), "mullw"),
        (extended(a, b, c, 233, 0), "mulld"),
        (extended(a, b, c, 75, 0), "mulhw"),
        (extended(a, b, c, 11, 0), "mulhwu"),
        (extended(a, b, c, 491, 0), "divw"),
        (extended(a, b, c, 459, 0), "divwu"),
        (extended(a, b, c, 489, 0), "divd"),
        (extended(a, b, c, 457, 0), "divdu"),
        (extended(a, b, 0, 104, 0), "neg"),
        // A trap whose condition never holds, so nothing traps on either side.
        // The model used to call the trap whatever the condition said, which
        // the stub turns into an exit, so a run that should have finished does
        // not. Only the case that does not fire can be checked here: one that
        // does takes the process with it on both sides.
        ((3 << 26) | (2 << 21) | (b << 16), "twi never"),
        ((2 << 26) | (2 << 21) | (b << 16), "tdi never"),
        (extended(2, b, b, 4, 0), "tw never"),
        (extended(2, b, b, 68, 0), "td never"),
        (extended(a, b, c, 10, 0), "addc"),
        (extended(a, b, c, 8, 0), "subfc"),
        // Subtracting a value from itself carries, and the result is zero, so
        // reading the carry off the result having come out below an operand
        // reports the opposite. The rest of the family adds the carry as a
        // third term, where the same reasoning fails the same way.
        (extended(a, b, b, 8, 0), "subfc from itself"),
        (extended(a, b, c, 138, 0), "adde"),
        (extended(a, b, 0, 202, 0), "addze"),
        (extended(a, b, 0, 234, 0), "addme"),
        (extended(a, b, c, 136, 0), "subfe"),
        (extended(a, b, 0, 200, 0), "subfze"),
        (extended(a, b, 0, 232, 0), "subfme"),
        // The logical and shift operations name their destination in the field
        // the arithmetic uses for a source.
        (extended(b, a, c, 28, 0), "and"),
        (extended(b, a, c, 444, 0), "or"),
        (extended(b, a, c, 316, 0), "xor"),
        (extended(b, a, c, 476, 0), "nand"),
        (extended(b, a, c, 124, 0), "nor"),
        (extended(b, a, c, 60, 0), "andc"),
        (extended(b, a, c, 412, 0), "orc"),
        (extended(b, a, c, 284, 0), "eqv"),
        (extended(b, a, c, 24, 0), "slw"),
        (extended(b, a, c, 536, 0), "srw"),
        (extended(b, a, c, 792, 0), "sraw"),
        (extended(b, a, c, 27, 0), "sld"),
        (extended(b, a, c, 539, 0), "srd"),
        (extended(b, a, c, 794, 0), "srad"),
        (extended(b, a, 0, 954, 0), "extsb"),
        (extended(b, a, 0, 922, 0), "extsh"),
        (extended(b, a, 0, 986, 0), "extsw"),
        (extended(b, a, 0, 26, 0), "cntlzw"),
        (extended(b, a, 0, 58, 0), "cntlzd"),
        (extended(b, a, 4, 824, 0), "srawi"),
        // The record bit, which compares the result against zero.
        (extended(a, b, c, 266, 1), "add."),
        (extended(b, a, c, 28, 1), "and."),
        // Immediate forms.
        ((14 << 26) | (a << 21) | (b << 16) | 0x0064, "addi"),
        ((15 << 26) | (a << 21) | (b << 16) | 0x0064, "addis"),
        ((12 << 26) | (a << 21) | (b << 16) | 0x0064, "addic"),
        ((7 << 26) | (a << 21) | (b << 16) | 0x0064, "mulli"),
        ((8 << 26) | (a << 21) | (b << 16) | 0x0064, "subfic"),
        ((24 << 26) | (b << 21) | (a << 16) | 0x1234, "ori"),
        ((25 << 26) | (b << 21) | (a << 16) | 0x1234, "oris"),
        ((26 << 26) | (b << 21) | (a << 16) | 0x1234, "xori"),
        ((28 << 26) | (b << 21) | (a << 16) | 0x1234, "andi."),
        ((29 << 26) | (b << 21) | (a << 16) | 0x1234, "andis."),
        // Rotates, whose masks are where the mistakes hide.
        (
            (21 << 26) | (b << 21) | (a << 16) | (2 << 11) | (29 << 1),
            "rlwinm left",
        ),
        // Rotated by nothing, so only the mask does anything.
        (
            (21 << 26) | (b << 21) | (a << 16) | (24 << 6) | (31 << 1),
            "rlwinm mask",
        ),
        (
            (21 << 26) | (b << 21) | (a << 16) | (8 << 11) | (4 << 6) | (20 << 1),
            "rlwinm across",
        ),
        // A mask whose end comes before its beginning really does wrap, and
        // then it reaches into the high half of the register. The subject that
        // used to carry this name had a mask running four to twenty, which does
        // not wrap at all, so nothing here tested the case until a real title
        // did.
        (
            (21 << 26) | (b << 21) | (a << 16) | (27 << 6) | (18 << 1),
            "rlwinm wrap",
        ),
        (
            (20 << 26) | (b << 21) | (a << 16) | (4 << 11) | (8 << 6) | (24 << 1),
            "rlwimi",
        ),
        (
            (20 << 26) | (b << 21) | (a << 16) | (27 << 6) | (18 << 1),
            "rlwimi wrap",
        ),
        (
            (23 << 26) | (b << 21) | (a << 16) | (c << 11) | (29 << 1),
            "rlwnm",
        ),
        // Compares, which write a condition field.
        ((11 << 26) | (6 << 23) | (b << 16) | 0x0007, "cmpi"),
        ((10 << 26) | (6 << 23) | (b << 16) | 0x0007, "cmpli"),
        (extended(6 << 2, b, c, 0, 0), "cmp"),
        (extended(6 << 2, b, c, 32, 0), "cmpl"),
    ]
    .into_iter()
    .chain(memory_subjects())
    .chain(vector_subjects())
    .collect()
}

/// The instructions that reach memory, addressed through the register holding
/// the scratch buffer.
fn memory_subjects() -> Vec<(u32, &'static str)> {
    let (a, b, c, d) = (WATCHED[0], WATCHED[1], WATCHED[2], WATCHED[3]);

    vec![
        // Memory, addressed through the register holding the scratch buffer.
        // A load that reads the wrong width or the wrong byte order brings back
        // something visibly wrong, since the buffer holds a pattern.
        ((34 << 26) | (a << 21) | (d << 16) | 0x0008, "lbz"),
        ((40 << 26) | (a << 21) | (d << 16) | 0x0008, "lhz"),
        ((42 << 26) | (a << 21) | (d << 16) | 0x0008, "lha"),
        ((32 << 26) | (a << 21) | (d << 16) | 0x0008, "lwz"),
        ((58 << 26) | (a << 21) | (d << 16) | 0x0008, "ld"),
        ((38 << 26) | (a << 21) | (d << 16) | 0x0010, "stb"),
        ((44 << 26) | (a << 21) | (d << 16) | 0x0010, "sth"),
        ((36 << 26) | (a << 21) | (d << 16) | 0x0010, "stw"),
        ((62 << 26) | (a << 21) | (d << 16) | 0x0010, "std"),
        // The indexed forms, whose address is a sum of two registers.
        (extended(a, d, 0, 87, 0), "lbzx"),
        (extended(a, d, 0, 279, 0), "lhzx"),
        (extended(a, d, 0, 23, 0), "lwzx"),
        (extended(a, d, 0, 21, 0), "ldx"),
        (extended(a, d, 0, 215, 0), "stbx"),
        (extended(a, d, 0, 407, 0), "sthx"),
        (extended(a, d, 0, 151, 0), "stwx"),
        (extended(a, d, 0, 149, 0), "stdx"),
        // The updating forms, which write the address register back.
        ((33 << 26) | (a << 21) | (d << 16) | 0x0008, "lwzu"),
        ((37 << 26) | (a << 21) | (d << 16) | 0x0010, "stwu"),
        // The byte reversed forms, whose whole point is the order. Over a
        // patterned buffer a missed swap brings back a visibly wrong value.
        (extended(a, d, 0, 534, 0), "lwbrx"),
        (extended(a, d, 0, 790, 0), "lhbrx"),
        (extended(a, d, 0, 662, 0), "stwbrx"),
        (extended(a, d, 0, 918, 0), "sthbrx"),
        // The high half of a doubleword product, which the interface computes
        // itself for want of a wider type to hold it in.
        (extended(a, b, c, 73, 0), "mulhd"),
        (extended(a, b, c, 9, 0), "mulhdu"),
    ]
}

/// Builds a floating point instruction of the three source form.
///
/// The third source sits between the second and the extended opcode, which is
/// the field the two source forms leave empty.
const fn floating(first: u32, second: u32, third: u32, opcode: u32, code: u32) -> u32 {
    (opcode << 26)
        | (FLOAT_DESTINATION << 21)
        | (first << 16)
        | (second << 11)
        | (third << 6)
        | (code << 1)
}

/// Builds a floating point instruction that reads one source.
const fn floating_one(second: u32, code: u32) -> u32 {
    (63 << 26) | (FLOAT_DESTINATION << 21) | (second << 11) | (code << 1)
}

/// Instructions of the floating point families, over both precisions.
///
/// These had no execution oracle at all until now. The harness seeded general
/// registers, condition fields, the exception register, memory, and vectors,
/// and never a floating point register, so the whole family rested on the
/// emitted corpus and on reading. A fused multiply that rounded twice where the
/// architecture rounds once survived that until a real function tripped over
/// one.
fn float_subjects() -> Vec<(u32, &'static str)> {
    let (a, b, c) = (FLOAT_FIRST, FLOAT_SECOND, FLOAT_THIRD);
    let (sa, sb, sc) = (SINGLE_FIRST, SINGLE_SECOND, SINGLE_THIRD);

    vec![
        (floating(a, b, 0, 63, 21), "fadd"),
        (floating(a, b, 0, 63, 20), "fsub"),
        (floating(a, 0, b, 63, 25), "fmul"),
        (floating(a, b, 0, 63, 18), "fdiv"),
        (floating(sa, sb, 0, 59, 21), "fadds"),
        (floating(sa, sb, 0, 59, 20), "fsubs"),
        (floating(sa, 0, sb, 59, 25), "fmuls"),
        (floating(sa, sb, 0, 59, 18), "fdivs"),
        // The fused families, over a pair and the value their product rounds
        // to, so that rounding once and rounding twice give different answers
        // rather than answers that differ in a last bit.
        (floating(a, c, b, 63, 29), "fmadd"),
        (floating(a, c, b, 63, 28), "fmsub"),
        (floating(a, c, FLOAT_NEGATED, 63, 29), "fmadd cancelling"),
        (floating(a, c, FLOAT_NEGATED, 63, 31), "fnmadd cancelling"),
        (
            floating(sa, sc, SINGLE_NEGATED, 59, 29),
            "fmadds cancelling",
        ),
        (
            floating(sa, sc, SINGLE_NEGATED, 59, 31),
            "fnmadds cancelling",
        ),
        (floating(a, c, b, 63, 31), "fnmadd"),
        (floating(a, c, b, 63, 30), "fnmsub"),
        (floating(sa, sc, sb, 59, 29), "fmadds"),
        (floating(sa, sc, sb, 59, 28), "fmsubs"),
        (floating(sa, sc, sb, 59, 31), "fnmadds"),
        (floating(sa, sc, sb, 59, 30), "fnmsubs"),
        // Moving and changing a sign, which touch the bits and not the value.
        (floating_one(a, 72), "fmr"),
        (floating_one(a, 40), "fneg"),
        (floating_one(a, 264), "fabs"),
        (floating_one(a, 136), "fnabs"),
        // Narrowing to single, and the conversions to and from an integer.
        (floating_one(a, 12), "frsp"),
        (floating_one(a, 15), "fctiwz"),
        (floating_one(a, 815), "fctidz"),
        (floating_one(a, 846), "fcfid"),
        // Selecting on a sign, and the two compares, which write a field.
        (floating(a, b, c, 63, 23), "fsel"),
        ((63 << 26) | (6 << 23) | (a << 16) | (b << 11), "fcmpu"),
        (
            (63 << 26) | (6 << 23) | (a << 16) | (b << 11) | (32 << 1),
            "fcmpo",
        ),
    ]
}

/// Builds a vector instruction of the standard extension.
///
/// The console's own forms are absent on purpose. Neither assembler accepts one
/// and the emulator does not implement them, so running them here would test
/// nothing and reporting a figure that folded them in would imply a check that
/// did not happen.
const fn vector(a: u32, b: u32, code: u32) -> u32 {
    (4 << 26) | (VECTOR_DESTINATION << 21) | (a << 16) | (b << 11) | code
}

/// Returns an extended opcode with the bit that makes a comparison record into
/// a condition field.
const fn recording(code: u32) -> u32 {
    (1 << 10) | code
}

/// Returns the extended opcode of a four operand form, with the third source
/// folded into the bits it occupies.
const fn third(register: u32, code: u32) -> u32 {
    (register << 6) | code
}

/// Builds a vector instruction that carries an immediate where the first source
/// would sit.
const fn vector_immediate(immediate: u32, b: u32, code: u32) -> u32 {
    (4 << 26) | (VECTOR_DESTINATION << 21) | (immediate << 16) | (b << 11) | code
}

/// The vector instructions this harness runs.
fn vector_subjects() -> Vec<(u32, &'static str)> {
    vec![
        // Bit by bit, which is the same whatever the lanes are read as.
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1028), "vand"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1092), "vandc"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1156), "vor"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1284), "vnor"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1220), "vxor"),
        // Lane by lane, wrapping.
        (vector(VECTOR_FIRST, VECTOR_SECOND, 0), "vaddubm"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 64), "vadduhm"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 128), "vadduwm"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1024), "vsububm"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1088), "vsubuhm"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1152), "vsubuwm"),
        // Lane by lane, stopping at the end of the range.
        (vector(VECTOR_FIRST, VECTOR_SECOND, 512), "vaddubs"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 576), "vadduhs"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 768), "vaddsbs"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 832), "vaddshs"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 896), "vaddsws"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1536), "vsububs"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1600), "vsubuhs"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1792), "vsubsbs"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1856), "vsubshs"),
        // Shifts and rotates, whose count comes from the matching lane.
        (vector(VECTOR_FIRST, VECTOR_SECOND, 260), "vslh"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 388), "vsrh"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 836), "vsrah"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 68), "vrlh"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 132), "vrlw"),
        // Choosing between lanes.
        (vector(VECTOR_FIRST, VECTOR_SECOND, 66), "vmaxuh"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 322), "vmaxsh"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 578), "vminuh"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 834), "vminsh"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1090), "vavguh"),
        // Moving lanes about.
        (vector(VECTOR_FIRST, VECTOR_SECOND, 12), "vmrghb"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 76), "vmrghh"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 140), "vmrghw"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 268), "vmrglb"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 396), "vmrglw"),
        (vector_immediate(0x1f, VECTOR_SECOND, 780), "vspltisb"),
        (vector_immediate(0x1f, VECTOR_SECOND, 844), "vspltish"),
        (vector_immediate(0x1f, VECTOR_SECOND, 908), "vspltisw"),
        (vector_immediate(9, VECTOR_SECOND, 524), "vspltb"),
        (vector_immediate(2, VECTOR_SECOND, 652), "vspltw"),
        // Narrowing and widening.
        (vector(VECTOR_FIRST, VECTOR_SECOND, 14), "vpkuhum"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 78), "vpkuwum"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 142), "vpkuhus"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 398), "vpkshss"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 270), "vpkshus"),
        (vector_immediate(0, VECTOR_SECOND, 526), "vupkhsb"),
        (vector_immediate(0, VECTOR_SECOND, 590), "vupkhsh"),
        (vector_immediate(0, VECTOR_SECOND, 654), "vupklsb"),
        // Shifting the whole register by whole bytes. The forms that shift by
        // bits are absent: they require every byte of the second operand to
        // agree in its low three bits, which the shared seeds do not, and
        // hardware is free to return anything when they do not.
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1036), "vslo"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1100), "vsro"),
        // Sliding a window across the pair.
        (vector(VECTOR_FIRST, VECTOR_SECOND, third(5, 44)), "vsldoi"),
        // Single precision across four lanes.
        (vector(VECTOR_FIRST, VECTOR_SECOND, 10), "vaddfp"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 74), "vsubfp"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1034), "vmaxfp"),
        (vector(VECTOR_FIRST, VECTOR_SECOND, 1098), "vminfp"),
        (
            vector(VECTOR_FIRST, VECTOR_SECOND, third(VECTOR_SECOND, 46)),
            "vmaddfp",
        ),
        (
            vector(VECTOR_FIRST, VECTOR_SECOND, third(VECTOR_SECOND, 47)),
            "vnmsubfp",
        ),
        // Rounding, which the emulator and the host both do exactly.
        (vector_immediate(0, VECTOR_SECOND, 522), "vrfin"),
        (vector_immediate(0, VECTOR_SECOND, 586), "vrfiz"),
        (vector_immediate(0, VECTOR_SECOND, 650), "vrfip"),
        (vector_immediate(0, VECTOR_SECOND, 714), "vrfim"),
        // Between fixed point and single precision, with a scale.
        (vector_immediate(2, VECTOR_SECOND, 842), "vcfsx"),
        (vector_immediate(2, VECTOR_SECOND, 778), "vcfux"),
        (vector_immediate(2, VECTOR_SECOND, 970), "vctsxs"),
        (vector_immediate(2, VECTOR_SECOND, 906), "vctuxs"),
        // Comparisons, in the form that also writes a condition field.
        (
            vector(VECTOR_FIRST, VECTOR_SECOND, recording(198)),
            "vcmpeqfp.",
        ),
        (
            vector(VECTOR_FIRST, VECTOR_SECOND, recording(710)),
            "vcmpgtfp.",
        ),
        (
            vector(VECTOR_FIRST, VECTOR_SECOND, recording(134)),
            "vcmpequw.",
        ),
        (
            vector(VECTOR_FIRST, VECTOR_SECOND, recording(902)),
            "vcmpgtsw.",
        ),
        // Choosing between two under a mask, which is the one that reads three.
        (
            vector(VECTOR_FIRST, VECTOR_SECOND, third(VECTOR_SECOND, 42)),
            "vsel",
        ),
        // Permuting bytes by an index out of a third register.
        (
            vector(VECTOR_FIRST, VECTOR_SECOND, third(VECTOR_SECOND, 43)),
            "vperm",
        ),
    ]
}

/// Runs every instruction on hardware and through the model, and compares.
#[test]
fn the_model_computes_what_the_hardware_computes() {
    // An instruction the model cannot write out cannot be run through it. That
    // is reported rather than treated as agreement, since a check that did not
    // run says nothing.
    let mut all = subjects();
    all.extend(float_subjects());
    let (subjects, unreached): (Vec<_>, Vec<_>) = all
        .iter()
        .partition(|(word, _)| xenolith_lift::code_for(Instruction::decode(*word), 0).is_some());
    let words: Vec<u32> = subjects.iter().map(|(word, _)| *word).collect();

    let hardware = match on_hardware(&words, &SEEDS) {
        Ok(states) => states,
        Err(complaint) if complaint.starts_with("skip: ") => {
            eprintln!("{complaint}");
            return;
        }
        Err(complaint) => panic!("{complaint}"),
    };
    let model = match on_the_model(&words, &SEEDS) {
        Ok(states) => states,
        Err(complaint) if complaint.starts_with("skip: ") => {
            eprintln!("{complaint}");
            return;
        }
        Err(complaint) => panic!("{complaint}"),
    };

    assert_eq!(
        hardware.len(),
        words.len() * SEEDS.len(),
        "the hardware did not report a result for every run"
    );
    assert_eq!(
        model.len(),
        hardware.len(),
        "the two sides reported different numbers of results"
    );

    let mut agreed = 0u32;
    let mut disagreements = Vec::new();

    for (at, (theirs, ours)) in hardware.iter().zip(&model).enumerate() {
        let seed = at / words.len();
        let subject = at % words.len();
        let (word, name) = subjects[subject];

        if theirs == ours {
            agreed += 1;
            continue;
        }
        disagreements.push(format!(
            "{name} ({word:08x}) on r{}={:016x} r{}={:016x}\n      \
             hardware r{}={:016x} r{}={:016x} cr={:08x} xer={:016x}\n      \
             model    r{}={:016x} r{}={:016x} cr={:08x} xer={:016x}",
            WATCHED[1],
            SEEDS[seed][1],
            WATCHED[2],
            SEEDS[seed][2],
            WATCHED[0],
            theirs.registers[0],
            WATCHED[1],
            theirs.registers[1],
            theirs.condition,
            theirs.exception,
            WATCHED[0],
            ours.registers[0],
            WATCHED[1],
            ours.registers[1],
            ours.condition,
            ours.exception,
        ));
    }

    eprintln!("instructions executed {:>8}", words.len());
    eprintln!("not reached           {:>8}", unreached.len());
    for (word, name) in &unreached {
        eprintln!("  {name} ({word:08x}) has no code in the model");
    }
    eprintln!("runs compared         {:>8}", hardware.len());
    eprintln!("agreed                {agreed:>8}");
    eprintln!("disagreed             {:>8}", disagreements.len());
    for complaint in disagreements.iter().take(12) {
        eprintln!("\n  {complaint}");
    }

    assert!(
        disagreements.is_empty(),
        "the model computes something different from the hardware in {} runs",
        disagreements.len()
    );
}

/// Where a lifted sequence is placed.
const SEQUENCE_BASE: u32 = 0x8200_0000;

/// Writes the assembly that runs each sequence on hardware.
///
/// A sequence is a whole function ending in a return, so it is called rather
/// than run inline: its return would otherwise leave the harness. The link
/// register is saved first, since calling it is what overwrites it.
fn sequence_assembly(sequences: &[(&str, Vec<u32>)], seeds: &[[u64; WATCHED_COUNT]]) -> String {
    let mut out = String::new();

    for (at, (_, words)) in sequences.iter().enumerate() {
        let _ = writeln!(out, "\t.globl seq{at}\n\t.type seq{at},@function\nseq{at}:");
        for word in words {
            let _ = writeln!(out, "\t.long {word}");
        }
    }

    out.push_str("\t.globl run\n\t.type run,@function\nrun:\n");
    let _ = writeln!(out, "\tmflr 0\n\tstd 0, 16(1)\n\tstdu 1, -224(1)");
    let _ = writeln!(out, "\tstd 29, 160(1)\n\tstd 30, 168(1)\n\tstd 31, 176(1)");
    let _ = writeln!(
        out,
        "\tmr 30, 3\n\tmr 31, 4\n\tlis 29, {}",
        MEMORY_BASE >> 16
    );

    for (seed, _) in seeds.iter().enumerate() {
        let base = seed * WATCHED_COUNT * 8;
        for (at, _) in sequences.iter().enumerate() {
            for (slot, register) in WATCHED.iter().enumerate() {
                let offset = base + slot * 8;
                let _ = writeln!(out, "\tld {register}, {offset}(30)");
            }
            for (slot, register) in WATCHED_VECTORS.iter().enumerate() {
                let offset = VECTOR_SEED_OFFSET as usize + slot * 16;
                let _ = writeln!(out, "\tli 0, {offset}\n\tlvx {register}, 29, 0");
            }
            for (slot, register) in WATCHED_FLOATS.iter().enumerate() {
                let offset = FLOAT_SEED_OFFSET as usize + slot * 8;
                let _ = writeln!(out, "\tlfd {register}, {offset}(29)");
            }
            let _ = writeln!(out, "\tli 0, 0\n\tmtxer 0\n\tmtcr 0");
            let _ = writeln!(out, "\tbl seq{at}");

            for (slot, register) in WATCHED.iter().enumerate() {
                let offset = slot * 8;
                let _ = writeln!(out, "\tstd {register}, {offset}({OUTPUT_REGISTER})");
            }
            let cr_at = WATCHED_COUNT * 8;
            let xer_at = cr_at + 8;
            let _ = writeln!(out, "\tmfcr 0\n\tstw 0, {cr_at}({OUTPUT_REGISTER})");
            let _ = writeln!(out, "\tmfxer 0\n\tstd 0, {xer_at}({OUTPUT_REGISTER})");
            for (slot, register) in WATCHED_VECTORS.iter().enumerate() {
                let offset = VECTOR_RESULT_OFFSET + slot * 16;
                let _ = writeln!(
                    out,
                    "\tli 0, {offset}\n\tstvx {register}, {OUTPUT_REGISTER}, 0"
                );
            }
            for (slot, register) in WATCHED_FLOATS.iter().enumerate() {
                let offset = FLOAT_RESULT_OFFSET + slot * 8;
                let _ = writeln!(out, "\tstfd {register}, {offset}({OUTPUT_REGISTER})");
            }
            for byte in (0..MEMORY_SIZE).step_by(8) {
                let to = MEMORY_RESULT_OFFSET + byte;
                let _ = writeln!(out, "\tld 0, {byte}(29)\n\tstd 0, {to}({OUTPUT_REGISTER})");
            }
            let _ = writeln!(
                out,
                "\taddi {OUTPUT_REGISTER}, {OUTPUT_REGISTER}, {RESULT_BYTES}"
            );
        }
    }

    let _ = writeln!(out, "\tld 29, 160(1)\n\tld 30, 168(1)\n\tld 31, 176(1)");
    let _ = writeln!(out, "\taddi 1, 1, 224\n\tld 0, 16(1)\n\tmtlr 0\n\tblr");
    out
}

/// Runs each sequence on emulated hardware.
fn sequences_on_hardware(
    sequences: &[(&str, Vec<u32>)],
    seeds: &[[u64; WATCHED_COUNT]],
) -> Result<Vec<State>, String> {
    let Some(prefix) = cross_prefix() else {
        return Err("skip: XENOLITH_PPC_TOOLCHAIN names no working toolchain".to_owned());
    };
    let Some(emulator) = emulator() else {
        return Err("skip: no emulator is installed".to_owned());
    };

    let directory = workspace("sequence-guest")?;
    std::fs::write(directory.join("run.s"), sequence_assembly(sequences, seeds))
        .map_err(|error| format!("writing the assembly: {error}"))?;
    std::fs::write(
        directory.join("driver.c"),
        driver_for(seeds, sequences.len() * seeds.len()),
    )
    .map_err(|error| format!("writing the driver: {error}"))?;

    let built = Command::new(format!("{prefix}gcc"))
        .args(["-static", "-O1", "-o"])
        .arg(directory.join("guest"))
        .arg(directory.join("driver.c"))
        .arg(directory.join("run.s"))
        .output()
        .map_err(|error| format!("running the cross compiler: {error}"))?;
    if !built.status.success() {
        return Err(format!(
            "the guest did not build:\n{}",
            String::from_utf8_lossy(&built.stderr)
        ));
    }

    let ran = Command::new(&emulator)
        .arg(directory.join("guest"))
        .output()
        .map_err(|error| format!("running the emulator: {error}"))?;
    if !ran.status.success() {
        return Err(format!(
            "the guest did not run:\n{}",
            String::from_utf8_lossy(&ran.stderr)
        ));
    }

    parse(&String::from_utf8_lossy(&ran.stdout))
}

/// Lifts each sequence and runs what came out.
///
/// This is the only way the emitted control flow can be checked. A branch's
/// effect is on which instruction runs next, which no single instruction's
/// registers can show, and the code for one is written by the emitter from the
/// graph rather than by the model from the encoding.
/// Builds the host program that runs each lifted sequence.
fn sequence_program(
    sequences: &[(&str, Vec<u32>)],
    seeds: &[[u64; WATCHED_COUNT]],
) -> Result<String, String> {
    let pattern: Vec<String> = memory_seed().iter().map(u8::to_string).collect();
    let mut out = String::from(
        "#include \"xenolith.h\"\n#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n\n",
    );
    out.push_str(RUNTIME_STUBS);
    out.push_str(
        "static uint32_t packed(const xenolith_context *ctx) {\n\
         \x20   uint32_t out = 0;\n\
         \x20   for (int i = 0; i < 8; i++) {\n\
         \x20       uint32_t f = (uint32_t)(ctx->cr[i].lt << 3 | ctx->cr[i].gt << 2 |\n\
         \x20                               ctx->cr[i].eq << 1 | ctx->cr[i].so);\n\
         \x20       out |= f << (28 - i * 4);\n\
         \x20   }\n\
         \x20   return out;\n\
         }\n\n",
    );
    let _ = writeln!(
        out,
        "static uint8_t memory[{}];",
        MEMORY_BASE as usize + 4096
    );
    let _ = writeln!(
        out,
        "static const unsigned char pattern[{}] = {{{}}};",
        MEMORY_SIZE,
        pattern.join(", ")
    );
    let seeded = vector_seed();
    let _ = writeln!(
        out,
        "static const unsigned char vectors[{}] = {{{}}};",
        seeded.len(),
        seeded.join(", ")
    );
    let _ = writeln!(
        out,
        "static const uint64_t floats[{}] = {{{}}};",
        WATCHED_FLOAT_COUNT,
        float_seed()
            .iter()
            .map(|bits| format!("0x{bits:016x}ull"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    out.push('\n');

    // Each sequence is lifted into its own function, named for where it would
    // sit if it were a real one.
    let mut names = Vec::new();
    for (at, (name, words)) in sequences.iter().enumerate() {
        let base = sequence_base(at);
        let image = image_of(words, base);
        let program = xenolith_analysis::analyze(&image, &[]);
        let function = program
            .functions()
            .find(|function| function.start == base)
            .ok_or_else(|| format!("{name} was not discovered as a function"))?;
        let lifted = xenolith_lift::lift(&image, function, &xenolith_lift::Imports::new())
            .map_err(|refusal| format!("{name} did not lift: {}", refusal.mnemonic))?;

        out.push_str(&lifted.code);
        out.push('\n');
        names.push(xenolith_lift::name_of(function.start));
    }

    out.push_str("int main(void) {\n");
    let _ = writeln!(
        out,
        "    memcpy(memory + 0x{MEMORY_BASE:x}, pattern, {MEMORY_SIZE});"
    );
    for seed in seeds {
        for name in &names {
            out.push_str("    {\n        xenolith_context state = {0};\n");
            seed_state(&mut out, seed);
            let _ = writeln!(out, "        {name}(&state, memory);");
            report_state(&mut out);
        }
    }
    out.push_str("    return 0;\n}\n");

    Ok(out)
}

fn sequences_on_the_model(
    sequences: &[(&str, Vec<u32>)],
    seeds: &[[u64; WATCHED_COUNT]],
) -> Result<Vec<State>, String> {
    let Some(compiler) = host_compiler() else {
        return Err("skip: no host C compiler is installed".to_owned());
    };

    let out = sequence_program(sequences, seeds)?;

    let directory = workspace("sequence-model")?;
    std::fs::write(directory.join("xenolith.h"), RUNTIME_HEADER)
        .map_err(|error| format!("writing the header: {error}"))?;
    std::fs::write(directory.join("model.c"), out)
        .map_err(|error| format!("writing the model program: {error}"))?;

    let built = Command::new(compiler)
        .args(["-std=c17", "-O1", "-Wno-infinite-recursion", "-lm", "-o"])
        .arg(directory.join("model"))
        .arg(directory.join("model.c"))
        .output()
        .map_err(|error| format!("running the host compiler: {error}"))?;
    if !built.status.success() {
        return Err(format!(
            "the model program did not build:\n{}",
            String::from_utf8_lossy(&built.stderr)
        ));
    }

    let ran = Command::new(directory.join("model"))
        .output()
        .map_err(|error| format!("running the model program: {error}"))?;

    parse(&String::from_utf8_lossy(&ran.stdout))
}

/// Builds an image holding one sequence, so it can be analyzed and lifted.
///
/// Each sequence sits at its own address, since a lifted function is named for
/// where it begins and two at the same place would be one name twice.
fn image_of(words: &[u32], base: u32) -> xenolith_xex::Image {
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
    let size = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    let sections = vec![xenolith_xex::Section {
        start: base,
        size,
        kind: xenolith_xex::PageKind::Code,
    }];
    xenolith_xex::Image::new(base, bytes, sections).with_entry_point(Some(base))
}

/// Returns where the sequence at `at` is placed.
fn sequence_base(at: usize) -> u32 {
    SEQUENCE_BASE + u32::try_from(at).unwrap_or(0) * 0x1000
}

/// Short functions whose control flow is the thing being checked.
///
/// Each ends in a return, so it can be called on hardware and lifted into a
/// function on the host. What matters is which instructions ran, which shows up
/// in the registers they left behind.
fn sequences() -> Vec<(&'static str, Vec<u32>)> {
    // Branch if greater, over one instruction.
    let compare = |field: u32, against: u32| (10 << 26) | (field << 23) | (4 << 16) | against;
    let branch = |taken: u32, bit: u32, forward: u32| {
        (16 << 26) | (taken << 21) | (bit << 16) | (forward & 0xfffc)
    };
    let load = |register: u32, value: u32| (14 << 26) | (register << 21) | (value & 0xffff);
    let ret = 0x4e80_0020;

    // A carry produced by one instruction and consumed by the next. Run alone,
    // every one of these starts with the carry clear, so the term that adds it
    // is always adding nothing and the case where the pair carries first can
    // never come up.
    let extend = |target: u32, first: u32, second: u32, code: u32| {
        (31 << 26) | (target << 21) | (first << 16) | (second << 11) | (code << 1)
    };
    let carry_out = extend(5, 4, 4, 8);

    vec![
        (
            "carry into an add",
            vec![carry_out, extend(3, 4, 6, 138), ret],
        ),
        (
            "carry into a subtract",
            vec![carry_out, extend(3, 4, 6, 136), ret],
        ),
        (
            "carry into an extend by zero",
            vec![carry_out, extend(3, 4, 0, 202), ret],
        ),
        (
            "carry into an extend by minus one",
            vec![carry_out, extend(3, 4, 0, 234), ret],
        ),
        (
            "branch when greater",
            vec![compare(7, 7), branch(12, 29, 8), load(3, 99), ret],
        ),
        (
            "branch when not greater",
            vec![compare(7, 7), branch(4, 29, 8), load(3, 99), ret],
        ),
        (
            "branch when equal",
            vec![compare(0, 3), branch(12, 2, 8), load(3, 77), ret],
        ),
        (
            "both paths write",
            vec![
                compare(6, 10),
                branch(12, 25, 12),
                load(3, 11),
                (18 << 26) | 8,
                load(3, 22),
                ret,
            ],
        ),
        (
            "counting down",
            vec![
                load(3, 0),
                // add one to the total and take one off the counter
                (14 << 26) | (3 << 21) | (3 << 16) | 1,
                (14 << 26) | (4 << 21) | (4 << 16) | 0xffff,
                (10 << 26) | (4 << 16),
                // branch back while the counter is not zero
                (16 << 26) | (4 << 21) | (2 << 16) | (0xfff4 & 0xfffc),
                ret,
            ],
        ),
        (
            "a store and a load either side of a branch",
            vec![
                (36 << 26) | (5 << 21) | (6 << 16),
                compare(7, 7),
                branch(12, 29, 8),
                (32 << 26) | (3 << 21) | (6 << 16),
                ret,
            ],
        ),
        // A reservation says nothing on its own: a conditional store with no
        // reserved load before it fails on hardware and would report the model
        // wrong for modelling a reservation it was never given.
        (
            "a count incremented under a reservation",
            vec![
                // lwarx r3, 0, r6; addi r3, r3, 1; stwcx. r3, 0, r6
                (31 << 26) | (3 << 21) | (6 << 11) | (20 << 1),
                (14 << 26) | (3 << 21) | (3 << 16) | 1,
                (31 << 26) | (3 << 21) | (6 << 11) | (150 << 1) | 1,
                // branch back while the store did not happen
                (16 << 26) | (4 << 21) | (2 << 16) | (0xfff4 & 0xfffc),
                // lwz r5, 0(r6), so the stored value is read back
                (32 << 26) | (5 << 21) | (6 << 16),
                ret,
            ],
        ),
        // The condition register is cleared before every run, so a logical
        // between its bits only says something once a compare has filled it.
        (
            "condition bits combined and read back",
            vec![
                compare(0, 3),
                compare(1, 7),
                // cror 8, 2, 6 then crand 9, 2, 6
                (19 << 26) | (8 << 21) | (2 << 16) | (6 << 11) | (449 << 1),
                (19 << 26) | (9 << 21) | (2 << 16) | (6 << 11) | (257 << 1),
                // mfcr r3, so the whole register lands in a watched place
                (31 << 26) | (3 << 21) | (19 << 1),
                ret,
            ],
        ),
    ]
}

/// Runs whole functions on hardware and through the lifter, and compares.
#[test]
fn lifted_control_flow_goes_where_the_hardware_goes() {
    let sequences = sequences();

    let hardware = match sequences_on_hardware(&sequences, &SEEDS) {
        Ok(states) => states,
        Err(complaint) if complaint.starts_with("skip: ") => {
            eprintln!("{complaint}");
            return;
        }
        Err(complaint) => panic!("{complaint}"),
    };
    let model = match sequences_on_the_model(&sequences, &SEEDS) {
        Ok(states) => states,
        Err(complaint) if complaint.starts_with("skip: ") => {
            eprintln!("{complaint}");
            return;
        }
        Err(complaint) => panic!("{complaint}"),
    };

    assert_eq!(
        hardware.len(),
        sequences.len() * SEEDS.len(),
        "the hardware did not report a result for every run"
    );
    assert_eq!(
        model.len(),
        hardware.len(),
        "the two sides disagree in size"
    );

    let mut agreed = 0u32;
    let mut disagreements = Vec::new();

    for (at, (theirs, ours)) in hardware.iter().zip(&model).enumerate() {
        let seed = at / sequences.len();
        let (name, _) = &sequences[at % sequences.len()];

        if theirs == ours {
            agreed += 1;
            continue;
        }
        disagreements.push(format!(
            "{name} on r4={:016x} r5={:016x}\n      hardware {:016x?} cr={:08x}\n      model    {:016x?} cr={:08x}",
            SEEDS[seed][1],
            SEEDS[seed][2],
            theirs.registers,
            theirs.condition,
            ours.registers,
            ours.condition,
        ));
    }

    // A branch that never branches would let both sides agree while neither
    // did anything, so the harness checks that the paths really diverge.
    let mut diverged = 0;
    for at in 0..sequences.len() {
        let outcomes: std::collections::BTreeSet<[u64; WATCHED_COUNT]> = (0..SEEDS.len())
            .filter_map(|seed| hardware.get(seed * sequences.len() + at))
            .map(|state| state.registers)
            .collect();
        if outcomes.len() > 1 {
            diverged += 1;
        }
    }
    assert!(
        diverged >= sequences.len() - 1,
        "only {diverged} of {} sequences took more than one path, so the branches \
         are not being exercised",
        sequences.len()
    );

    eprintln!("sequences lifted      {:>8}", sequences.len());
    eprintln!("taking more than one path {diverged:>4}");
    eprintln!("runs compared         {:>8}", hardware.len());
    eprintln!("agreed                {agreed:>8}");
    eprintln!("disagreed             {:>8}", disagreements.len());
    for complaint in disagreements.iter().take(10) {
        eprintln!("\n  {complaint}");
    }

    assert!(
        disagreements.is_empty(),
        "lifted control flow goes somewhere else in {} runs",
        disagreements.len()
    );
}
