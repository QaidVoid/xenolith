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

/// Bytes each run reports: the watched registers, the condition register, and
/// the exception register.
const RESULT_BYTES: usize = (WATCHED_COUNT + 2) * 8 + MEMORY_SIZE;

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

            // The scratch memory afterwards, so that a store can be seen.
            for at in (0..MEMORY_SIZE).step_by(8) {
                let to = xer_at + 8 + at;
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
    static uint64_t out[RESULT_WORDS];

    void *scratch = mmap((void *)(uintptr_t)MEMORY_BASE, 4096,
                         PROT_READ | PROT_WRITE,
                         MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
    if (scratch != (void *)(uintptr_t)MEMORY_BASE) {
        fprintf(stderr, "could not map the scratch memory\n");
        return 1;
    }
    memcpy(scratch, pattern, MEMORY_SIZE);

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
        const unsigned char *m = (const unsigned char *)(r + WATCHED_COUNT + 2);
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
        if fields.len() != WATCHED.len() + 3 {
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
            memory,
        });
    }

    Ok(states)
}

/// Writes the host C that runs each encoding through this project's model.
fn model_program(words: &[u32], seeds: &[[u64; WATCHED_COUNT]]) -> String {
    let pattern: Vec<String> = memory_seed().iter().map(u8::to_string).collect();
    let mut out =
        String::from("#include \"xenolith.h\"\n#include <stdio.h>\n#include <string.h>\n\n");
    out.push_str(
        "void xenolith_dispatch(xenolith_context *c, uint8_t *b, uint32_t a) { (void)c; (void)b; (void)a; }\n\
         void xenolith_trap(xenolith_context *c, uint8_t *b, uint32_t a) { (void)c; (void)b; (void)a; }\n\n",
    );
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
            for (register, value) in WATCHED.iter().zip(seed) {
                let _ = writeln!(out, "        ctx->r[{register}] = {value}ull;");
            }
            out.push_str(&code);

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
            let _ = writeln!(
                out,
                "        for (int b = 0; b < {MEMORY_SIZE}; b++) {{ printf(\"%02x\", memory[0x{MEMORY_BASE:x} + b]); }}"
            );
            out.push_str("        printf(\"\\n\");\n    }\n");
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
        .args(["-std=c17", "-O1", "-o"])
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
const SEEDS: [[u64; WATCHED_COUNT]; 4] = [
    [
        0,
        0xdead_beef_cafe_babe,
        0x0123_4567_89ab_cdef,
        MEMORY_BASE as u64,
    ],
    [
        0,
        0x0000_0000_0000_0007,
        0x0000_0000_0000_0003,
        MEMORY_BASE as u64,
    ],
    [
        0,
        0xffff_ffff_ffff_ffff,
        0x0000_0000_0000_0001,
        MEMORY_BASE as u64,
    ],
    [
        0,
        0x8000_0000_8000_0000,
        0xffff_ffff_ffff_fffe,
        MEMORY_BASE as u64,
    ],
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
    let (a, b, c, d) = (WATCHED[0], WATCHED[1], WATCHED[2], WATCHED[3]);

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
        (extended(a, b, c, 10, 0), "addc"),
        (extended(a, b, c, 8, 0), "subfc"),
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
            "rlwinm wrap",
        ),
        (
            (20 << 26) | (b << 21) | (a << 16) | (4 << 11) | (8 << 6) | (24 << 1),
            "rlwimi",
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
    ]
}

/// Runs every instruction on hardware and through the model, and compares.
#[test]
fn the_model_computes_what_the_hardware_computes() {
    // An instruction the model cannot write out cannot be run through it. That
    // is reported rather than treated as agreement, since a check that did not
    // run says nothing.
    let all = subjects();
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
            for byte in (0..MEMORY_SIZE).step_by(8) {
                let to = xer_at + 8 + byte;
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
fn sequences_on_the_model(
    sequences: &[(&str, Vec<u32>)],
    seeds: &[[u64; WATCHED_COUNT]],
) -> Result<Vec<State>, String> {
    let Some(compiler) = host_compiler() else {
        return Err("skip: no host C compiler is installed".to_owned());
    };

    let pattern: Vec<String> = memory_seed().iter().map(u8::to_string).collect();
    let mut out = String::from(
        "#include \"xenolith.h\"\n#include <stdio.h>\n#include <string.h>\n\n\
         void xenolith_dispatch(xenolith_context *c, uint8_t *b, uint32_t a) { (void)c; (void)b; (void)a; }\n\
         void xenolith_trap(xenolith_context *c, uint8_t *b, uint32_t a) { (void)c; (void)b; (void)a; }\n\n\
         static uint32_t packed(const xenolith_context *ctx) {\n\
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
            for (register, value) in WATCHED.iter().zip(seed) {
                let _ = writeln!(out, "        state.r[{register}] = {value}ull;");
            }
            let _ = writeln!(out, "        {name}(&state, memory);");
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
            let _ = writeln!(
                out,
                "        for (int b = 0; b < {MEMORY_SIZE}; b++) {{ printf(\"%02x\", memory[0x{MEMORY_BASE:x} + b]); }}"
            );
            out.push_str("        printf(\"\\n\");\n    }\n");
        }
    }
    out.push_str("    return 0;\n}\n");

    let directory = workspace("sequence-model")?;
    std::fs::write(directory.join("xenolith.h"), RUNTIME_HEADER)
        .map_err(|error| format!("writing the header: {error}"))?;
    std::fs::write(directory.join("model.c"), out)
        .map_err(|error| format!("writing the model program: {error}"))?;

    let built = Command::new(compiler)
        .args(["-std=c17", "-O1", "-Wno-infinite-recursion", "-o"])
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

    vec![
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
