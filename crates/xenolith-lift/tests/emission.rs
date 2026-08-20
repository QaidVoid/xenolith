//! Checking that emitted C says what the instructions said, and compiles.
//!
//! There is no PowerPC execution on hand, so nothing here can run the original
//! instructions and the emitted C and compare. What can be checked is that the
//! emitted code compiles under warnings, that its control flow matches the graph
//! the analysis produced, and that a function holding anything unmodelled is
//! refused whole rather than emitted with a hole in it.

use std::path::PathBuf;
use std::process::Command;

use xenolith_analysis::analyze;
use xenolith_lift::{Imported, Imports, RUNTIME_HEADER, lift};
use xenolith_xex::{Image, PageKind, Section};

/// The address images are placed at, matching where a title loads.
const BASE: u32 = 0x8200_0000;

/// Builds an image over `words`, all one executable section.
fn image_of(words: &[u32]) -> Image {
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
    let size = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    let sections = vec![Section {
        start: BASE,
        size,
        kind: PageKind::Code,
    }];
    Image::new(BASE, bytes, sections).with_entry_point(Some(BASE))
}

/// Lifts the function at the image's entry, or says why it could not be.
fn lift_entry(words: &[u32]) -> Result<String, String> {
    let image = image_of(words);
    let program = analyze(&image, &[]);
    let function = program
        .functions()
        .find(|function| function.start == BASE)
        .ok_or_else(|| "nothing was discovered at the entry point".to_owned())?;

    lift(&image, function, &Imports::new())
        .map(|lifted| lifted.code)
        .map_err(|unlifted| {
            format!(
                "{:#010x} stopped at {:#010x} on {}",
                unlifted.function, unlifted.address, unlifted.mnemonic
            )
        })
}

/// Builds and runs a whole C program against the interface, returning what it
/// printed, or nothing when there is no compiler to build it with.
///
/// The interface computes some things itself rather than declaring them, and
/// what it computes has to be checked by running it. Reading it is not a check.
fn run_program(name: &str, program: &str) -> Option<String> {
    let usable = Command::new("clang")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !usable {
        eprintln!("skipping the run: clang is not installed");
        return None;
    }

    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::create_dir_all(&directory);
    let _ = std::fs::write(directory.join("xenolith.h"), RUNTIME_HEADER);

    let source = directory.join("program.c");
    let _ = std::fs::write(&source, program);

    let binary = directory.join("program");
    let built = Command::new("clang")
        .args(["-std=c17", "-Wall", "-Wextra", "-Werror", "-O2"])
        .arg("-o")
        .arg(&binary)
        .arg(&source)
        .output()
        .ok()?;
    assert!(
        built.status.success(),
        "{}\n--- program ---\n{program}",
        String::from_utf8_lossy(&built.stderr)
    );

    let ran = Command::new(&binary).output().ok()?;
    assert!(ran.status.success(), "the program exited with a failure");
    Some(String::from_utf8_lossy(&ran.stdout).into_owned())
}

/// Compiles emitted C against the interface, returning what the compiler said.
fn compiles(name: &str, emitted: &str) -> Result<(), String> {
    let usable = Command::new("clang")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !usable {
        eprintln!("skipping the compile: clang is not installed");
        return Ok(());
    }

    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&directory).map_err(|error| format!("making a directory: {error}"))?;
    std::fs::write(directory.join("xenolith.h"), RUNTIME_HEADER)
        .map_err(|error| format!("writing the header: {error}"))?;

    let source = directory.join("lifted.c");
    let program = format!(
        "#include \"xenolith.h\"\n\n\
         void xenolith_dispatch(xenolith_context *c, uint8_t *b, uint32_t a) {{ (void)c; (void)b; (void)a; }}\n\
         void xenolith_trap(xenolith_context *c, uint8_t *b, uint32_t a) {{ (void)c; (void)b; (void)a; }}\n\
         void xenolith_import(xenolith_context *c, uint8_t *b, const char *l, uint32_t o) {{ (void)c; (void)b; (void)l; (void)o; }}\n\
         uint32_t xenolith_reserve32(const uint8_t *b, uint32_t a) {{ return xenolith_load32(b, a); }}\n\
         uint64_t xenolith_reserve64(const uint8_t *b, uint32_t a) {{ return xenolith_load64(b, a); }}\n\
         uint8_t xenolith_conditional32(uint8_t *b, uint32_t a, uint32_t v) {{ xenolith_store32(b, a, v); return 1; }}\n\
         uint8_t xenolith_conditional64(uint8_t *b, uint32_t a, uint64_t v) {{ xenolith_store64(b, a, v); return 1; }}\n\
         uint64_t xenolith_timebase(void) {{ return 0; }}\n\n\
         {emitted}"
    );
    std::fs::write(&source, &program).map_err(|error| format!("writing the source: {error}"))?;

    let output = Command::new("clang")
        // A guest function that recurses on every path is a statement about
        // the program that was translated, not about the translation. That
        // warning reads intent, and lifted code has none to read.
        .args([
            "-std=c17",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wno-infinite-recursion",
            "-c",
        ])
        .arg("-o")
        .arg(directory.join("lifted.o"))
        .arg(&source)
        .output()
        .map_err(|error| format!("running the compiler: {error}"))?;

    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "{}\n--- emitted ---\n{program}",
        String::from_utf8_lossy(&output.stderr)
    ))
}

/// The smallest function there is: do something, then go back.
#[test]
fn a_single_block_function_is_emitted() {
    // addi r3, r0, 1 then blr
    let emitted = lift_entry(&[0x3860_0001, 0x4e80_0020]).expect("it should lift");

    assert!(emitted.contains("void sub_82000000(xenolith_context *ctx, uint8_t *base)"));
    assert!(emitted.contains("ctx->r[3] = (uint64_t)(int64_t)(1);"));
    assert!(emitted.contains("return;"));
    assert!(
        emitted.contains("/* 0x82000000  li r3, 1 */"),
        "the disassembly should sit beside the code: {emitted}"
    );

    if let Err(complaint) = compiles("single_block", &emitted) {
        panic!("emitted C did not compile:\n{complaint}");
    }
}

/// Blocks are written in address order, but nothing may depend on that, so a
/// function jumps to its entry rather than falling into whichever block came
/// first.
#[test]
fn the_entry_block_is_reached_however_the_blocks_were_written() {
    let emitted = lift_entry(&[0x3860_0001, 0x4e80_0020]).expect("it should lift");

    let goto = emitted.find("goto loc_82000000;").expect("an entry jump");
    let label = emitted.find("loc_82000000:").expect("an entry label");
    assert!(goto < label, "the jump should precede the label: {emitted}");
}

/// A loop is the case where a block is reached from below as well as above.
#[test]
fn a_loop_becomes_a_branch_backward() {
    let emitted = lift_entry(&[
        // addi r3, r3, -1
        0x3863_ffff,
        // cmplwi r3, 0
        0x2803_0000,
        // bne back two instructions
        0x4082_fff8,
        0x4e80_0020,
    ])
    .expect("it should lift");

    assert!(
        emitted.contains("goto loc_82000000;"),
        "the loop should branch back: {emitted}"
    );
    if let Err(complaint) = compiles("loop", &emitted) {
        panic!("emitted C did not compile:\n{complaint}");
    }
}

/// A call records where to come back to before going.
#[test]
fn a_call_records_the_return_address() {
    let emitted = lift_entry(&[
        // bl forward eight bytes, to a callee that returns
        0x4800_0009,
        0x4e80_0020,
        0x4e80_0020,
    ])
    .expect("it should lift");

    assert!(
        emitted.contains("ctx->lr = 0x82000004u;"),
        "the link register should hold the address after the call: {emitted}"
    );
    assert!(emitted.contains("sub_82000008(ctx, base);"));
}

/// A branch out of the function is a tail call, which returns once the callee
/// has rather than continuing in a frame that is no longer its own.
#[test]
fn a_tail_call_returns_after_calling() {
    let words = [
        // bl to the second function so it is discovered, then return
        0x4800_0009,
        0x4e80_0020,
        // the second function tail calls the first, branching without a link
        0x4bff_fff8,
        0x4e80_0020,
    ];
    let image = image_of(&words);
    let program = analyze(&image, &[]);

    let mut found = false;
    for function in program.functions() {
        if let Ok(lifted) = lift(&image, function, &Imports::new()) {
            if lifted.code.contains("(ctx, base); return;") {
                found = true;
            }
        }
    }
    assert!(found, "a tail call should call and then return");
}

/// A function holding one instruction with no semantics is refused whole. Code
/// that is right except in one place compiles and runs and is wrong.
#[test]
fn one_unmodelled_instruction_stops_the_whole_function() {
    // addi r3, r0, 1 then vsl, which has no semantics here, then blr
    let outcome = lift_entry(&[0x3860_0001, 0x1000_01c4, 0x4e80_0020]);

    let complaint = outcome.expect_err("it should not lift");
    assert!(
        complaint.contains("0x82000004"),
        "the address that stopped it should be named: {complaint}"
    );
}

/// One function failing must not stop the others.
#[test]
fn other_functions_still_lift() {
    let words = [
        // a function that calls the next and returns
        0x4800_0009,
        0x4e80_0020,
        // a function holding something unmodelled
        0x1000_01c4,
        0x4e80_0020,
    ];
    let image = image_of(&words);
    let program = analyze(&image, &[]);

    let lifted = program
        .functions()
        .filter(|function| lift(&image, function, &Imports::new()).is_ok())
        .count();
    let refused = program
        .functions()
        .filter(|function| lift(&image, function, &Imports::new()).is_err())
        .count();

    assert!(lifted > 0, "the sound function should still be emitted");
    assert!(refused > 0, "the unsound one should still be refused");
}

/// Lifts a real title and compiles what came out.
///
/// The unit tests above check shapes someone thought to write. This checks that
/// the emitter survives a million real instructions and that a C compiler accepts
/// the result, which is the strongest statement available about the emitted code
/// while there is no way to run it.
#[test]
fn a_real_title_lifts_and_compiles() {
    let Some(path) = std::env::var_os("XENOLITH_ANALYSIS_XEX") else {
        eprintln!("skipping: XENOLITH_ANALYSIS_XEX is not set");
        return;
    };
    let bytes = std::fs::read(&path).expect("reading the container");
    let container = xenolith_xex::Container::parse(&bytes).expect("parsing the container");
    let key = std::env::var("XENOLITH_XEX_KEY")
        .ok()
        .map(|text| xenolith_xex::KeyMaterial::from_hex(text.trim()).expect("the supplied key"));
    let image = container.load(key.as_ref()).expect("decoding the image");

    let program = analyze(&image, &[]);

    let mut lifted = 0u64;
    let mut refused = 0u64;
    let mut blocked: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    let mut referenced: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    let mut emitted = String::new();

    for function in program.functions() {
        match lift(&image, function, &Imports::new()) {
            Ok(result) => {
                lifted += 1;
                referenced.extend(result.calls);
                // Every function is compiled rather than a sample. A sample of
                // two thousand missed a function with no blocks at all, whose
                // entry jumped to a label that was never written.
                emitted.push_str(&result.code);
                emitted.push('\n');
            }
            Err(unlifted) => {
                refused += 1;
                *blocked.entry(unlifted.mnemonic).or_default() += 1;
            }
        }
    }

    let total = lifted + refused;
    eprintln!("functions           {total:>10}");
    eprintln!(
        "  lifted            {lifted:>10}  ({} percent)",
        lifted * 100 / total.max(1)
    );
    eprintln!("  refused           {refused:>10}");

    let mut ranked: Vec<(&str, u64)> = blocked.into_iter().collect();
    ranked.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    eprintln!("\ninstructions blocking the most functions");
    for (mnemonic, count) in ranked.iter().take(12) {
        eprintln!("  {mnemonic:<12} {count:>10}");
    }

    assert!(lifted > 0, "nothing lifted at all");

    // Everything the emitted code names is declared, which is more than the
    // discovered functions: a call into a register save helper lands partway
    // through one, and discovery does not claim those.
    for function in program.functions() {
        referenced.insert(function.start);
    }
    let mut declarations = String::new();
    for address in &referenced {
        declarations.push_str(&xenolith_lift::declaration_of(*address));
    }

    let source = format!("{declarations}\n{emitted}");
    if let Err(complaint) = compiles("real_title", &source) {
        let head: String = complaint.lines().take(30).collect::<Vec<_>>().join("\n");
        panic!("emitted C did not compile:\n{head}");
    }
}

/// A thunk stands for a function the console provided, so the only thing that
/// can be emitted for it is a call naming what the container says it is.
#[test]
fn an_import_thunk_becomes_a_call() {
    // The two placeholder words the loader overwrites, then mtctr r11; bctr.
    const WORDS: [u32; 4] = [0x0100_028b, 0x0200_028b, 0x7d69_03a6, 0x4e80_0420];

    let image = image_of(&WORDS);
    let program = analyze(&image, &[]);
    let function = program
        .functions()
        .find(|function| function.start == BASE)
        .expect("the entry point should be discovered");

    let mut imports = Imports::new();
    imports.insert(
        BASE,
        Imported {
            library: "xam.xex".to_owned(),
            ordinal: 651,
        },
    );

    let lifted = lift(&image, function, &imports).expect("a thunk should lift");

    assert!(
        lifted
            .code
            .contains("xenolith_import(ctx, base, \"xam.xex\", 651);"),
        "the call was not emitted: {}",
        lifted.code
    );
    assert!(
        lifted.code.contains("import: xam.xex ordinal 651"),
        "what it stands for was not stated: {}",
        lifted.code
    );
    assert!(
        lifted.calls.is_empty(),
        "a thunk calls no lifted function: {:?}",
        lifted.calls
    );
    assert_eq!(compiles("import_thunk", &lifted.code), Ok(()));
}

/// Without the container saying so, the same words are what they look like,
/// which is not an instruction.
#[test]
fn a_thunk_with_no_import_behind_it_does_not_lift() {
    const WORDS: [u32; 4] = [0x0100_028b, 0x0200_028b, 0x7d69_03a6, 0x4e80_0420];

    assert!(
        lift_entry(&WORDS).is_err(),
        "placeholder words are not instructions"
    );
}

/// The reference count idiom is the most common shape in either title that was
/// unmodelled, and the branch after the conditional store is what it rests on.
#[test]
fn a_reservation_retry_lifts() {
    // mflr r12; stwu r1, -96(r1);
    // lwarx r10, r0, r11; addi r10, r10, 1; stwcx. r10, r0, r11;
    // bc 4, 2, back to the lwarx; addi r1, r1, 96; blr
    const WORDS: [u32; 8] = [
        0x7d88_02a6,
        0x9421_ffa0,
        0x7d40_5828,
        0x394a_0001,
        0x7d40_592d,
        0x4082_fff4,
        0x3821_0060,
        0x4e80_0020,
    ];

    let emitted = lift_entry(&WORDS).expect("the reservation idiom should lift");

    assert!(
        emitted.contains("ctx->r[10] = xenolith_reserve32(base, address);"),
        "the reserved load was not emitted: {emitted}"
    );
    assert!(
        emitted.contains(
            "ctx->cr[0].eq = xenolith_conditional32(base, address, (uint32_t)ctx->r[10]);"
        ),
        "the conditional store was not emitted: {emitted}"
    );
    assert!(
        emitted.contains("ctx->cr[0].lt = 0;"),
        "the field's leading bits were not cleared: {emitted}"
    );
    assert!(
        emitted.contains("if (!ctx->cr[0].eq)"),
        "the retry did not read the bit the store set: {emitted}"
    );
    // The low bit of a conditional store is part of its spelling rather than a
    // record bit. Treating it as one appended a comparison that overwrote the
    // outcome of the store, and the retry then never stopped retrying.
    assert!(
        !emitted.contains("ctx->cr[0].eq = ctx->r[10] == 0;"),
        "the store's outcome was overwritten by a comparison: {emitted}"
    );
    assert_eq!(compiles("reservation", &emitted), Ok(()));
}

/// Packing eight condition fields into a word and back is where a bit order
/// mistake would live, and a mistake made in both directions would cancel out.
#[test]
fn the_condition_register_round_trips() {
    // mflr r12; stwu r1, -96(r1); mfcr r3; mtcrf 255, r3;
    // addi r1, r1, 96; blr
    const WORDS: [u32; 6] = [
        0x7d88_02a6,
        0x9421_ffa0,
        0x7c60_0026,
        0x7c6f_f120,
        0x3821_0060,
        0x4e80_0020,
    ];

    let emitted = lift_entry(&WORDS).expect("the round trip should lift");

    assert!(
        emitted.contains("ctx->r[3] = xenolith_condition_pack(ctx->cr);"),
        "packing was not emitted: {emitted}"
    );
    assert!(
        emitted.contains("xenolith_condition_unpack(ctx->cr, (uint32_t)ctx->r[3], 255u);"),
        "unpacking was not emitted: {emitted}"
    );
    assert_eq!(compiles("condition_round_trip", &emitted), Ok(()));
}

/// The two registers held as storage, and the counter that is not.
#[test]
fn the_system_registers_lift() {
    // mflr r12; stwu r1, -96(r1);
    // mfmsr r9; mtmsrd r9, 1; mffs f0; mtfsf 255, f0; mftb r10;
    // addi r1, r1, 96; blr
    const WORDS: [u32; 9] = [
        0x7d88_02a6,
        0x9421_ffa0,
        0x7d20_00a6,
        0x7d21_0164,
        0xfc00_048e,
        0xfdfe_058e,
        0x7d4c_42e6,
        0x3821_0060,
        0x4e80_0020,
    ];

    let emitted = lift_entry(&WORDS).expect("the system registers should lift");

    assert!(emitted.contains("ctx->r[9] = ctx->msr;"), "{emitted}");
    assert!(emitted.contains("ctx->msr = ctx->r[9];"), "{emitted}");
    assert!(emitted.contains("ctx->f[0].u64 = ctx->fpscr"), "{emitted}");
    assert!(
        emitted.contains("ctx->r[10] = xenolith_timebase();"),
        "the time base was not read through the runtime: {emitted}"
    );
    assert_eq!(compiles("system_registers", &emitted), Ok(()));
}

/// A logical between condition bits writes one bit and leaves the rest, and a
/// field move takes the whole field.
#[test]
fn the_condition_register_logicals_lift() {
    // mflr r12; stwu r1, -96(r1);
    // cror 2, 0, 1; crand 6, 4, 5; crnot-shaped crnor 10, 8, 9; mcrf cr1, cr7;
    // addi r1, r1, 96; blr
    const WORDS: [u32; 8] = [
        0x7d88_02a6,
        0x9421_ffa0,
        0x4c40_0b82,
        0x4cc4_2a02,
        0x4d48_4842,
        0x4c9c_0000,
        0x3821_0060,
        0x4e80_0020,
    ];

    let emitted = lift_entry(&WORDS).expect("the logicals should lift");

    assert!(
        emitted.contains("ctx->cr[0].eq = (uint8_t)(ctx->cr[0].lt | ctx->cr[0].gt) & 1;"),
        "the or was not emitted: {emitted}"
    );
    assert!(
        emitted.contains("ctx->cr[1].eq = (uint8_t)(ctx->cr[1].lt & ctx->cr[1].gt) & 1;"),
        "the and was not emitted: {emitted}"
    );
    assert!(
        emitted
            .contains("ctx->cr[2].eq = (uint8_t)((uint8_t)!(ctx->cr[2].lt | ctx->cr[2].gt)) & 1;"),
        "the nor was not emitted: {emitted}"
    );
    assert!(
        emitted.contains("ctx->cr[1] = ctx->cr[7];"),
        "the field move was not emitted: {emitted}"
    );
    assert_eq!(compiles("condition_logicals", &emitted), Ok(()));
}

/// Byte order is what these are for, so the emitted code has to assemble the
/// bytes in the order the instruction names rather than swap after the fact.
#[test]
fn the_byte_reversed_accesses_lift() {
    // mflr r12; stwu r1, -96(r1);
    // lwbrx r3, 0, r4; sthbrx r3, 0, r4; lhbrx r5, 0, r4; stwbrx r5, 0, r4;
    // addi r1, r1, 96; blr
    const WORDS: [u32; 8] = [
        0x7d88_02a6,
        0x9421_ffa0,
        0x7c60_242c,
        0x7c60_272c,
        0x7ca0_262c,
        0x7ca0_252c,
        0x3821_0060,
        0x4e80_0020,
    ];

    let emitted = lift_entry(&WORDS).expect("the byte reversed accesses should lift");

    assert!(
        emitted.contains("(uint32_t)xenolith_load8(base, address + 0) << 0")
            && emitted.contains("(uint32_t)xenolith_load8(base, address + 3) << 24"),
        "the word load did not assemble its bytes in reverse: {emitted}"
    );
    assert!(
        emitted.contains("xenolith_store8(base, address + 0, (uint8_t)(ctx->r[3] >> 0));"),
        "the halfword store did not write its low byte first: {emitted}"
    );
    assert_eq!(compiles("byte_reversed", &emitted), Ok(()));
}

/// The high half of a doubleword product needs a type C does not have, so it
/// is built by hand and has to be checked against known answers.
#[test]
fn the_doubleword_multiplies_lift() {
    // mflr r12; stwu r1, -96(r1); mulhdu r3, r4, r5; mulhd r6, r4, r5;
    // addi r1, r1, 96; blr
    const WORDS: [u32; 6] = [
        0x7d88_02a6,
        0x9421_ffa0,
        0x7c64_2812,
        0x7cc4_2892,
        0x3821_0060,
        0x4e80_0020,
    ];

    let emitted = lift_entry(&WORDS).expect("the multiplies should lift");

    assert!(
        emitted.contains("ctx->r[3] = xenolith_multiply_high(ctx->r[4], ctx->r[5]);"),
        "the unsigned high half was not emitted: {emitted}"
    );
    assert!(
        emitted.contains("xenolith_multiply_high_signed((int64_t)ctx->r[4], (int64_t)ctx->r[5])"),
        "the signed high half was not emitted: {emitted}"
    );
    assert_eq!(compiles("doubleword_multiply", &emitted), Ok(()));
}

/// The interface computes the high half itself, so the answers it gives are
/// worth checking against ones worked out another way.
#[test]
fn the_high_half_matches_a_wider_type() {
    let cases: [(u64, u64); 6] = [
        (0, 0),
        (1, 1),
        (u64::MAX, u64::MAX),
        (u64::MAX, 2),
        (0x0123_4567_89ab_cdef, 0xfedc_ba98_7654_3210),
        (1 << 63, 1 << 63),
    ];

    let mut program =
        String::from("#include \"xenolith.h\"\n#include <stdio.h>\nint main(void) {\n");
    for (left, right) in cases {
        let _ = std::fmt::Write::write_fmt(
            &mut program,
            format_args!(
                "  printf(\"%llu %lld\\n\", (unsigned long long)xenolith_multiply_high({left}ull, {right}ull), (long long)xenolith_multiply_high_signed((int64_t){left}ull, (int64_t){right}ull));\n"
            ),
        );
    }
    program.push_str("  return 0;\n}\n");

    let Some(output) = run_program("multiply_high", &program) else {
        return;
    };

    let expected: Vec<String> = cases
        .iter()
        .map(|(left, right)| {
            let unsigned =
                u64::try_from((u128::from(*left) * u128::from(*right)) >> 64).unwrap_or(u64::MAX);
            let wide = |value: u64| i128::from(i64::from_ne_bytes(value.to_ne_bytes()));
            let signed = i64::try_from((wide(*left) * wide(*right)) >> 64).unwrap_or(i64::MAX);
            format!("{unsigned} {signed}")
        })
        .collect();

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        expected,
        "the high half disagrees with a wider type"
    );
}

/// A block is zeroed whole, and nothing either side of it is touched.
#[test]
fn zeroing_a_block_stays_inside_it() {
    let program = String::from(
        "#include \"xenolith.h\"\n#include <stdio.h>\n\
         int main(void) {\n\
         \x20 static uint8_t memory[256];\n\
         \x20 for (unsigned i = 0; i < 256; i++) { memory[i] = 0xaa; }\n\
         \x20 xenolith_zero_block(memory, 100, 32);\n\
         \x20 unsigned first = 256, last = 0, count = 0;\n\
         \x20 for (unsigned i = 0; i < 256; i++) {\n\
         \x20   if (memory[i] == 0) { if (i < first) first = i; last = i; count++; }\n\
         \x20 }\n\
         \x20 printf(\"%u %u %u\\n\", first, last, count);\n\
         \x20 return 0;\n\
         }\n",
    );

    let Some(output) = run_program("zero_block", &program) else {
        return;
    };

    assert_eq!(
        output.trim(),
        "96 127 32",
        "the zeroed range was not the block containing the address"
    );
}

/// The whole change rests on this: a lane written through one width and read
/// through another has to agree with what the guest's byte order says, and a
/// mistake would be invisible on one host and wrong on the other.
#[test]
fn the_vector_views_agree_about_byte_order() {
    let program = String::from(
        "#include \"xenolith.h\"\n#include <stdio.h>\n\
         int main(void) {\n\
         \x20 xenolith_vector v;\n\
         \x20 for (unsigned i = 0; i < 16; i++) { xenolith_vector_set_u8(&v, i, (uint8_t)(0x10 + i)); }\n\
         \x20 printf(\"%04x %08x %016llx\\n\", xenolith_vector_u16(&v, 0),\n\
         \x20        xenolith_vector_u32(&v, 0), (unsigned long long)xenolith_vector_u64(&v, 0));\n\
         \x20 printf(\"%04x %08x %016llx\\n\", xenolith_vector_u16(&v, 7),\n\
         \x20        xenolith_vector_u32(&v, 3), (unsigned long long)xenolith_vector_u64(&v, 1));\n\
         \x20 xenolith_vector_set_u32(&v, 2, 0xdeadbeefu);\n\
         \x20 printf(\"%02x%02x%02x%02x\\n\", xenolith_vector_u8(&v, 8), xenolith_vector_u8(&v, 9),\n\
         \x20        xenolith_vector_u8(&v, 10), xenolith_vector_u8(&v, 11));\n\
         \x20 xenolith_vector_set_f32(&v, 1, 1.5f);\n\
         \x20 printf(\"%08x %g\\n\", xenolith_vector_u32(&v, 1), (double)xenolith_vector_f32(&v, 1));\n\
         \x20 return 0;\n\
         }\n",
    );

    let Some(output) = run_program("vector_lanes", &program) else {
        return;
    };

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        [
            // The first halfword, word, and doubleword are the leading bytes.
            "1011 10111213 1011121314151617",
            // The last of each is the trailing bytes.
            "1e1f 1c1d1e1f 18191a1b1c1d1e1f",
            // A word written lands most significant byte first.
            "deadbeef",
            // A float lane holds its bits, and reads back as the value.
            "3fc00000 1.5",
        ],
        "a lane written through one width disagrees with another"
    );
}

/// The vector families that work lane by lane, checked for where each lane
/// comes from rather than only for compiling.
#[test]
fn the_vector_lane_operations_lift() {
    // mflr r12; stwu r1, -96(r1);
    // vand v1,v2,v3; vsel v1,v2,v3,v4; vspltisw v1,-1; vspltw v1,v3,2;
    // vmrghw v1,v2,v3; vmrglw v1,v2,v3; vsubfp v1,v2,v3; vmaddfp v1,v2,v3,v4;
    // addi r1, r1, 96; blr
    const WORDS: [u32; 12] = [
        0x7d88_02a6,
        0x9421_ffa0,
        0x1022_1c04,
        0x1022_192a,
        0x103f_038c,
        0x1022_1a8c,
        0x1022_188c,
        0x1022_198c,
        0x1022_184a,
        0x1022_192e,
        0x3821_0060,
        0x4e80_0020,
    ];

    let emitted = lift_entry(&WORDS).expect("the vector operations should lift");

    assert!(
        emitted.contains(
            "xenolith_vector_set_u32(&t, lane, xenolith_vector_u32(&ctx->v[2], lane) & xenolith_vector_u32(&ctx->v[3], lane));"
        ),
        "the bitwise and was not emitted: {emitted}"
    );
    assert!(
        emitted.contains("(uint32_t)(int32_t)(-1)"),
        "the immediate was not sign extended: {emitted}"
    );
    assert!(
        emitted.contains("xenolith_vector_u32(&ctx->v[3], 2)"),
        "the splat did not take the lane the encoding names: {emitted}"
    );
    assert!(
        emitted.contains("lane / 2 + 0") && emitted.contains("lane / 2 + 2"),
        "the two merges did not take different halves: {emitted}"
    );
    assert!(
        emitted.contains(
            "xenolith_vector_f32(&ctx->v[2], lane) - xenolith_vector_f32(&ctx->v[3], lane)"
        ),
        "the float subtraction was not emitted: {emitted}"
    );
    assert!(
        emitted.contains(
            "__builtin_fmaf(xenolith_vector_f32(&ctx->v[2], lane), xenolith_vector_f32(&ctx->v[4], lane), xenolith_vector_f32(&ctx->v[3], lane))"
        ),
        "the fused multiply took the wrong operands: {emitted}"
    );
    // Every lane goes into a temporary, so an instruction naming its
    // destination as a source cannot read a lane it has already written.
    assert_eq!(
        emitted.matches("xenolith_vector t;").count(),
        8,
        "not every lane operation built its result somewhere else: {emitted}"
    );
    assert_eq!(compiles("vector_lanes_emitted", &emitted), Ok(()));
}

/// A comparison writes a lane mask, and the recording form also writes the
/// field a branch after it reads.
#[test]
fn the_vector_comparisons_lift() {
    // mflr r12; stwu r1, -96(r1);
    // vcmpeqfp. v1,v2,v3; vcmpeqfp v4,v2,v3; vcfsx v1,v3,2; vctuxs v1,v3,2;
    // addi r1, r1, 96; blr
    const WORDS: [u32; 8] = [
        0x7d88_02a6,
        0x9421_ffa0,
        0x1022_1cc6,
        0x1082_18c6,
        0x1022_1b4a,
        0x1022_1b8a,
        0x3821_0060,
        0x4e80_0020,
    ];

    let emitted = lift_entry(&WORDS).expect("the comparisons should lift");

    assert_eq!(
        emitted.matches("ctx->cr[6].lt = (uint8_t)all;").count(),
        1,
        "only the recording form should write the condition field: {emitted}"
    );
    assert!(
        emitted.contains("? (uint32_t)~(uint32_t)0 : 0;"),
        "a matching lane was not filled with ones: {emitted}"
    );
    // The scale divides on the way to a float and multiplies on the way back.
    assert!(
        emitted.contains(") / 4.0f)"),
        "the scale did not divide: {emitted}"
    );
    assert!(
        emitted.contains("xenolith_saturate_unsigned(") && emitted.contains("* 4.0f)"),
        "the scale did not multiply, or the result was not clamped: {emitted}"
    );
    assert_eq!(compiles("vector_compare_emitted", &emitted), Ok(()));
}

/// An unaligned load is the one place where reading the right bytes in the
/// wrong order gives a plausible answer, so this runs one over a patterned
/// buffer at every alignment and reads back what landed.
#[test]
fn an_unaligned_vector_load_takes_the_right_bytes() {
    // lvlx v1, 0, r3; blr
    let emitted = lift_entry(&[0x7c20_1c0e, 0x4e80_0020]).expect("the load should lift");

    let program = format!(
        "#include \"xenolith.h\"\n#include <stdio.h>\n\
         void xenolith_dispatch(xenolith_context *c, uint8_t *b, uint32_t a) {{ (void)c; (void)b; (void)a; }}\n\
         void xenolith_trap(xenolith_context *c, uint8_t *b, uint32_t a) {{ (void)c; (void)b; (void)a; }}\n\n\
         {emitted}\n\
         int main(void) {{\n\
         \x20 static uint8_t memory[256];\n\
         \x20 for (unsigned i = 0; i < 256; i++) {{ memory[i] = (uint8_t)i; }}\n\
         \x20 for (unsigned offset = 0; offset < 4; offset++) {{\n\
         \x20   xenolith_context ctx = {{0}};\n\
         \x20   ctx.r[3] = 64 + offset;\n\
         \x20   sub_82000000(&ctx, memory);\n\
         \x20   for (unsigned lane = 0; lane < 16; lane++) {{\n\
         \x20     printf(\"%02x\", xenolith_vector_u8(&ctx.v[1], lane));\n\
         \x20   }}\n\
         \x20   printf(\"\\n\");\n\
         \x20 }}\n\
         \x20 return 0;\n\
         }}\n"
    );

    let Some(output) = run_program("unaligned_vector_load", &program) else {
        return;
    };

    // At offset n the load takes the bytes at the address and the ones after
    // it up to the end of the block, and writes zeroes over the rest.
    let expected: Vec<String> = (0..4u32)
        .map(|offset| {
            (0..16u32)
                .map(|lane| {
                    if lane + offset < 16 {
                        format!("{:02x}", 64 + offset + lane)
                    } else {
                        "00".to_owned()
                    }
                })
                .collect()
        })
        .collect();

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        expected,
        "the unaligned load did not take the bytes the address names"
    );
}

/// The console's forms carry one register field fewer than the standard ones,
/// so an instruction needing three sources takes the third from the field it
/// writes. Reading the second source twice instead compiles and is wrong.
#[test]
fn a_console_fused_multiply_reads_what_it_writes() {
    // vmaddfp128 v11, v126, v0; vor128 v127, v0, v0; blr
    let emitted = lift_entry(&[0x157e_04f0, 0x17e0_02dc, 0x4e80_0020]).expect("both should lift");

    assert!(
        emitted.contains(
            "__builtin_fmaf(xenolith_vector_f32(&ctx->v[126], lane), xenolith_vector_f32(&ctx->v[0], lane), xenolith_vector_f32(&ctx->v[11], lane))"
        ),
        "the accumulate source was not the register written: {emitted}"
    );
    // A vector register is moved with an or of one with itself, so the two
    // sources of one have to name the same register. They did not while the
    // first source was read from a bit the opcode owns.
    assert!(
        emitted.contains("/* 0x82000004  vor128 v127, v0, v0 */"),
        "the move idiom did not decode with equal sources: {emitted}"
    );
    assert_eq!(compiles("console_fused_multiply", &emitted), Ok(()));
}

/// A saturating form stops at the end of the range rather than wrapping, and a
/// clamp at the wrong bound gives an answer that is almost right, which is the
/// kind that survives a reading.
#[test]
fn the_saturating_bounds_are_where_the_range_ends() {
    // vaddshs v1, v2, v3; vsubuhs v4, v2, v3; blr
    let emitted = lift_entry(&[0x1022_1b40, 0x1082_1e40, 0x4e80_0020]).expect("both should lift");

    let program = format!(
        "#include \"xenolith.h\"\n#include <stdio.h>\n\
         void xenolith_dispatch(xenolith_context *c, uint8_t *b, uint32_t a) {{ (void)c; (void)b; (void)a; }}\n\
         void xenolith_trap(xenolith_context *c, uint8_t *b, uint32_t a) {{ (void)c; (void)b; (void)a; }}\n\n\
         {emitted}\n\
         int main(void) {{\n\
         \x20 static uint8_t memory[16];\n\
         \x20 xenolith_context ctx = {{0}};\n\
         \x20 const uint16_t left[8]  = {{0x7fff, 0x8000, 0x7fff, 0x8000, 0x0001, 0xffff, 0x0000, 0x1234}};\n\
         \x20 const uint16_t right[8] = {{0x0001, 0xffff, 0xffff, 0x0001, 0x0002, 0x0001, 0xffff, 0x1000}};\n\
         \x20 for (unsigned lane = 0; lane < 8; lane++) {{\n\
         \x20   xenolith_vector_set_u16(&ctx.v[2], lane, left[lane]);\n\
         \x20   xenolith_vector_set_u16(&ctx.v[3], lane, right[lane]);\n\
         \x20 }}\n\
         \x20 sub_82000000(&ctx, memory);\n\
         \x20 for (unsigned lane = 0; lane < 8; lane++) {{ printf(\"%04x\", xenolith_vector_u16(&ctx.v[1], lane)); }}\n\
         \x20 printf(\"\\n\");\n\
         \x20 for (unsigned lane = 0; lane < 8; lane++) {{ printf(\"%04x\", xenolith_vector_u16(&ctx.v[4], lane)); }}\n\
         \x20 printf(\"\\n\");\n\
         \x20 return 0;\n\
         }}\n"
    );

    let Some(output) = run_program("vector_saturation", &program) else {
        return;
    };

    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(
        lines[0],
        // Signed halfwords stop at the ends of the signed range and pass
        // through anything that fits.
        concat!(
            "7fff", "8000", "7ffe", "8001", "0003", "0000", "ffff", "2234"
        ),
        "the signed saturating add did not stop at the signed bounds"
    );
    assert_eq!(
        lines[1],
        // Unsigned halfwords stop at zero rather than wrapping below it.
        concat!(
            "7ffe", "0000", "0000", "7fff", "0000", "fffe", "0000", "0234"
        ),
        "the unsigned saturating subtract did not stop at zero"
    );
}
