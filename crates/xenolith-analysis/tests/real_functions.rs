//! Function discovery against a real title.
//!
//! Discovery reaches what direct calls reach. Anything called only through a
//! pointer is invisible to it, which in compiled game code means everything
//! behind a virtual table. What this measures is how much of a real code
//! section that leaves unclaimed, which is the number that says whether a
//! second pass is worth building and how much it has to find.
//!
//! No game data is committed. The image is supplied through the environment and
//! the test skips when it is absent.

use std::path::PathBuf;

use xenolith_analysis::{Origin, analyze};
use xenolith_xex::{Image, PageKind, Section};

/// Returns the image path, base address, and entry point, if all were supplied.
fn supplied_source() -> Option<(PathBuf, u32, Option<u32>)> {
    let path = PathBuf::from(std::env::var_os("XENOLITH_ANALYSIS_IMAGE")?);
    let number = |name: &str| {
        std::env::var(name)
            .ok()
            .and_then(|text| u32::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok())
    };

    Some((
        path,
        number("XENOLITH_ANALYSIS_BASE")?,
        number("XENOLITH_ANALYSIS_ENTRY"),
    ))
}

/// Builds an image treating the whole file as one executable span.
fn image_of(bytes: Vec<u8>, base: u32, entry: Option<u32>) -> Image {
    let size = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    let sections = vec![Section {
        start: base,
        size,
        kind: PageKind::Code,
    }];
    Image::new(base, bytes, sections).with_entry_point(entry)
}

/// Loads the supplied image, or skips the enclosing test.
macro_rules! supplied_image {
    () => {
        match supplied_source() {
            Some((path, base, entry)) => {
                let bytes = std::fs::read(&path).expect("reading the analysis image");
                let words = bytes.len() / 4;
                (image_of(bytes, base, entry), words as u64)
            }
            None => {
                eprintln!("skipping: XENOLITH_ANALYSIS_IMAGE is not set");
                return;
            }
        }
    };
}

#[test]
fn reports_what_direct_calls_reach() {
    let (image, words) = supplied_image!();

    let program = analyze(&image, &[]);
    let claimed = program.claimed_instructions();

    eprintln!("functions      {:>10}", program.function_count());
    eprintln!(
        "  entry point  {:>10}",
        program.count_from(Origin::EntryPoint)
    );
    eprintln!("  called       {:>10}", program.count_from(Origin::Called));
    eprintln!("  root         {:>10}", program.count_from(Origin::Root));
    eprintln!("words          {words:>10}");
    eprintln!(
        "claimed        {claimed:>10}  ({}.{:03} percent)",
        claimed * 100 / words.max(1),
        (claimed * 100_000 / words.max(1)) % 1000
    );

    let unresolved = program
        .functions()
        .filter(|f| f.has_unresolved_transfer())
        .count();
    let tail_calls: usize = program.functions().map(|f| f.tail_calls.len()).sum();
    eprintln!("functions with an unresolved transfer {unresolved}");
    eprintln!("tail calls     {tail_calls:>10}");

    assert!(
        program.function_count() > 0,
        "nothing was discovered, so the entry point was not usable"
    );
    assert!(
        claimed <= words,
        "more instructions were claimed than the section holds"
    );
}

/// Functions may share code, but an instruction claimed by two of them must
/// still only be counted once, or coverage would read above what exists.
#[test]
fn claimed_instructions_never_exceed_the_section() {
    let (image, words) = supplied_image!();

    let program = analyze(&image, &[]);

    assert!(program.claimed_instructions() <= words);
}
