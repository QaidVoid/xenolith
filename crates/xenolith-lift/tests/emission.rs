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
         void xenolith_import(xenolith_context *c, uint8_t *b, const char *l, uint32_t o) {{ (void)c; (void)b; (void)l; (void)o; }}\n\n\
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
    // addi r3, r0, 1 then an instruction with no semantics, then blr
    let outcome = lift_entry(&[0x3860_0001, 0x1000_0000, 0x4e80_0020]);

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
        0x1000_0000,
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
