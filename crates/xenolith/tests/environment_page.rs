//! The environment page and the variables the code reads have to agree.
//!
//! An environment variable is an interface with no compiler checking it. A
//! variable the code reads and the page does not name is one nobody can find;
//! a variable the page names and the code does not read is a page that lies, and
//! the reader who sets it gets nothing and no message. Both directions are
//! checked here.
//!
//! What the code reads is taken from the source rather than from a list kept
//! beside it, because a list would be the second thing to go stale.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// The page that documents them.
const PAGE: &str = "docs/reference/environment.md";

/// Returns the root of the workspace.
fn root() -> PathBuf {
    // The test binary runs in `target/release/deps`, so the workspace is three
    // levels up.
    let mut at = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    at.pop();
    at.pop();
    at
}

/// Returns every variable name the given text reads through `getenv` or `var`.
fn read_from(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for (opening, closing) in [
        ("getenv(\"", "\")"),
        ("env::var(\"", "\")"),
        ("env::var_os(\"", "\")"),
        ("const SAMPLE_VARIABLE: &str = \"", "\""),
        ("require_path(\"", "\")"),
    ] {
        let mut rest = text;
        while let Some(at) = rest.find(opening) {
            rest = &rest[at + opening.len()..];
            if let Some(end) = rest.find(closing) {
                found.insert(rest[..end].to_owned());
                rest = &rest[end..];
            }
        }
    }

    /* An `#[ignore]` reason names the variables that would have run the test,
     * and a reason can name two of them joined by `and`. They are the same
     * interface as a call, so they are collected the same way, split out of the
     * sentence they sit in. */
    for line in text.lines() {
        let Some(at) = line.find("ignore = \"needs ") else {
            continue;
        };
        let reason = &line[at + "ignore = \"needs ".len()..];
        let Some(end) = reason.find('"') else {
            continue;
        };
        for word in reason[..end].split_whitespace() {
            let name = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
            if name.starts_with("XENOLITH_") {
                found.insert(name.to_owned());
            }
        }
    }

    found
}

/// Returns every variable name the page puts in backticks in a table row.
fn named_on_the_page(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for line in text.lines() {
        if !line.starts_with('|') {
            continue;
        }
        // The first cell of a table row is the variable, in backticks.
        let Some(cell) = line.split('|').nth(1) else {
            continue;
        };
        let cell = cell.trim();
        if let Some(name) = cell
            .strip_prefix('`')
            .and_then(|rest| rest.strip_suffix('`'))
        {
            if name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
            {
                found.insert(name.to_owned());
            }
        }
    }
    found
}

/// Reads a file, or says it could not.
#[allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
fn read(path: &PathBuf) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
}

/// The variables the code reads, over everything that reads one.
#[allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
fn read_by_the_code() -> BTreeSet<String> {
    let root = root();
    let mut found = BTreeSet::new();

    let runtime = root.join("crates/xenolith-lift/runtime");
    for entry in std::fs::read_dir(&runtime).expect("the runtime directory should be there") {
        let path = entry.expect("a directory entry").path();
        if path
            .extension()
            .is_some_and(|kind| kind == "c" || kind == "h")
        {
            found.extend(read_from(&read(&path)));
        }
    }

    // Every crate's source, since a variable can be read anywhere. A name a
    // library reads is as much an interface as one the front end reads.
    for crate_entry in std::fs::read_dir(root.join("crates")).expect("the crates directory") {
        let crate_path = crate_entry.expect("a directory entry").path();
        let source = crate_path.join("src");
        let Ok(entries) = std::fs::read_dir(&source) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("a directory entry").path();
            if path.extension().is_some_and(|kind| kind == "rs") {
                found.extend(read_from(&read(&path)));
            }
        }
    }

    // And the variables the tests read, which are an interface too: each one
    // names an input a check needs, and a reader who cannot find the name
    // cannot run the check.
    for crate_entry in std::fs::read_dir(root.join("crates")).expect("the crates directory") {
        let crate_path = crate_entry.expect("a directory entry").path();
        let tests = crate_path.join("tests");
        let Ok(entries) = std::fs::read_dir(&tests) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("a directory entry").path();
            if path.extension().is_some_and(|kind| kind == "rs") {
                found.extend(read_from(&read(&path)));
            }
        }
    }

    // `WANTED` is a test telling the program it launched what to hold, which is
    // internal to one check rather than an input to the tool.
    found.remove("WANTED");

    found
}

#[test]
fn every_variable_the_code_reads_is_on_the_page() {
    let page = named_on_the_page(&read(&root().join(PAGE)));
    let missing: Vec<_> = read_by_the_code()
        .into_iter()
        .filter(|name| !page.contains(name))
        .collect();

    assert!(
        missing.is_empty(),
        "the code reads {} and the page does not name it: {missing:?}",
        if missing.len() == 1 {
            "a variable"
        } else {
            "variables"
        }
    );
}

#[test]
fn the_page_names_no_variable_the_code_does_not_read() {
    let page = named_on_the_page(&read(&root().join(PAGE)));
    let code = read_by_the_code();
    let extra: Vec<_> = page
        .into_iter()
        .filter(|name| !code.contains(name))
        .collect();

    assert!(
        extra.is_empty(),
        "the page names {} the code does not read: {extra:?}",
        if extra.len() == 1 {
            "a variable"
        } else {
            "variables"
        }
    );
}
