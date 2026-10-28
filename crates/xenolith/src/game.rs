//! What a lift needs to know about the game it is making a program of.
//!
//! A program is named for the game: its window says the game's name, and a
//! desktop, a sound server and the program file go by an id made from it. The
//! program reads the game itself from the player's copy, which it looks for
//! under that id among xenolith's data, so the lift puts the copy it was given
//! there.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

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

/// Where xenolith keeps the user's data: `XENOLITH_DATA` where it names a whole
/// path, then `xenolith` in `XDG_DATA_HOME`, then `~/.local/share/xenolith`,
/// as a built program looks for it.
pub(crate) fn where_xenolith_keeps() -> Option<PathBuf> {
    let whole = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    whole("XENOLITH_DATA")
        .or_else(|| whole("XDG_DATA_HOME").map(|data| data.join("xenolith")))
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|home| !home.is_empty())
                .map(|home| PathBuf::from(home).join(".local/share/xenolith"))
        })
}

/// What putting a game where its program looks for it came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Placed {
    /// The place was made, as a link to the game's folder.
    Linked(PathBuf),
    /// Something was there already and was left as it was.
    There(PathBuf),
}

/// Puts the folder holding a game's executable where the program built from
/// it looks for the game: `games/<id>` in xenolith's data, as a link.
///
/// Whatever is there already is left alone, whether it is this game, a copy of
/// it, or something the player put there on purpose.
///
/// # Errors
///
/// Returns an error when the executable cannot be found or the link cannot be
/// made.
pub(crate) fn put_the_game_in_place(executable: &Path, id: &str, data: &Path) -> Result<Placed> {
    let executable = executable
        .canonicalize()
        .into_diagnostic()
        .wrap_err_with(|| format!("finding {}", executable.display()))?;
    let Some(game) = executable.parent() else {
        return Err(miette!("{} is not in a folder", executable.display()));
    };
    let games = data.join("games");
    let place = games.join(id);
    if place.symlink_metadata().is_ok() {
        return Ok(Placed::There(place));
    }
    std::fs::create_dir_all(&games)
        .into_diagnostic()
        .wrap_err_with(|| format!("making {}", games.display()))?;
    std::os::unix::fs::symlink(game, &place)
        .into_diagnostic()
        .wrap_err_with(|| format!("linking {} to {}", place.display(), game.display()))?;
    Ok(Placed::Linked(place))
}

#[cfg(test)]
mod tests {
    use super::{Placed, c_string, naming, put_the_game_in_place};

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
    fn a_game_is_linked_where_its_program_looks_and_left_once_there() {
        let root = std::env::temp_dir().join("xenolith-put-the-game-in-place");
        let _ = std::fs::remove_dir_all(&root);
        let game = root.join("my copy");
        let data = root.join("data");
        std::fs::create_dir_all(&game).ok();
        std::fs::write(game.join("default.xex"), "x").ok();

        let placed = put_the_game_in_place(&game.join("default.xex"), "a-game", &data).ok();
        let place = data.join("games/a-game");
        assert_eq!(placed, Some(Placed::Linked(place.clone())));
        assert!(
            place.join("default.xex").is_file(),
            "the link does not reach the game"
        );

        /* A second lift finds it there and leaves it. */
        let again = put_the_game_in_place(&game.join("default.xex"), "a-game", &data).ok();
        assert_eq!(again, Some(Placed::There(place)));
        let _ = std::fs::remove_dir_all(&root);
    }
}
