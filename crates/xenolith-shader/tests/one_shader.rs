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

/// What a validator says about a module, or nothing where none is installed.
fn validate(words: &[u32]) -> Option<Result<(), String>> {
    let bytes: Vec<u8> = words.iter().copied().flat_map(u32::to_le_bytes).collect();

    let mut running = Command::new("spirv-val")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    running.stdin.take()?.write_all(&bytes).ok()?;
    let done = running.wait_with_output().ok()?;

    Some(if done.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&done.stderr)
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned())
    })
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

#[test]
fn the_one_a_title_hands_over_while_running_is_said_in_a_host_s_terms() {
    let words: Vec<u32> = WORDS
        .split_whitespace()
        .map(|word| u32::from_str_radix(word, 16).expect("a hexadecimal word"))
        .collect();
    assert_eq!(words.len(), 66);

    let said = match translate(Kind::Vertex, &words) {
        Ok(said) => said,
        Err(why) => panic!("the shader a title hands over was refused: {why:?}"),
    };
    assert!(
        !said.words.is_empty(),
        "a shader that translates says something"
    );

    let Some(checked) = validate(&said.words) else {
        eprintln!("skipping the check: no validator is installed");
        return;
    };
    if let Err(complaint) = checked {
        panic!("the shader was translated and then turned down: {complaint}");
    }
}
