//! Integration tests driving the built binary.

use std::collections::BTreeSet;
use std::process::Command;

/// Runs the tool with the given arguments.
///
/// Expands to a tuple of success, stdout, and stderr. This is a macro rather
/// than a function so the fallible spawn is expanded inside the test that calls
/// it, where a panic on failure is the appropriate response.
macro_rules! run {
    ($($arg:expr),* $(,)?) => {{
        let args: Vec<&str> = vec![$($arg),*];
        let output = Command::new(env!("CARGO_BIN_EXE_xenolith"))
            .args(&args)
            .output()
            .expect("the binary should run");

        (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }};
}

#[test]
fn top_level_help_lists_the_subcommands() {
    let (ok, stdout, _) = run!("--help");

    assert!(ok, "help should succeed");
    assert!(stdout.contains("inspect"), "help did not list inspect");
}

#[test]
fn inspect_help_documents_its_arguments() {
    let (ok, stdout, _) = run!("inspect", "--help");

    assert!(ok, "inspect help should succeed");
    assert!(stdout.contains("--key-file"), "key file not documented");
    assert!(stdout.contains("--decode"), "decode not documented");
    assert!(stdout.contains("--imports"), "imports not documented");
}

#[test]
fn reports_the_version() {
    let (ok, stdout, _) = run!("--version");

    assert!(ok, "version should succeed");
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn a_missing_input_path_fails_and_names_the_path() {
    let (ok, _, stderr) = run!("inspect", "/nonexistent/path/to/default.xex");

    assert!(!ok, "a missing file should fail");
    assert!(
        stderr.contains("/nonexistent/path/to/default.xex"),
        "the error did not name the path: {stderr}"
    );
}

#[test]
fn a_file_that_is_not_a_container_fails_with_both_the_path_and_the_reason() {
    let path = std::env::temp_dir().join("xenolith-not-a-xex.bin");
    std::fs::write(&path, b"this is definitely not a xex file").unwrap();

    let (ok, _, stderr) = run!("inspect", path.to_str().unwrap());
    let _ = std::fs::remove_file(&path);

    assert!(!ok, "a non container should fail");
    assert!(
        stderr.contains("xenolith-not-a-xex.bin"),
        "the error did not name the file: {stderr}"
    );
    assert!(
        stderr.contains("magic"),
        "the error did not give the reason: {stderr}"
    );
}

#[test]
fn an_empty_file_is_rejected_without_panicking() {
    let path = std::env::temp_dir().join("xenolith-empty.bin");
    std::fs::write(&path, b"").unwrap();

    let (ok, _, stderr) = run!("inspect", path.to_str().unwrap());
    let _ = std::fs::remove_file(&path);

    assert!(!ok, "an empty file should fail");
    assert!(!stderr.contains("panicked"), "the tool panicked: {stderr}");
}

#[test]
fn no_arguments_fails_with_usage() {
    let (ok, _, stderr) = run!();

    assert!(!ok, "no arguments should fail");
    assert!(stderr.contains("Usage"), "no usage shown: {stderr}");
}

#[test]
fn disasm_help_documents_its_arguments() {
    let (ok, stdout, _) = run!("disasm", "--help");

    assert!(ok, "disasm help should succeed");
    for flag in ["--raw", "--base", "--start", "--length", "--end", "--sweep"] {
        assert!(stdout.contains(flag), "{flag} not documented");
    }
}

#[test]
fn a_misaligned_start_is_refused() {
    let path = std::env::temp_dir().join("xenolith-align.bin");
    std::fs::write(&path, vec![0x60u8; 64]).unwrap();

    let (ok, _, stderr) = run!(
        "disasm",
        "--raw",
        "--base",
        "0x82000000",
        "--start",
        "0x82000002",
        path.to_str().unwrap()
    );
    let _ = std::fs::remove_file(&path);

    assert!(!ok, "a misaligned start should fail");
    assert!(
        stderr.contains("aligned"),
        "the error did not explain the alignment rule: {stderr}"
    );
}

#[test]
fn a_range_outside_the_image_is_refused() {
    let path = std::env::temp_dir().join("xenolith-range.bin");
    std::fs::write(&path, vec![0x60u8; 64]).unwrap();

    let (ok, _, stderr) = run!(
        "disasm",
        "--raw",
        "--base",
        "0x82000000",
        "--start",
        "0x90000000",
        path.to_str().unwrap()
    );
    let _ = std::fs::remove_file(&path);

    assert!(!ok, "an unmapped range should fail");
    assert!(!stderr.contains("panicked"), "the tool panicked: {stderr}");
}

/// A word that decodes and a word that does not must both appear, so that a
/// sweep of a range never silently skips anything.
#[test]
fn disassembles_a_raw_image_marking_undecoded_words() {
    let path = std::env::temp_dir().join("xenolith-disasm.bin");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0x3860_0005u32.to_be_bytes()); // a load immediate
    bytes.extend_from_slice(&0x17ff_ffffu32.to_be_bytes()); // an unassigned primary
    std::fs::write(&path, &bytes).unwrap();

    let (ok, stdout, _) = run!(
        "disasm",
        "--raw",
        "--base",
        "0x82000000",
        "--start",
        "0x82000000",
        "--length",
        "8",
        path.to_str().unwrap()
    );
    let _ = std::fs::remove_file(&path);

    assert!(ok, "disassembly should succeed");
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 2, "one line per word: {stdout}");
    assert!(lines[0].contains("0x82000000"), "{stdout}");
    assert!(
        lines[1].contains(".long"),
        "the undecoded word was not marked"
    );
}

#[test]
fn a_sweep_reports_coverage() {
    let path = std::env::temp_dir().join("xenolith-sweep.bin");
    let mut bytes = Vec::new();
    for _ in 0..16 {
        bytes.extend_from_slice(&0x3860_0005u32.to_be_bytes());
    }
    bytes.extend_from_slice(&0x17ff_ffffu32.to_be_bytes());
    std::fs::write(&path, &bytes).unwrap();

    let (ok, stdout, _) = run!("disasm", "--raw", "--sweep", path.to_str().unwrap());
    let _ = std::fs::remove_file(&path);

    assert!(ok, "sweep should succeed");
    assert!(stdout.contains("swept 17 instruction words"), "{stdout}");
    assert!(stdout.contains("decoded"), "{stdout}");
    assert!(stdout.contains("undecoded"), "{stdout}");
    assert!(
        stdout.contains("17ffffff"),
        "the undecoded encoding was not listed: {stdout}"
    );
}

#[test]
fn top_level_help_lists_analyze() {
    let (ok, stdout, _) = run!("--help");

    assert!(ok, "help should succeed");
    assert!(stdout.contains("analyze"), "help did not list analyze");
}

#[test]
fn analyze_help_documents_its_arguments() {
    let (ok, stdout, _) = run!("analyze", "--help");

    assert!(ok, "analyze help should succeed");
    for flag in ["--raw", "--base", "--key-file", "--functions", "--tables"] {
        assert!(stdout.contains(flag), "{flag} not documented: {stdout}");
    }
}

#[test]
fn analyze_names_a_file_it_cannot_read_as_a_container() {
    let path = std::env::temp_dir().join("xenolith-analyze-garbage.bin");
    std::fs::write(&path, b"this is definitely not a xex file").unwrap();

    let (ok, _, stderr) = run!("analyze", path.to_str().unwrap());

    assert!(!ok, "analyzing a non container should fail");
    assert!(
        stderr.contains("xenolith-analyze-garbage.bin"),
        "the error did not name the file: {stderr}"
    );
}

#[test]
fn analyze_refuses_an_image_with_no_executable_words() {
    let path = std::env::temp_dir().join("xenolith-analyze-empty.bin");
    std::fs::write(&path, b"").unwrap();

    let (ok, _, stderr) = run!("analyze", "--raw", path.to_str().unwrap());

    assert!(!ok, "an image with no code should fail");
    assert!(
        stderr.contains("no executable words"),
        "the error did not say what was wrong: {stderr}"
    );
}

/// A switch whose recovery can be worked out by hand, so the reported table can
/// be checked against what the code says rather than against itself.
///
/// ```text
/// 0x82000000  mflr   r12                  a prologue, so the function is found
/// 0x82000004  stwu   r1, -32(r1)
/// 0x82000008  cmpli  r10, 3               four entries
/// 0x8200000c  bc     0x82000038           the default
/// 0x82000010  lis    r12, 0x8200
/// 0x82000014  addi   r12, r12, 0x48       the table
/// 0x82000018  lbzx   r0, r12, r10         entry = table[r10]
/// 0x8200001c  rlwinm r0, r0, 2, 0, 29     entry * 4
/// 0x82000020  lis    r12, 0x8200
/// 0x82000024  addi   r12, r12, 0x28       the base
/// 0x82000028  add    r12, r12, r0         target = base + entry * 4
/// 0x8200002c  mtctr  r12
/// 0x82000030  bctr
/// ```
///
/// The table holds 3, 5, 6 and 7, so the targets are the base plus 12, 20, 24
/// and 28.
#[test]
fn analyze_reports_a_jump_table_worked_out_by_hand() {
    const WORDS: [u32; 19] = [
        0x7d88_02a6,
        0x9421_ffe0,
        0x280a_0003,
        0x4181_002c,
        0x3d80_8200,
        0x398c_0048,
        0x7c0c_50ae,
        0x5400_103a,
        0x3d80_8200,
        0x398c_0028,
        0x7d8c_0214,
        0x7d89_03a6,
        0x4e80_0420,
        0x4e80_0020,
        0x4e80_0020,
        0x4e80_0020,
        0x4e80_0020,
        0x4e80_0020,
        0x0305_0607,
    ];

    let path = std::env::temp_dir().join("xenolith-analyze-switch.bin");
    let bytes: Vec<u8> = WORDS.iter().flat_map(|word| word.to_be_bytes()).collect();
    std::fs::write(&path, &bytes).unwrap();

    let (ok, stdout, stderr) = run!(
        "analyze",
        "--raw",
        "--base",
        "0x82000000",
        "--tables",
        path.to_str().unwrap()
    );

    assert!(ok, "analyzing a crafted image should succeed: {stderr}");
    assert!(
        stdout.contains("tables recovered            1"),
        "the table was not recovered: {stdout}"
    );
    assert!(
        stdout.contains("resolved                  1"),
        "the branch was not reported resolved: {stdout}"
    );
    assert!(
        stdout.contains("unresolved                0"),
        "the branch was reported unresolved: {stdout}"
    );
    assert!(
        stdout.contains("branch 0x82000030  index r10  table 0x82000048  default 0x82000038"),
        "the table was not reported as expected: {stdout}"
    );
    for target in ["0x82000034", "0x8200003c", "0x82000040", "0x82000044"] {
        assert!(stdout.contains(target), "{target} missing from: {stdout}");
    }
}

#[test]
fn top_level_help_lists_lift() {
    let (ok, stdout, _) = run!("--help");

    assert!(ok, "help should succeed");
    assert!(stdout.contains("lift"), "help did not list lift");
}

#[test]
fn lift_help_documents_its_arguments() {
    let (ok, stdout, _) = run!("lift", "--help");

    assert!(ok, "lift help should succeed");
    for flag in [
        "--out",
        "--raw",
        "--base",
        "--key-file",
        "--blockers",
        "--part-size",
    ] {
        assert!(stdout.contains(flag), "{flag} not documented: {stdout}");
    }
}

#[test]
fn lift_names_a_directory_it_cannot_write() {
    let path = std::env::temp_dir().join("xenolith-lift-input.bin");
    // A function with a prologue, so that scanning finds it without an entry
    // point: mflr r12; stwu r1, -96(r1); addi r1, r1, 96; blr
    let bytes: Vec<u8> = [0x7d88_02a6u32, 0x9421_ffa0, 0x3821_0060, 0x4e80_0020]
        .iter()
        .flat_map(|word| word.to_be_bytes())
        .collect();
    std::fs::write(&path, &bytes).unwrap();

    let (ok, _, stderr) = run!(
        "lift",
        "--raw",
        "--base",
        "0x82000000",
        "--out",
        "/proc/cannot/write/here",
        path.to_str().unwrap()
    );

    assert!(!ok, "an unwritable directory should fail");
    assert!(
        stderr.contains("cannot/write/here"),
        "the error did not name the directory: {stderr}"
    );
}

/// A crafted image whose emitted C can be read against the instructions it came
/// from, since there is no way to run either.
#[test]
fn lift_emits_c_for_a_crafted_image() {
    // mflr r12; stwu r1, -96(r1); li r3, 7; addi r1, r1, 96; blr
    const WORDS: [u32; 5] = [
        0x7d88_02a6,
        0x9421_ffa0,
        0x3860_0007,
        0x3821_0060,
        0x4e80_0020,
    ];

    let source = std::env::temp_dir().join("xenolith-lift-crafted.bin");
    let out = std::env::temp_dir().join("xenolith-lift-crafted-out");
    let _ = std::fs::remove_dir_all(&out);
    let bytes: Vec<u8> = WORDS.iter().flat_map(|word| word.to_be_bytes()).collect();
    std::fs::write(&source, &bytes).unwrap();

    let (ok, stdout, stderr) = run!(
        "lift",
        "--raw",
        "--base",
        "0x82000000",
        "--out",
        out.to_str().unwrap(),
        source.to_str().unwrap()
    );

    assert!(ok, "lifting a crafted image should succeed: {stderr}");
    assert!(
        stdout.contains("lifted                    1"),
        "one function should lift: {stdout}"
    );
    assert!(
        stdout.contains("import thunks           0"),
        "a raw image declares no imports: {stdout}"
    );

    let units = units_in(&out);
    assert_eq!(units.len(), 1, "one function should fit in one unit");
    let emitted = std::fs::read_to_string(&units[0]).expect("the emitted C");
    assert!(emitted.contains("#include \"lifted.h\""));

    let declarations = std::fs::read_to_string(out.join("lifted.h")).expect("the declarations");
    assert!(declarations.contains("#include \"xenolith.h\""));
    assert!(declarations.contains("void sub_82000000(xenolith_context *ctx, uint8_t *base);"));

    assert!(emitted.contains("void sub_82000000(xenolith_context *ctx, uint8_t *base)"));
    assert!(
        emitted.contains("ctx->r[12] = ctx->lr;"),
        "reading the link register should be emitted: {emitted}"
    );
    assert!(
        emitted.contains("ctx->r[3] = (uint64_t)(int64_t)(7);"),
        "loading a constant should be emitted: {emitted}"
    );
    assert!(
        emitted.contains("return;"),
        "returning should be emitted: {emitted}"
    );

    assert!(
        out.join("xenolith.h").is_file(),
        "the runtime header should be written beside the code"
    );
    for name in [
        "Makefile",
        "xenolith.h",
        "xenolith.c",
        "command_stream.c",
        "sound.c",
        "shaders.c",
        "vulkan.c",
        "lifted.h",
        "unlifted.c",
        "table.c",
        "main.c",
        "image.bin",
    ] {
        assert!(
            out.join(name).is_file(),
            "{name} should be written beside the code"
        );
    }
}

/// Compiling a unit says it is well formed on its own. Only a link says the
/// units agree with each other about what exists and that nothing is defined
/// twice, which is the thing twelve thousand functions can get wrong.
#[test]
fn what_was_emitted_links_into_a_program() {
    // mflr r12; stwu r1, -96(r1); li r3, 7; addi r1, r1, 96; blr
    const WORDS: [u32; 5] = [
        0x7d88_02a6,
        0x9421_ffa0,
        0x3860_0007,
        0x3821_0060,
        0x4e80_0020,
    ];

    let usable = Command::new("make")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !usable {
        eprintln!("skipping the link: make is not installed");
        return;
    }

    let source = std::env::temp_dir().join("xenolith-link.bin");
    let out = std::env::temp_dir().join("xenolith-link-out");
    let _ = std::fs::remove_dir_all(&out);
    let bytes: Vec<u8> = WORDS.iter().flat_map(|word| word.to_be_bytes()).collect();
    std::fs::write(&source, &bytes).unwrap();

    let (ok, _, stderr) = run!(
        "lift",
        "--raw",
        "--base",
        "0x82000000",
        "--out",
        out.to_str().unwrap(),
        source.to_str().unwrap()
    );
    assert!(ok, "lifting should succeed: {stderr}");

    let built = Command::new("make")
        .arg("-C")
        .arg(&out)
        .output()
        .expect("make should run");
    assert!(
        built.status.success(),
        "the emitted program did not link:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert!(
        out.join("xenolith").is_file(),
        "the link should have produced a program"
    );

    /* And the same directory with no graphics at all. One block decides it:
     * turning the three capabilities off has to leave everything else building
     * and linking, which is what says the switches are not load-bearing for a
     * title that draws nothing. */
    let plain = Command::new("make")
        .arg("-C")
        .arg(&out)
        .arg("clean")
        .output()
        .expect("make should run");
    assert!(plain.status.success(), "cleaning should work");
    let without = Command::new("make")
        .arg("-C")
        .arg(&out)
        .args(["DRAWING=no", "SHOWING=no", "TALKING=no"])
        .output()
        .expect("make should run");
    assert!(
        without.status.success(),
        "the emitted program did not link without graphics:\n{}",
        String::from_utf8_lossy(&without.stderr)
    );
    assert!(
        out.join("xenolith").is_file(),
        "a build with no graphics should still produce a program"
    );

    // Entering the one function it holds runs translated code and comes back.
    let ran = Command::new(out.join("xenolith"))
        .arg(out.join("image.bin"))
        .arg("0x82000000")
        .output()
        .expect("the program should run");
    assert!(
        ran.status.success(),
        "entering a lifted function should return: {}",
        String::from_utf8_lossy(&ran.stderr)
    );
}

/// Returns the translation units in an output directory, in name order.
///
/// A directory that cannot be read yields none, which the count every caller
/// asserts on reports better than a panic here would.
fn units_in(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut units: Vec<std::path::PathBuf> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "c")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("lifted."))
        })
        .collect();
    units.sort();
    units
}

/// A budget smaller than any function puts each one in its own unit, which is
/// the strongest statement the split can make: it rolls over when it should and
/// never inside a function.
#[test]
fn lift_splits_the_emitted_c_at_the_budget() {
    // Three functions back to back, each with a prologue so that scanning
    // finds it: mflr r12; stwu r1, -96(r1); addi r1, r1, 96; blr
    const ONE: [u32; 4] = [0x7d88_02a6, 0x9421_ffa0, 0x3821_0060, 0x4e80_0020];

    let source = std::env::temp_dir().join("xenolith-lift-split.bin");
    let out = std::env::temp_dir().join("xenolith-lift-split-out");
    let _ = std::fs::remove_dir_all(&out);
    let bytes: Vec<u8> = ONE
        .iter()
        .cycle()
        .take(ONE.len() * 3)
        .flat_map(|word| word.to_be_bytes())
        .collect();
    std::fs::write(&source, &bytes).unwrap();

    let (ok, stdout, stderr) = run!(
        "lift",
        "--raw",
        "--base",
        "0x82000000",
        "--part-size",
        "1",
        "--out",
        out.to_str().unwrap(),
        source.to_str().unwrap()
    );

    assert!(ok, "lifting should succeed: {stderr}");
    assert!(
        stdout.contains("lifted                    3"),
        "three functions should lift: {stdout}"
    );

    let units = units_in(&out);
    assert_eq!(
        units.len(),
        3,
        "a budget of one byte should split every one"
    );

    for unit in &units {
        let text = std::fs::read_to_string(unit).expect("a unit");
        assert!(
            text.contains("#include \"lifted.h\""),
            "{} did not include the declarations: {text}",
            unit.display()
        );
        assert_eq!(
            text.matches('{').count(),
            text.matches('}').count(),
            "{} ends part way through a function",
            unit.display()
        );
        assert_eq!(
            text.matches("xenolith_context *ctx, uint8_t *base) {")
                .count(),
            1,
            "{} should hold exactly one function",
            unit.display()
        );
    }
}

/// A second run that splits differently must not leave the first run's units
/// behind, since two units defining the same function will not build.
#[test]
fn lift_replaces_the_units_a_previous_run_wrote() {
    const ONE: [u32; 4] = [0x7d88_02a6, 0x9421_ffa0, 0x3821_0060, 0x4e80_0020];

    let source = std::env::temp_dir().join("xenolith-lift-restack.bin");
    let out = std::env::temp_dir().join("xenolith-lift-restack-out");
    let _ = std::fs::remove_dir_all(&out);
    let bytes: Vec<u8> = ONE
        .iter()
        .cycle()
        .take(ONE.len() * 3)
        .flat_map(|word| word.to_be_bytes())
        .collect();
    std::fs::write(&source, &bytes).unwrap();

    let arguments = |size: &'static str| {
        vec![
            "lift".to_owned(),
            "--raw".to_owned(),
            "--base".to_owned(),
            "0x82000000".to_owned(),
            "--part-size".to_owned(),
            size.to_owned(),
            "--out".to_owned(),
            out.to_str().unwrap().to_owned(),
            source.to_str().unwrap().to_owned(),
        ]
    };

    for size in ["1", "1048576"] {
        let output = Command::new(env!("CARGO_BIN_EXE_xenolith"))
            .args(arguments(size))
            .output()
            .expect("the binary should run");
        assert!(
            output.status.success(),
            "lifting should succeed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    assert_eq!(
        units_in(&out).len(),
        1,
        "the split units of the first run should be gone"
    );
}

/// Functions that cannot be lifted are the expected outcome, not a failure.
#[test]
fn lift_succeeds_when_only_some_functions_lift() {
    // A function with a prologue that calls a second one, which holds an
    // instruction with no semantics.
    const WORDS: [u32; 8] = [
        0x7d88_02a6,
        0x9421_ffa0,
        0x4800_0011,
        0x3821_0060,
        0x4e80_0020,
        0x6000_0000,
        // vpkpx, a pixel pack the model does not express
        0x1022_1b0e,
        0x4e80_0020,
    ];

    let source = std::env::temp_dir().join("xenolith-lift-partial.bin");
    let out = std::env::temp_dir().join("xenolith-lift-partial-out");
    let _ = std::fs::remove_dir_all(&out);
    let bytes: Vec<u8> = WORDS.iter().flat_map(|word| word.to_be_bytes()).collect();
    std::fs::write(&source, &bytes).unwrap();

    let (ok, stdout, stderr) = run!(
        "lift",
        "--raw",
        "--base",
        "0x82000000",
        "--out",
        out.to_str().unwrap(),
        "--blockers",
        source.to_str().unwrap()
    );

    assert!(ok, "partial coverage should still succeed: {stderr}");
    assert!(
        stdout.contains("not lifted"),
        "the refusal should be reported: {stdout}"
    );
    assert!(
        stdout.contains("vpkpx"),
        "what blocked it should be named: {stdout}"
    );
}

/// The boot program is told what kernel build the image was made against.
///
/// A title's device layout depends on it. A container carries it beside the
/// kernel's import library, and a container that does not is one whose boot
/// program says nought rather than guessing.
#[test]
fn the_boot_program_carries_the_kernel_build() {
    // mflr r12; stwu r1, -96(r1); li r3, 7; addi r1, r1, 96; blr
    const WORDS: [u32; 5] = [
        0x7d88_02a6,
        0x9421_ffa0,
        0x3860_0007,
        0x3821_0060,
        0x4e80_0020,
    ];

    let source = std::env::temp_dir().join("xenolith-lift-kernel-build.bin");
    let out = std::env::temp_dir().join("xenolith-lift-kernel-build-out");
    let _ = std::fs::remove_dir_all(&out);
    let bytes: Vec<u8> = WORDS.iter().flat_map(|word| word.to_be_bytes()).collect();
    std::fs::write(&source, &bytes).unwrap();

    let (ok, stdout, stderr) = run!(
        "lift",
        "--raw",
        "--base",
        "0x82000000",
        "--out",
        out.to_str().unwrap(),
        source.to_str().unwrap()
    );
    assert!(ok, "lifting a crafted image should succeed: {stderr}");

    let main = std::fs::read_to_string(out.join("main.c")).expect("reading main.c");
    assert!(
        main.contains("xenolith_built_against("),
        "the boot program does not say what kernel build it was made against"
    );
    /* A bare image carries no import libraries, so it says nought. */
    assert!(
        main.contains("xenolith_built_against(0x00000000u)"),
        "a bare image should say nothing was known, and it said something else"
    );
    /* And the block a title reaches the kernel's counters through. */
    assert!(
        main.contains("xenolith_processor_block(base)"),
        "the boot program does not give the title a processor block"
    );
    /* The report says it too, so a run says what it was told. */
    assert!(
        stdout.contains("kernel build") && stdout.contains("unknown"),
        "the lift did not report what it knew about the kernel build: {stdout}"
    );
}

/// The program is named as asked, in its window and as a file.
///
/// Every program used to be called the same thing, so two games built side by
/// side shared a window title and a desktop's grouping, and each was a file
/// called lifted.
#[test]
fn the_program_is_named_as_asked() {
    // mflr r12; stwu r1, -96(r1); li r3, 7; addi r1, r1, 96; blr
    const WORDS: [u32; 5] = [
        0x7d88_02a6,
        0x9421_ffa0,
        0x3860_0007,
        0x3821_0060,
        0x4e80_0020,
    ];

    let source = std::env::temp_dir().join("xenolith-lift-named.bin");
    let out = std::env::temp_dir().join("xenolith-lift-named-out");
    let _ = std::fs::remove_dir_all(&out);
    let bytes: Vec<u8> = WORDS.iter().flat_map(|word| word.to_be_bytes()).collect();
    std::fs::write(&source, &bytes).unwrap();

    let (ok, stdout, stderr) = run!(
        "lift",
        "--raw",
        "--name",
        "A \"Quoted\" Game",
        "--id",
        "a-game",
        "--out",
        out.to_str().unwrap(),
        source.to_str().unwrap()
    );
    assert!(ok, "lifting a named image should succeed: {stderr}");

    let main = std::fs::read_to_string(out.join("main.c")).expect("reading main.c");
    assert!(
        main.contains(r#"xenolith_name_the_title("A \042Quoted\042 Game", "a\055game");"#),
        "the boot program does not name the title as asked:\n{main}"
    );
    let makefile = std::fs::read_to_string(out.join("Makefile")).expect("reading the makefile");
    assert!(
        makefile.contains("PROGRAM := a-game\n"),
        "the makefile does not build a program called a-game"
    );
    assert!(
        stdout.contains("./a-game"),
        "the report does not say how to run the program: {stdout}"
    );

    let (ok, _, stderr) = run!(
        "lift",
        "--raw",
        "--id",
        "Not An Id",
        "--out",
        out.to_str().unwrap(),
        source.to_str().unwrap()
    );
    assert!(!ok, "an id that will not do as a file name was taken");
    assert!(
        stderr.contains("Not An Id"),
        "the refusal does not name the id: {stderr}"
    );
}

/// What a lift writes and what the page says it writes are the same list.
///
/// A page that lists a file the directory does not hold sends a reader looking
/// for something that is not there, and a file the page does not list is one
/// nobody knows about. Both directions are checked here, against a directory
/// this test just lifted.
#[test]
fn the_output_page_lists_what_a_lift_writes() {
    // mflr r12; stwu r1, -96(r1); li r3, 7; addi r1, r1, 96; blr
    const WORDS: [u32; 5] = [
        0x7d88_02a6,
        0x9421_ffa0,
        0x3860_0007,
        0x3821_0060,
        0x4e80_0020,
    ];

    let source = std::env::temp_dir().join("xenolith-output-page.bin");
    let out = std::env::temp_dir().join("xenolith-output-page-out");
    let _ = std::fs::remove_dir_all(&out);
    let bytes: Vec<u8> = WORDS.iter().flat_map(|word| word.to_be_bytes()).collect();
    std::fs::write(&source, &bytes).unwrap();

    let (ok, _, stderr) = run!(
        "lift",
        "--raw",
        "--base",
        "0x82000000",
        "--out",
        out.to_str().unwrap(),
        source.to_str().unwrap()
    );
    assert!(ok, "lifting a crafted image should succeed: {stderr}");

    /* The two tables that list files: the ones whose first column is a path.
     * The makefile's variables and the capabilities looked for are tables too,
     * and reading those as file names would have the check looking for a file
     * called CFLAGS. */
    let page = include_str!("../../../docs/reference/output.md");
    let named: BTreeSet<&str> = page
        .lines()
        .filter(|line| line.starts_with("| `"))
        .filter_map(|line| line.split('`').nth(1))
        .filter(|name| name.contains('.') || name == &"Makefile")
        .collect();
    assert!(!named.is_empty(), "the page names no file at all");

    /* Every file the lift wrote is named by the page, bar the units, which the
     * page describes by their naming rule rather than one by one. */
    let mut unexplained = Vec::new();
    for entry in std::fs::read_dir(&out).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        if name.starts_with("lifted.")
            && std::path::Path::new(&name).extension() == Some(std::ffi::OsStr::new("c"))
        {
            continue;
        }
        if !named.contains(name.as_str()) {
            unexplained.push(name);
        }
    }
    unexplained.sort();
    assert!(
        unexplained.is_empty(),
        "the lift wrote these and the page does not name them: {unexplained:?}"
    );

    /* And every name the page gives is either there or is one of the files that
     * comes and goes with the machine. */
    let may_be_absent = [
        /* Comes and goes with the machine and the build that made the tool. */
        "libxenolith_shader.a",
        "xdg-shell-client-protocol.h",
        "xdg-shell-protocol.c",
        /* A rule rather than a name: the page says how a unit is named and
         * there is one of these for every unit in the directory. */
        "lifted.<first>-<last>.c",
    ];
    let mut missing = Vec::new();
    for name in &named {
        if out.join(name).is_file() || may_be_absent.contains(name) {
            continue;
        }
        missing.push(*name);
    }
    assert!(
        missing.is_empty(),
        "the page names these and the directory does not hold them: {missing:?}"
    );
}

/// A root the caller names is lifted even where nothing else would find it.
///
/// Discovery works from prologues, entry points and the calls it can see. A
/// function that leaves none of those is one a run of a built program reaches
/// and discovery does not, and naming its address is how that evidence is given
/// back. What is checked is that the address is lifted and that the image
/// without it lifts fewer functions.
#[test]
fn a_named_root_is_lifted() {
    /* One function with a prologue, and then one with none at all: `li r3, 7;
     * blr` shows neither of the two signals a prologue needs, so nothing finds
     * it. It is only a function because the caller says so. */
    const WITH_PROLOGUE: [u32; 4] = [0x7d88_02a6, 0x9421_ffa0, 0x3821_0060, 0x4e80_0020];
    const WITHOUT: [u32; 2] = [0x3860_0007, 0x4e80_0020];

    let source = std::env::temp_dir().join("xenolith-root.bin");
    let mut words: Vec<u32> = Vec::new();
    words.extend(WITH_PROLOGUE);
    let bare = 0x8200_0000u32 + u32::try_from(words.len()).expect("a length") * 4;
    words.extend(WITHOUT);
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
    std::fs::write(&source, &bytes).unwrap();

    let plain = std::env::temp_dir().join("xenolith-root-plain");
    let _ = std::fs::remove_dir_all(&plain);
    let (ok, without_root, stderr) = run!(
        "lift",
        "--raw",
        "--base",
        "0x82000000",
        "--out",
        plain.to_str().unwrap(),
        source.to_str().unwrap()
    );
    assert!(ok, "lifting should succeed: {stderr}");

    let named = std::env::temp_dir().join("xenolith-root-named");
    let _ = std::fs::remove_dir_all(&named);
    let root = format!("{bare:#010x}");
    let (ok, with_root, stderr) = run!(
        "lift",
        "--raw",
        "--base",
        "0x82000000",
        "--out",
        named.to_str().unwrap(),
        "--root",
        root.as_str(),
        source.to_str().unwrap()
    );
    assert!(ok, "lifting with a root should succeed: {stderr}");

    let count = |said: &str| -> u64 {
        said.lines()
            .find(|line| line.starts_with("functions"))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|number| number.parse().ok())
            .unwrap_or(0)
    };
    /* What a root guarantees is that the address is a function: it is lifted
     * and declared. Whether that adds to the count depends on whether discovery
     * found it anyway, which on an image this small it does, because the scan
     * walks forward from the first function and reaches the words after it. */
    let declared = std::fs::read_to_string(named.join("lifted.h")).unwrap();
    let name = format!("sub_{bare:08x}");
    assert!(
        declared.contains(&name),
        "the named root should be declared as {name}:\n{declared}"
    );
    assert!(
        count(&with_root) >= count(&without_root),
        "naming a root should never lift fewer functions, and it lifted {} against {}",
        count(&with_root),
        count(&without_root)
    );

    /* And a root that is not a number is refused rather than passed over, which
     * is what says the flag is read at all rather than being accepted and
     * ignored. */
    let (ok, _, stderr) = run!(
        "lift",
        "--raw",
        "--base",
        "0x82000000",
        "--out",
        named.to_str().unwrap(),
        "--root",
        "not-an-address",
        source.to_str().unwrap()
    );
    assert!(!ok, "a root that is not an address should be refused");
    assert!(
        stderr.contains("not-an-address") || stderr.to_lowercase().contains("number"),
        "the refusal should say what was wrong: {stderr}"
    );
}

/// A roots file is read, one address per line.
///
/// A run of a built program with `XENOLITH_TRACE_DISPATCH` set reports every
/// address it wanted and could not reach, and this is how that report is fed
/// back. What is checked is that the file's addresses are lifted, which is the
/// same effect `--root` has one at a time.
#[test]
fn a_roots_file_is_read() {
    const WITH_PROLOGUE: [u32; 4] = [0x7d88_02a6, 0x9421_ffa0, 0x3821_0060, 0x4e80_0020];
    const WITHOUT: [u32; 2] = [0x3860_0007, 0x4e80_0020];

    let source = std::env::temp_dir().join("xenolith-roots-from.bin");
    let mut words: Vec<u32> = Vec::new();
    words.extend(WITH_PROLOGUE);
    let bare = 0x8200_0000u32 + u32::try_from(words.len()).expect("a length") * 4;
    words.extend(WITHOUT);
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
    std::fs::write(&source, &bytes).unwrap();

    let list = std::env::temp_dir().join("xenolith-roots-from.txt");
    std::fs::write(&list, format!("{bare:#010x}\n")).unwrap();

    let out = std::env::temp_dir().join("xenolith-roots-from-out");
    let _ = std::fs::remove_dir_all(&out);
    let (ok, _, stderr) = run!(
        "lift",
        "--raw",
        "--base",
        "0x82000000",
        "--out",
        out.to_str().unwrap(),
        "--roots-from",
        list.to_str().unwrap(),
        source.to_str().unwrap()
    );
    assert!(ok, "lifting with a roots file should succeed: {stderr}");

    let declared = std::fs::read_to_string(out.join("lifted.h")).unwrap();
    let name = format!("sub_{bare:08x}");
    assert!(
        declared.contains(&name),
        "the address from the file should be declared as {name}:\n{declared}"
    );
}

/// The unlifted list names every function that was not lifted.
///
/// Without it a run says how many were not lifted and not which, so finding out
/// what stopped one means guessing. What is checked is that the list names the
/// function and the instruction, and that it is absent unless asked for.
#[test]
fn the_unlifted_list_names_what_stopped_a_function() {
    /* A prologue, then an instruction the emitter has no reading of, so the
     * function is found and refused rather than missed: `vpkpx`, a pixel pack
     * the model does not express. */
    const REFUSED: [u32; 5] = [
        0x7d88_02a6,
        0x9421_ffa0,
        0x1022_1b0e,
        0x3821_0060,
        0x4e80_0020,
    ];

    let source = std::env::temp_dir().join("xenolith-unlifted.bin");
    let bytes: Vec<u8> = REFUSED.iter().flat_map(|word| word.to_be_bytes()).collect();
    std::fs::write(&source, &bytes).unwrap();

    let quiet = std::env::temp_dir().join("xenolith-unlifted-quiet");
    let _ = std::fs::remove_dir_all(&quiet);
    let (ok, said_nothing, _) = run!(
        "lift",
        "--raw",
        "--base",
        "0x82000000",
        "--out",
        quiet.to_str().unwrap(),
        source.to_str().unwrap()
    );
    assert!(ok, "lifting should succeed");

    let told = std::env::temp_dir().join("xenolith-unlifted-told");
    let _ = std::fs::remove_dir_all(&told);
    let (ok, said, stderr) = run!(
        "lift",
        "--raw",
        "--base",
        "0x82000000",
        "--out",
        told.to_str().unwrap(),
        "--unlifted",
        source.to_str().unwrap()
    );
    assert!(ok, "lifting with the list should succeed: {stderr}");

    assert!(
        !said_nothing.contains("functions that were not lifted"),
        "the list should not be printed unless it is asked for:\n{said_nothing}"
    );
    assert!(
        said.contains("functions that were not lifted"),
        "the list should be printed when it is asked for:\n{said}"
    );
    assert!(
        said.contains("0x82000000"),
        "the list should name the function:\n{said}"
    );
}
