//! Resolution of the static key used to unwrap a container's session key.
//!
//! No key is compiled into this tool. It is looked up at runtime, and when it
//! is missing the error names every place that was consulted so the user knows
//! exactly where to put it.

use std::path::{Path, PathBuf};

use miette::{Context, IntoDiagnostic, Result, miette};
use xenolith_xex::KeyMaterial;

/// Environment variable holding the key as hexadecimal.
pub(crate) const KEY_ENV: &str = "XENOLITH_XEX_KEY";

/// Returns the default path a key file is looked for at.
#[must_use]
pub(crate) fn default_key_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("xenolith")
        .join("xex.key")
}

/// Resolves key material from the environment or a key file.
///
/// Returns `None` when no source holds a key, which lets a caller that does not
/// need one carry on.
///
/// # Errors
///
/// Returns an error when a source holds something that is not a valid key, or
/// when an explicitly requested key file cannot be read.
pub(crate) fn resolve(explicit: Option<&Path>) -> Result<Option<KeyMaterial>> {
    /* The explicit path first, then the environment, then the default.
     *
     * A flag that is silently ignored is a trap: a caller who names a file has
     * said which one they mean, and the environment is a fallback for a caller
     * who said nothing. Checking the environment first makes `--key-file` do
     * nothing at all whenever the variable happens to be set, which is exactly
     * when a caller is most likely to be trying to use a different key. */
    if let Some(path) = explicit {
        return read_key_file(path).map(Some);
    }

    if let Some(text) = std::env::var_os(KEY_ENV) {
        let text = text
            .into_string()
            .map_err(|_| miette!("{KEY_ENV} is not valid text"))?;
        let key = KeyMaterial::from_hex(&text)
            .into_diagnostic()
            .wrap_err_with(|| format!("reading the key from {KEY_ENV}"))?;
        return Ok(Some(key));
    }

    let default = default_key_path();
    if default.is_file() {
        return read_key_file(&default).map(Some);
    }

    Ok(None)
}

/// Reads and parses a key file.
fn read_key_file(path: &Path) -> Result<KeyMaterial> {
    let text = std::fs::read_to_string(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("reading the key file {}", path.display()))?;

    KeyMaterial::from_hex(text.trim())
        .into_diagnostic()
        .wrap_err_with(|| format!("parsing the key file {}", path.display()))
}

/// Describes where a key was looked for, for use in an error message.
#[must_use]
pub(crate) fn sources_consulted(explicit: Option<&Path>) -> String {
    let path = explicit.map_or_else(default_key_path, Path::to_path_buf);
    format!(
        "set the {KEY_ENV} environment variable to the key as 32 hexadecimal digits, or write it to {}",
        path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::resolve;
    use std::path::PathBuf;
    use xenolith_xex::KeyMaterial;

    /// A key file written under the target directory, and the key it holds.
    ///
    /// `CARGO_TARGET_TMPDIR` is not set for a unit test, so this uses the
    /// target directory's parent that cargo does set.
    fn a_key_file(name: &str, text: &str) -> (PathBuf, KeyMaterial) {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("key-tests")
            .join(name);
        std::fs::create_dir_all(&directory).expect("making a directory");
        let path = directory.join("xex.key");
        std::fs::write(&path, text).expect("writing the key file");
        (path, KeyMaterial::from_hex(text).expect("a key"))
    }

    /// An explicit key file is read where it names one.
    ///
    /// This is the half of the ordering that can be checked without setting an
    /// environment variable for the whole process: the file is a key that is
    /// not the one the default path holds, so a resolver that read the default
    /// instead would come back with a different key or with none.
    #[test]
    fn an_explicit_key_file_is_read() {
        let text = "00112233445566778899aabbccddeeff";
        let (path, expected) = a_key_file("explicit", text);

        let key = resolve(Some(&path))
            .expect("resolving the key")
            .expect("a key was resolved");
        assert_eq!(key, expected, "the explicit file was not the one read");
    }

    /// A key file that is not there is an error naming it, not a quiet nothing.
    ///
    /// A caller who named a file has said which one they mean, and being told
    /// it could not be read is the whole of what can be said. Coming back with
    /// no key would leave the caller to find out later, from a decode that
    /// failed for a reason that does not name the file.
    #[test]
    fn an_explicit_key_file_that_is_not_there_is_an_error() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("key-tests")
            .join("not_there")
            .join("xex.key");
        let resolved = resolve(Some(&path));
        assert!(
            resolved.is_err(),
            "a key file that is not there was not reported"
        );
    }
}
