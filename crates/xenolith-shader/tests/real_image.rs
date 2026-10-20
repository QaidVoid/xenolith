//! Finding shaders in a real title's image.
//!
//! No game data is committed. The image is supplied through the environment, so
//! these are ignored without it and fail naming the variable when they are
//! included and it is absent.
//!
//! What is checked is not how many shaders an image holds, which nobody knows.
//! It is that the ones a title was watched handing to the hardware are among
//! what the reading finds, at the word they start on rather than near it. A
//! shader found at the wrong word would be translated and then never matched to
//! the upload it belongs to.

use std::path::PathBuf;

/// The image to read, or a failure naming the variable that would have named it.
// A check that could not run must fail rather than pass, which is the point.
#[allow(clippy::panic)]
fn image() -> Vec<u32> {
    let path = std::env::var_os("XENOLITH_SHADER_IMAGE").map_or_else(
        || panic!("XENOLITH_SHADER_IMAGE is not set, so this check could not run"),
        PathBuf::from,
    );
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("{} could not be read: {error}", path.display()));

    bytes
        .chunks_exact(4)
        .filter_map(|four| four.try_into().ok())
        .map(u32::from_be_bytes)
        .collect()
}

/// The four a title was watched handing over, as the word each starts on
/// counted from the front of the image.
///
/// The addresses they were seen at are 0x82053be4, 0x82053fb0, 0x82054010 and
/// 0x82054070, and the image loads at 0x82000000.
const SEEN: [(usize, usize); 4] = [
    ((0x0005_3be4) / 4, 9),
    ((0x0005_3fb0) / 4, 24),
    ((0x0005_4010) / 4, 15),
    ((0x0005_4070) / 4, 27),
];

#[test]
#[ignore = "needs XENOLITH_SHADER_IMAGE"]
fn what_a_title_handed_over_is_among_what_is_found() {
    let words = image();

    let found = xenolith_shader::scan(&words);
    assert!(!found.is_empty(), "an image holds shaders");

    for (at, _) in SEEN {
        assert!(
            found.iter().any(|one| one.at == at),
            "a shader a title handed over was not found at the word it starts on"
        );
    }
}

#[test]
#[ignore = "needs XENOLITH_SHADER_IMAGE"]
fn each_one_seen_reads_as_a_shader_where_it_sits() {
    let words = image();

    for (at, uploaded) in SEEN {
        let rest = words.get(at..).unwrap_or_default();
        let length = xenolith_shader::reads_as_shader(rest);
        assert!(
            length.is_some(),
            "words a title handed over do not read as a shader"
        );

        /* What a title hands over is the shader and the padding after it, so
         * what is read may be shorter than what was uploaded but never longer:
         * reading past the upload would be reading somebody else's words. */
        if let Some(length) = length {
            assert!(
                length <= uploaded,
                "reading ran past what the title handed over"
            );
        }
    }
}
