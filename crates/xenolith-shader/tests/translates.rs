//! Translating a shader a title actually handed the hardware.
//!
//! The words below came out of a retail title as it ran. What is checked is not
//! that the translation is right, which nothing here can tell: it is that what
//! comes out is a module a host driver would accept, which is the part that can
//! be checked without a screen to look at.
//!
//! Where the validator is not installed these are ignored rather than passing,
//! and including one without it fails naming what is missing.

use std::io::Write;
use std::process::{Command, Stdio};

use xenolith_shader::{Kind, Refused, translate};

/// A pixel shader, nine words, as a title handed it over.
const PIXEL: [u32; 9] = [
    0x0000_0000,
    0x1001_c400,
    0x2200_0000,
    0xc80f_8000,
    0x0000_0000,
    0xc200_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
];

/// What the validator said, or a failure naming what is missing.
fn validate(words: &[u32]) -> Result<(), String> {
    let bytes: Vec<u8> = words.iter().copied().flat_map(u32::to_le_bytes).collect();

    let mut running = Command::new("spirv-val")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spirv-val could not be run: {error}"))?;

    running
        .stdin
        .take()
        .ok_or_else(|| "spirv-val took no input".to_owned())?
        .write_all(&bytes)
        .map_err(|error| format!("writing to spirv-val: {error}"))?;
    let done = running
        .wait_with_output()
        .map_err(|error| format!("waiting for spirv-val: {error}"))?;

    if done.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&done.stderr).into_owned())
    }
}

#[test]
#[ignore = "needs spirv-val"]
fn a_shader_a_title_handed_over_becomes_one_a_driver_takes() {
    let translated = match translate(Kind::Pixel, &PIXEL) {
        Ok(translated) => translated,
        Err(why) => panic!("a shader a title handed over was refused: {why:?}"),
    };

    if let Err(complaint) = validate(&translated.words) {
        panic!("the validator refused a translation: {complaint}");
    }
}

#[test]
fn words_that_are_not_a_shader_are_refused_rather_than_translated() {
    assert!(matches!(
        translate(Kind::Pixel, &[0; 12]),
        Err(Refused::NotAShader)
    ));
}
