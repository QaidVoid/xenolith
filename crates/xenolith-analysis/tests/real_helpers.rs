//! Helper detection against a real title.
//!
//! Two separate recompilation projects record the save and restore helper
//! addresses for two different titles, worked out by hand by someone who was
//! not building this. Detection has to find those exact addresses without being
//! given any of them, which is the strongest check available anywhere in this
//! project: the expected values are independent, exact, and were derived by a
//! different method.
//!
//! No game data is committed here. The image and the expected addresses are
//! supplied through the environment, and the test skips when they are absent.

use std::path::PathBuf;

use xenolith_analysis::{HelperDirection, HelperKind, detect};
use xenolith_xex::{Image, PageKind, Section};

/// Returns the image path and base address, if both were supplied.
fn supplied_source() -> Option<(PathBuf, u32)> {
    let path = PathBuf::from(std::env::var_os("XENOLITH_ANALYSIS_IMAGE")?);
    let base = std::env::var("XENOLITH_ANALYSIS_BASE")
        .ok()
        .and_then(|text| u32::from_str_radix(text.trim_start_matches("0x"), 16).ok())?;
    Some((path, base))
}

/// Builds an image treating the whole file as one executable span.
fn image_of(bytes: Vec<u8>, base: u32) -> Image {
    let size = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    let sections = vec![Section {
        start: base,
        size,
        kind: PageKind::Code,
    }];
    Image::new(base, bytes, sections)
}

/// Loads the supplied image, or skips the enclosing test.
macro_rules! supplied_image {
    () => {
        match supplied_source() {
            Some((path, base)) => {
                let bytes = std::fs::read(&path).expect("reading the analysis image");
                image_of(bytes, base)
            }
            None => {
                eprintln!("skipping: XENOLITH_ANALYSIS_IMAGE is not set");
                return;
            }
        }
    };
}

/// Reads the expected helper addresses, if they were supplied.
fn expected_addresses() -> Option<Vec<u32>> {
    let text = std::env::var("XENOLITH_ANALYSIS_HELPERS").ok()?;
    Some(
        text.split(',')
            .filter_map(|part| u32::from_str_radix(part.trim().trim_start_matches("0x"), 16).ok())
            .collect(),
    )
}

#[test]
fn detection_finds_the_hand_recorded_helper_addresses() {
    let image = supplied_image!();
    let helpers = detect(&image);

    eprintln!("detected {} helpers", helpers.all().len());
    for helper in helpers.all() {
        eprintln!(
            "  {:#010x}..{:#010x}  {:<16} {:<8} registers {}..{}",
            helper.start,
            helper.end,
            helper.kind.name(),
            helper.direction.name(),
            helper.first_register,
            helper.last_register
        );
    }
    for (kind, direction) in helpers.missing() {
        eprintln!("  missing: {} {}", kind.name(), direction.name());
    }

    let Some(expected) = expected_addresses() else {
        eprintln!("skipping the comparison: XENOLITH_ANALYSIS_HELPERS is not set");
        return;
    };

    let found: Vec<u32> = helpers.all().iter().map(|helper| helper.start).collect();
    let mut absent = Vec::new();
    for address in &expected {
        if !found.contains(address) {
            absent.push(*address);
        }
    }

    assert!(
        absent.is_empty(),
        "detection did not find these hand recorded addresses: {}",
        absent
            .iter()
            .map(|a| format!("{a:#010x}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

#[test]
fn every_helper_kind_and_direction_is_present() {
    let image = supplied_image!();
    let helpers = detect(&image);

    for kind in HelperKind::ALL {
        for direction in HelperDirection::ALL {
            let present = helpers
                .all()
                .iter()
                .any(|h| h.kind == kind && h.direction == direction);
            assert!(
                present,
                "no {} {} helper was detected",
                kind.name(),
                direction.name()
            );
        }
    }
}
