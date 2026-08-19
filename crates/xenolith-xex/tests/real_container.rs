//! Parsing checks against a real XEX supplied by whoever runs the suite.
//!
//! Synthetic fixtures prove the parser handles the layouts we thought to write.
//! Only a shipped file proves the layout we inferred is the one retail titles
//! actually use. No game data is committed here, so the tests read a path from
//! `XENOLITH_TEST_XEX` and skip when it is unset.

use std::path::PathBuf;

use xenolith_xex::{Container, Format, OptionalHeaderValue};

/// Returns the path named by `XENOLITH_TEST_XEX`, or `None` when it is unset.
fn test_xex_path() -> Option<PathBuf> {
    std::env::var_os("XENOLITH_TEST_XEX").map(PathBuf::from)
}

/// Skips the enclosing test when no real container has been supplied.
macro_rules! container_bytes {
    () => {
        match test_xex_path() {
            Some(path) => {
                std::fs::read(&path).expect("reading the container named by the environment")
            }
            None => {
                eprintln!("skipping: XENOLITH_TEST_XEX is not set");
                return;
            }
        }
    };
}

#[test]
fn parses_a_real_container() {
    let bytes = container_bytes!();
    let container = Container::parse(&bytes).expect("real container should parse");

    assert_eq!(container.format(), Format::Xex2);
    assert!(!container.optional_headers().is_empty());

    let image_offset = usize::try_from(container.image_offset()).unwrap();
    let security_info_offset = usize::try_from(container.security_info_offset()).unwrap();

    assert!(image_offset < bytes.len(), "image offset outside the file");
    assert!(
        security_info_offset < bytes.len(),
        "security info offset outside the file"
    );
}

#[test]
fn every_optional_header_resolves_within_the_file() {
    let bytes = container_bytes!();
    let container = Container::parse(&bytes).expect("real container should parse");

    for header in container.optional_headers() {
        if let OptionalHeaderValue::Data(data) = header.value {
            assert!(
                !data.is_empty(),
                "entry {:#010x} resolved empty",
                header.key.0
            );
            assert!(
                data.len() <= bytes.len(),
                "entry {:#010x} resolved past the file",
                header.key.0
            );
        }
    }
}

/// The low byte of a key is read as a count of 32-bit words. Retail files are
/// the only place that reading can be confirmed, and a wrong reading here would
/// silently mis-size every structure the loader goes on to parse.
#[test]
fn fixed_size_entries_match_their_declared_word_count() {
    let bytes = container_bytes!();
    let container = Container::parse(&bytes).expect("real container should parse");

    let mut checked = 0;
    for header in container.optional_headers() {
        let words = header.key.0 & 0xff;
        if !(0x02..0xff).contains(&words) {
            continue;
        }

        let data = header
            .value
            .data()
            .expect("a fixed size entry should resolve to data");

        assert_eq!(
            data.len(),
            usize::try_from(words).unwrap() * 4,
            "entry {:#010x} resolved {} bytes for {words} words",
            header.key.0,
            data.len()
        );
        checked += 1;
    }

    assert!(checked > 0, "the container carried no fixed size entries");
}
