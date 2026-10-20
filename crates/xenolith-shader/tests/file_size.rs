//! Keeping the crate readable by keeping one file from becoming the crate.
//!
//! The translator arrived as one file of nearly four thousand lines, and the
//! only thing that made it that size was that nobody had to split it. A cap is
//! worth having only if it is enforced by something that runs, so this reads the
//! source and fails rather than leaving the number in a document.

use std::path::Path;

/// The most lines one file may hold.
///
/// A file this long is still one a reader can hold in their head at once. Past
/// it the things that belong together start being separated by things that do
/// not, and finding one of them means reading past the others.
const MOST_LINES: usize = 700;

/// Every source file in the crate, as a path and a line count.
///
/// # Panics
///
/// Where the source cannot be read, because a check that cannot see the files
/// must not pass by finding none of them.
#[allow(clippy::panic)]
fn sources() -> Vec<(String, usize)> {
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    let mut waiting = vec![here];
    while let Some(directory) = waiting.pop() {
        let entries = std::fs::read_dir(&directory)
            .unwrap_or_else(|why| panic!("cannot read {}: {why}", directory.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|why| panic!("cannot read an entry: {why}"));
            let path = entry.path();
            if path.is_dir() {
                waiting.push(path);
            } else if path.extension().is_some_and(|one| one == "rs") {
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|why| panic!("cannot read {}: {why}", path.display()));
                let name = path
                    .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                found.push((name, text.lines().count()));
            }
        }
    }
    assert!(!found.is_empty(), "the source was read and held no file");
    found
}

#[test]
fn file_size_stays_under_the_cap() {
    let over: Vec<String> = sources()
        .into_iter()
        .filter(|(_, lines)| *lines > MOST_LINES)
        .map(|(name, lines)| format!("{name} holds {lines} lines"))
        .collect();
    assert!(
        over.is_empty(),
        "these files are past {MOST_LINES} lines and want splitting: {over:?}"
    );
}

#[test]
fn the_file_size_cap_is_nearly_reached_by_something() {
    let longest = sources().into_iter().map(|(_, lines)| lines).max();
    assert!(
        longest.is_some_and(|lines| lines > MOST_LINES / 2),
        "no file comes within half the cap, so the cap is not measuring anything"
    );
}
