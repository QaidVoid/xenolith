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
const WATCHED: [u32; 3] = [3, 4, 5];

/// Where the guest keeps the pointer to its output.
///
/// Callee saved and outside the watched set, so nothing under test can name it.
const OUTPUT_REGISTER: u32 = 31;

/// Bits of the exception register this project models.
///
/// Summary overflow, overflow, and carry. The rest holds state no lifted code
/// reads, and the emulator sets some of it for reasons of its own.
const EXCEPTION_MODELLED: u64 = 0xe000_0000;

/// The state an instruction started from or left behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State {
    /// The watched general purpose registers, in the order of `WATCHED`.
    pub registers: [u64; 3],
    /// The condition register, packed as the architecture packs it.
    pub condition: u32,
    /// The bits of the exception register this project models.
    pub exception: u64,
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
    // r3 holds the seed table and r4 the output on entry, moved somewhere the
    // instructions under test cannot reach.
    out.push_str("\tstd 30, -16(1)\n\tstd 31, -8(1)\n\tmr 30, 3\n\tmr 31, 4\n");

    for seed in 0..seeds {
        let base = seed * WATCHED.len() * 8;
        for word in words {
            out.push_str("\t/* seed */\n");
            for (at, register) in WATCHED.iter().enumerate() {
                let offset = base + at * 8;
                let _ = writeln!(out, "\tld {register}, {offset}(30)");
            }
            // Start from a cleared condition and exception register so that
            // what is read back was written by the instruction.
            out.push_str("\tli 0, 0\n\tmtxer 0\n\tmtcr 0\n");
            let _ = writeln!(out, "\t.long {word}");

            for (at, register) in WATCHED.iter().enumerate() {
                let offset = at * 8;
                let _ = writeln!(out, "\tstd {register}, {offset}({OUTPUT_REGISTER})");
            }
            let _ = writeln!(out, "\tmfcr 0\n\tstw 0, 24({OUTPUT_REGISTER})");
            let _ = writeln!(out, "\tmfxer 0\n\tstd 0, 32({OUTPUT_REGISTER})");
            let _ = writeln!(out, "\taddi {OUTPUT_REGISTER}, {OUTPUT_REGISTER}, 40");
        }
    }

    out.push_str("\tld 30, -16(1)\n\tld 31, -8(1)\n\tblr\n");
    out
}

/// The driver that calls the generated assembly and prints what it found.
const GUEST_DRIVER: &str = r#"
#include <stdio.h>
#include <stdint.h>

void run(const uint64_t *seeds, uint64_t *out);

int main(void) {
    static const uint64_t seeds[SEED_WORDS] = SEED_VALUES;
    static uint64_t out[RESULT_WORDS];

    run(seeds, out);

    for (int i = 0; i < RESULT_COUNT; i++) {
        const uint64_t *r = out + i * 5;
        printf("%016llx %016llx %016llx %08llx %016llx\n",
               (unsigned long long)r[0], (unsigned long long)r[1],
               (unsigned long long)r[2], (unsigned long long)(r[3] >> 32),
               (unsigned long long)r[4]);
    }
    return 0;
}
"#;

/// Runs each encoding on emulated hardware, once per seed.
///
/// Returns the state each left behind, seeds varying slowest.
fn on_hardware(words: &[u32], seeds: &[[u64; 3]]) -> Result<Vec<State>, String> {
    let Some(prefix) = cross_prefix() else {
        return Err("skip: XENOLITH_PPC_TOOLCHAIN names no working toolchain".to_owned());
    };
    let Some(emulator) = emulator() else {
        return Err("skip: no emulator is installed".to_owned());
    };

    let directory = workspace("execution-guest")?;
    std::fs::write(directory.join("run.s"), guest_assembly(words, seeds.len()))
        .map_err(|error| format!("writing the assembly: {error}"))?;

    let flat: Vec<String> = seeds
        .iter()
        .flat_map(|seed| seed.iter().map(|value| format!("{value}ull")))
        .collect();
    let count = words.len() * seeds.len();
    let driver = GUEST_DRIVER
        .replace("SEED_WORDS", &flat.len().to_string())
        .replace("SEED_VALUES", &format!("{{{}}}", flat.join(", ")))
        .replace("RESULT_WORDS", &(count * 5).to_string())
        .replace("RESULT_COUNT", &count.to_string());
    std::fs::write(directory.join("driver.c"), driver)
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
        if fields.len() != 5 {
            continue;
        }
        let number = |at: usize| {
            fields
                .get(at)
                .and_then(|field| u64::from_str_radix(field, 16).ok())
        };
        let (Some(a), Some(b), Some(c), Some(cr), Some(xer)) =
            (number(0), number(1), number(2), number(3), number(4))
        else {
            return Err(format!("could not read the state from {line:?}"));
        };
        states.push(State {
            registers: [a, b, c],
            condition: u32::try_from(cr).unwrap_or(0),
            exception: xer & EXCEPTION_MODELLED,
        });
    }

    Ok(states)
}

/// Writes the host C that runs each encoding through this project's model.
fn model_program(words: &[u32], seeds: &[[u64; 3]]) -> String {
    let mut out = String::from("#include \"xenolith.h\"\n#include <stdio.h>\n\n");
    out.push_str(
        "void xenolith_dispatch(xenolith_context *c, uint8_t *b, uint32_t a) { (void)c; (void)b; (void)a; }\n\
         void xenolith_trap(xenolith_context *c, uint8_t *b, uint32_t a) { (void)c; (void)b; (void)a; }\n\n",
    );
    // The condition register is packed the way the architecture packs it, most
    // significant field first, so the two sides can be compared as one number.
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

    out.push_str("int main(void) {\n    static uint8_t memory[65536];\n    (void)memory;\n");
    for seed in seeds {
        for word in words {
            let instruction = Instruction::decode(*word);
            let Some(code) = xenolith_lift::code_for(instruction, 0) else {
                continue;
            };

            // The emitted code writes through a pointer named ctx, so the
            // state is declared beside it and pointed at rather than the code
            // being rewritten to suit the test.
            out.push_str("    {\n        xenolith_context state = {0};\n");
            out.push_str("        xenolith_context *ctx = &state;\n");
            out.push_str("        uint8_t *base = memory;\n        (void)base;\n");
            out.push_str("        uint32_t address; (void)address;\n");
            for (register, value) in WATCHED.iter().zip(seed) {
                let _ = writeln!(out, "        ctx->r[{register}] = {value}ull;");
            }
            out.push_str(&code);
            let _ = write!(
                out,
                "        printf(\"%016llx %016llx %016llx %08llx %016llx\\n\",\n\
                 \x20               (unsigned long long)state.r[{}], (unsigned long long)state.r[{}],\n\
                 \x20               (unsigned long long)state.r[{}], (unsigned long long)packed(&state),\n\
                 \x20               (unsigned long long)state.xer);\n    }}\n",
                WATCHED[0], WATCHED[1], WATCHED[2]
            );
        }
    }
    out.push_str("    return 0;\n}\n");
    out
}

/// Runs each encoding through the model on the host.
fn on_the_model(words: &[u32], seeds: &[[u64; 3]]) -> Result<Vec<State>, String> {
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
const SEEDS: [[u64; 3]; 4] = [
    [0, 0xdead_beef_cafe_babe, 0x0123_4567_89ab_cdef],
    [0, 0x0000_0000_0000_0007, 0x0000_0000_0000_0003],
    [0, 0xffff_ffff_ffff_ffff, 0x0000_0000_0000_0001],
    [0, 0x8000_0000_8000_0000, 0xffff_ffff_ffff_fffe],
];

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
    let (a, b, c) = (WATCHED[0], WATCHED[1], WATCHED[2]);

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
