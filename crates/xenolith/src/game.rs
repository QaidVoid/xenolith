//! What a lift needs to know about the game it is making a program of.
//!
//! A program is named for the game: its window says the game's name, and a
//! desktop, a sound server and the program file go by an id made from it. The
//! game also brings the files it reads beside its executable, and the program
//! runs where those are.

use std::fmt::Write as _;
use std::path::Path;

use miette::{IntoDiagnostic, Result, WrapErr, miette};

/// What a program is called where a game gives no name and none is asked for.
const UNNAMED: &str = "xenolith";

/// What a game's program is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Naming {
    /// What its window is titled.
    pub(crate) name: String,
    /// What the program file is called, and what a desktop and a sound server
    /// know it by.
    pub(crate) id: String,
}

/// Settles what a game's program is called, from what was asked for and what
/// the game says of itself.
///
/// # Errors
///
/// Returns an error when an asked for id would not do as a file name or a
/// desktop's id for a program.
pub(crate) fn naming(name: Option<&str>, id: Option<&str>, found: Option<&str>) -> Result<Naming> {
    let name = name.or(found).unwrap_or(UNNAMED).trim().to_owned();
    let id = match id {
        Some(id) if fits(id) => id.to_owned(),
        Some(id) => {
            return Err(miette!(
                help = "use lower case letters, digits, dots, dashes and underscores, starting with a letter or digit",
                "{id:?} will not do as a program's id"
            ));
        }
        None => id_from(&name),
    };
    Ok(Naming { name, id })
}

/// Whether text will do as an id: a file name and a desktop's id at once.
fn fits(id: &str) -> bool {
    id.starts_with(|first: char| first.is_ascii_lowercase() || first.is_ascii_digit())
        && id.chars().all(|letter| {
            letter.is_ascii_lowercase()
                || letter.is_ascii_digit()
                || matches!(letter, '.' | '-' | '_')
        })
}

/// Makes an id from a name: its letters and digits in lower case, with one dash
/// for each run of anything else.
fn id_from(name: &str) -> String {
    let mut id = String::new();
    for letter in name.chars() {
        if letter.is_ascii_alphanumeric() {
            id.push(letter.to_ascii_lowercase());
        } else if !id.is_empty() && !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_end_matches('-');
    if id.is_empty() { UNNAMED } else { id }.to_owned()
}

/// Writes text as a C string, with everything but plain letters and spaces
/// escaped so that no name can end the string or the line early.
pub(crate) fn c_string(text: &str) -> String {
    let mut written = String::from("\"");
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b' ' {
            written.push(char::from(byte));
        } else {
            let _ = write!(written, "\\{byte:03o}");
        }
    }
    written.push('"');
    written
}

/// Links every file beside a game's executable into the directory its program
/// is built in, returning how many were linked.
///
/// The program reads a game's files from the directory it runs in, so they are
/// put there. A name the directory already holds is left alone, which keeps
/// what the lift wrote and the links an earlier lift made. Nothing is linked
/// where the program is built beside the executable already, and the directory
/// the program is built in is never linked into itself.
///
/// # Errors
///
/// Returns an error when the game's directory cannot be read or a link cannot
/// be made.
pub(crate) fn link_the_data(executable: &Path, out: &Path) -> Result<usize> {
    let executable = executable
        .canonicalize()
        .into_diagnostic()
        .wrap_err_with(|| format!("finding {}", executable.display()))?;
    let out = out
        .canonicalize()
        .into_diagnostic()
        .wrap_err_with(|| format!("finding {}", out.display()))?;
    let Some(game) = executable.parent() else {
        return Ok(0);
    };
    if game == out {
        return Ok(0);
    }

    let mut linked = 0;
    let entries = std::fs::read_dir(game)
        .into_diagnostic()
        .wrap_err_with(|| format!("reading {}", game.display()))?;
    for entry in entries {
        let entry = entry
            .into_diagnostic()
            .wrap_err_with(|| format!("reading {}", game.display()))?;
        let from = entry.path();
        if from == executable || out.starts_with(&from) {
            continue;
        }
        let to = out.join(entry.file_name());
        if to.symlink_metadata().is_ok() {
            continue;
        }
        std::os::unix::fs::symlink(&from, &to)
            .into_diagnostic()
            .wrap_err_with(|| format!("linking {} to {}", to.display(), from.display()))?;
        linked += 1;
    }
    Ok(linked)
}

#[cfg(test)]
mod tests {
    use super::{c_string, link_the_data, naming};

    #[test]
    fn a_game_s_own_name_names_the_program() {
        let named = naming(None, None, Some("Example Game"));
        assert_eq!(
            named.ok().map(|named| (named.name, named.id)),
            Some(("Example Game".to_owned(), "example-game".to_owned()))
        );
    }

    #[test]
    fn what_was_asked_for_wins_over_what_the_game_says() {
        let named = naming(Some("Mine"), Some("mine-2"), Some("Theirs"));
        assert_eq!(
            named.ok().map(|named| (named.name, named.id)),
            Some(("Mine".to_owned(), "mine-2".to_owned()))
        );
    }

    #[test]
    fn a_name_with_nothing_to_make_an_id_from_is_still_named() {
        let named = naming(None, None, Some("!!!"));
        assert_eq!(
            named.ok().map(|named| named.id),
            Some("xenolith".to_owned())
        );
        let named = naming(None, None, None);
        assert_eq!(
            named.ok().map(|named| (named.name, named.id)),
            Some(("xenolith".to_owned(), "xenolith".to_owned()))
        );
    }

    #[test]
    fn an_id_that_will_not_do_as_a_file_name_is_refused() {
        for id in ["Upper", "has space", "../escape", "-dash", ""] {
            assert!(naming(None, Some(id), None).is_err(), "{id:?} was taken");
        }
    }

    #[test]
    fn a_name_cannot_end_its_c_string_early() {
        assert_eq!(c_string("A \"B\"\\\n"), "\"A \\042B\\042\\134\\012\"");
    }

    #[test]
    fn the_files_beside_a_game_are_linked_and_nothing_else() {
        let root = std::env::temp_dir().join("xenolith-link-the-data");
        let _ = std::fs::remove_dir_all(&root);
        let game = root.join("game");
        let out = game.join("built");
        std::fs::create_dir_all(&out).ok();
        for name in ["default.xex", "data.img", "Makefile"] {
            std::fs::write(game.join(name), name).ok();
        }
        std::fs::write(out.join("Makefile"), "the lift's own").ok();

        let linked = link_the_data(&game.join("default.xex"), &out).ok();

        assert_eq!(linked, Some(1));
        assert!(out.join("data.img").symlink_metadata().is_ok());
        assert!(
            !out.join("default.xex").exists(),
            "the executable was linked"
        );
        assert!(
            !out.join("built").exists(),
            "the output was linked into itself"
        );
        assert_eq!(
            std::fs::read_to_string(out.join("Makefile"))
                .ok()
                .as_deref(),
            Some("the lift's own")
        );

        /* A second lift into the same place finds every link already made. */
        assert_eq!(link_the_data(&game.join("default.xex"), &out).ok(), Some(0));
        let _ = std::fs::remove_dir_all(&root);
    }
}
