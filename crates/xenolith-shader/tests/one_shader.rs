//! Checking one shader a real title hands the hardware while it is running.
//!
//! Everything else here is checked against shaders found by reading a title's
//! executable. This one is not in the executable at all: it was read out of a
//! running title's memory, having been loaded from the data beside it, and it
//! is the only shader that title was seen to hand over that reading the
//! executable does not find.
//!
//! It is worth pinning because it is the one that drove four separate pieces of
//! work. It refused as not a shader at all until stretches that run on a
//! predicate were read, then refused on a fetch until the packed vertex formats
//! were, and each of those refusals was the truth about a real gap rather than
//! anything wrong with the shader.

use std::io::Write as _;
use std::process::{Command, Stdio};

use xenolith_shader::{Kind, translate};

/// What a validator says about a module.
///
/// # Panics
///
/// Where no validator is installed, because the test that calls this is the one
/// that needs it and a test may not report a pass it did not earn.
#[allow(clippy::expect_used, clippy::panic)]
fn validate(words: &[u32]) -> Result<(), String> {
    let bytes: Vec<u8> = words.iter().copied().flat_map(u32::to_le_bytes).collect();

    let mut running = Command::new("spirv-val")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spirv-val could not be run, so this check could not run");

    running
        .stdin
        .take()
        .expect("spirv-val took no input")
        .write_all(&bytes)
        .expect("writing to spirv-val");
    let done = running.wait_with_output().expect("waiting for spirv-val");

    if done.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&done.stderr)
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned())
    }
}

/// The words the title handed over, as they sat in its memory.
const WORDS: &str = "\
    70956003 60091000 56000000 00000000 400fc200 12000000 00000000 2013c400 \
    22000000 3df80000 00263647 0000000d 05f81000 4026321a 0000040d 05f82000 \
    0006060a 00000c0d 70000000 0000001b e2000000 c8010000 181bc6b1 8b00ffff \
    2c100000 1800006c e2000000 c8040003 186c1b6c 8b00ffff c0130003 1ac5b0c6 \
    e0000103 c4280000 1ab16cc6 e1030003 c8070000 18bcb000 c1030000 c8030000 \
    18b01a00 c0000000 c8060000 1861bc00 c0000100 c801803e 00c5b01b 91002828 \
    c802803e 00c5b01b 91002929 c804803e 00c5b01b 91002a2a c808803e 00c5b01b \
    91002b2b c8038000 001a1a00 c2010100 c80f8001 00000000 c2020200 4e4a0000 \
    649bd3d7 177391ea";

/// The words the title handed over, read as words.
// A word that is not hexadecimal is a failure of the fixture rather than
// something to carry on from, so this panics where a reader would want to know.
#[allow(clippy::expect_used)]
fn words() -> Vec<u32> {
    WORDS
        .split_whitespace()
        .map(|word| u32::from_str_radix(word, 16).expect("a hexadecimal word"))
        .collect()
}

#[test]
fn the_one_a_title_hands_over_while_running_is_said_in_a_host_s_terms() {
    let words = words();
    assert_eq!(words.len(), 66);

    let said = translate(Kind::Vertex, &words)
        .unwrap_or_else(|why| panic!("the shader a title hands over was refused: {why:?}"));
    assert!(
        !said.words.is_empty(),
        "a shader that translates says something"
    );
    assert_eq!(said.words[0], 0x0723_0203, "a module begins as one does");
}

/// The same shader, checked by a validator where one is installed.
///
/// Ignored rather than passed where `spirv-val` is absent, because a test that
/// reports a pass it did not earn is worse than one that reports it did not run.
/// Nothing in this tree runs it with `--include-ignored`, so on a machine without
/// the tool it stays ignored and the run says so.
#[test]
#[ignore = "needs spirv-val"]
#[allow(clippy::expect_used)]
fn the_validator_accepts_the_one_a_title_hands_over() {
    let said =
        translate(Kind::Vertex, &words()).expect("the shader a title hands over should translate");
    let checked = validate(&said.words);
    if let Err(complaint) = checked {
        panic!("the shader was translated and then turned down: {complaint}");
    }
}
