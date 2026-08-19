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
    if let Some(text) = std::env::var_os(KEY_ENV) {
        let text = text
            .into_string()
            .map_err(|_| miette!("{KEY_ENV} is not valid text"))?;
        let key = KeyMaterial::from_hex(&text)
            .into_diagnostic()
            .wrap_err_with(|| format!("reading the key from {KEY_ENV}"))?;
        return Ok(Some(key));
    }

    if let Some(path) = explicit {
        return read_key_file(path).map(Some);
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
