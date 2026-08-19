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
use xenolith_xex::{Container, Image, KeyMaterial, PageKind, Section};

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
///
/// A container is preferred when one was given, because it describes itself.
macro_rules! supplied_image {
    () => {
        match std::env::var_os("XENOLITH_ANALYSIS_XEX") {
            Some(path) => {
                // A container carries its own section layout and entry point,
                // so neither has to be described by hand and neither can be
                // described wrongly. Decoding one needs key material, supplied
                // through the environment rather than embedded.
                let bytes = std::fs::read(&path).expect("reading the analysis container");
                let container = Container::parse(&bytes).expect("parsing the container");
                let key = std::env::var("XENOLITH_XEX_KEY")
                    .ok()
                    .map(|text| KeyMaterial::from_hex(text.trim()).expect("the supplied key"));
                container.load(key.as_ref()).expect("decoding the image")
            }
            None => match supplied_source() {
                Some((path, base)) => {
                    let bytes = std::fs::read(&path).expect("reading the analysis image");
                    image_of(bytes, base)
                }
                None => {
                    eprintln!("skipping: no analysis image was supplied");
                    return;
                }
            },
        }
    };
}

/// Reads the hand recorded helper addresses, if they were supplied.
///
/// A bare address is where a helper begins. An address written `0x...@N` is
/// where the helper is entered to cover registers N upward, which is how these
/// are recorded when someone worked out one entry rather than the whole run.
/// Both forms are checked against a detected helper, so a title with only a
/// single entry recorded still tests something.
fn expected_addresses() -> Option<Vec<(u32, Option<u8>)>> {
    let text = std::env::var("XENOLITH_ANALYSIS_HELPERS").ok()?;
    Some(
        text.split(',')
            .filter_map(|part| {
                let (address, register) = match part.trim().split_once('@') {
                    Some((address, register)) => (address, register.trim().parse().ok()),
                    None => (part.trim(), None),
                };
                let address = u32::from_str_radix(address.trim().trim_start_matches("0x"), 16);
                Some((address.ok()?, register))
            })
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

    let mut absent = Vec::new();
    for (address, register) in &expected {
        // A helper covering registers N upward is entered one instruction per
        // register past its start, because that is what the run is: one save or
        // restore each, in order.
        let matched = helpers.all().iter().any(|helper| match register {
            None => helper.start == *address,
            Some(register) => {
                let steps = u32::from(register.saturating_sub(helper.first_register));
                *register >= helper.first_register
                    && *register <= helper.last_register
                    && helper.start.saturating_add(steps * 4) == *address
            }
        });
        if !matched {
            absent.push(match register {
                None => format!("{address:#010x}"),
                Some(register) => format!("{address:#010x} (register {register})"),
            });
        }
    }

    assert!(
        absent.is_empty(),
        "detection did not account for these hand recorded addresses: {}",
        absent.join(", ")
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
