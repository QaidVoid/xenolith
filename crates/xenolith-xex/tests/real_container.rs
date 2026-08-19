//! Parsing checks against a real XEX supplied by whoever runs the suite.
//!
//! Synthetic fixtures prove the parser handles the layouts we thought to write.
//! Only a shipped file proves the layout we inferred is the one retail titles
//! actually use. No game data is committed here, so the tests read a path from
//! `XENOLITH_TEST_XEX` and skip when it is unset.

use std::path::PathBuf;

use xenolith_xex::{
    CompressionType, Container, EncryptionType, Error, Format, KeyMaterial, OptionalHeaderValue,
};

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

/// The optional header and the security info record the load address
/// independently. Disagreement means one of the two was read at the wrong
/// offset, which no synthetic fixture can catch because the builder writes both
/// from the same value.
#[test]
fn the_two_recorded_load_addresses_agree() {
    let bytes = container_bytes!();
    let container = Container::parse(&bytes).expect("real container should parse");

    if let Some(declared) = container.image_base_address() {
        assert_eq!(
            declared,
            container.security_info().load_address(),
            "optional header and security info disagree on the load address"
        );
    }
}

#[test]
fn the_descriptors_account_for_the_whole_image() {
    let bytes = container_bytes!();
    let container = Container::parse(&bytes).expect("real container should parse");
    let security = container.security_info();

    let pages = u64::from(security.total_pages());
    let image_size = u64::from(security.image_size());

    assert!(pages > 0, "no page descriptors");
    assert_eq!(
        image_size % pages,
        0,
        "image size {image_size:#x} does not divide evenly across {pages} pages"
    );
    assert_eq!(
        image_size / pages,
        0x1_0000,
        "expected 64 KiB pages, got {:#x}",
        image_size / pages
    );
}

#[test]
fn the_entry_point_falls_inside_the_image() {
    let bytes = container_bytes!();
    let container = Container::parse(&bytes).expect("real container should parse");
    let security = container.security_info();

    let Some(entry) = container.entry_point() else {
        return;
    };
    let base = u64::from(security.load_address());
    let end = base + u64::from(security.image_size());

    assert!(
        (base..end).contains(&u64::from(entry)),
        "entry point {entry:#x} outside image {base:#x}..{end:#x}"
    );
}

#[test]
fn the_import_library_count_matches_the_security_info() {
    let bytes = container_bytes!();
    let container = Container::parse(&bytes).expect("real container should parse");

    let declared = container.security_info().import_table_count();
    let parsed = u32::try_from(container.import_libraries().len()).unwrap();

    assert_eq!(declared, parsed, "declared and parsed import counts differ");

    for library in container.import_libraries() {
        assert!(
            !library.name.is_empty(),
            "import library with an empty name"
        );
    }
}

/// For a basic scheme image the stored blocks must account for exactly the
/// bytes between the image offset and the end of the file. This is the single
/// strongest check available without decrypting anything.
#[test]
fn basic_scheme_blocks_account_for_the_stored_image() {
    let bytes = container_bytes!();
    let container = Container::parse(&bytes).expect("real container should parse");

    let Some(info) = container.file_format_info() else {
        return;
    };
    if info.compression() != CompressionType::Basic {
        eprintln!("skipping: image is not basic scheme");
        return;
    }

    let stored: u64 = info
        .basic_blocks()
        .iter()
        .map(|block| u64::from(block.data_size))
        .sum();
    let available = u64::try_from(bytes.len()).unwrap() - u64::from(container.image_offset());

    assert_eq!(
        stored, available,
        "basic blocks describe {stored:#x} stored bytes, file holds {available:#x}"
    );
}

/// Decoding an encrypted title without key material must fail loudly rather
/// than handing back a buffer of noise.
#[test]
fn decoding_an_encrypted_title_without_a_key_is_refused() {
    let bytes = container_bytes!();
    let container = Container::parse(&bytes).expect("real container should parse");

    if container.encryption() != EncryptionType::Encrypted {
        eprintln!("skipping: title is not encrypted");
        return;
    }

    assert!(
        matches!(container.image(None), Err(Error::KeyMaterialRequired)),
        "an encrypted title decoded without key material"
    );
}

/// Compares the decoded image against a reference produced by an independent
/// implementation, named by `XENOLITH_TEST_IMAGE`.
///
/// This is the only check that covers decryption and reconstruction together
/// against ground truth rather than against our own expectations. It needs key
/// material, so it skips when the image cannot be decoded.
///
/// The reference is expected to cover only what the blocks describe. Our own
/// reconstruction zero fills out to the declared image size, because the page
/// descriptors cover that whole range and a section must not claim addresses
/// the byte buffer cannot serve.
#[test]
fn the_decoded_image_matches_an_independent_reference() {
    let bytes = container_bytes!();

    let Some(reference_path) = std::env::var_os("XENOLITH_TEST_IMAGE") else {
        eprintln!("skipping: XENOLITH_TEST_IMAGE is not set");
        return;
    };
    let reference = std::fs::read(&reference_path).expect("reading the reference image");

    let container = Container::parse(&bytes).expect("real container should parse");
    let key = std::env::var("XENOLITH_XEX_KEY")
        .ok()
        .map(|text| KeyMaterial::from_hex(&text).expect("parsing XENOLITH_XEX_KEY"));

    if container.encryption() == EncryptionType::Encrypted && key.is_none() {
        eprintln!("skipping: title is encrypted and XENOLITH_XEX_KEY is not set");
        return;
    }

    let image = container.image(key.as_ref()).expect("decoding the image");

    assert!(
        image.len() >= reference.len(),
        "decoded {} bytes, reference holds {}",
        image.len(),
        reference.len()
    );

    let overlap = reference.len();
    assert_eq!(
        &image[..overlap],
        &reference[..],
        "decoded image differs from the reference within the described range"
    );
    assert!(
        image[overlap..].iter().all(|byte| *byte == 0),
        "the zero filled tail is not zero"
    );
}
