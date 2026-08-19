//! Reading an image from whichever input shape a subcommand was given.
//!
//! Two shapes are accepted. The usual one is a container, which is decoded
//! through the loader and carries a real section layout. The other is an image
//! something else already decoded, which matters because decoding a container
//! needs key material and reading an existing image does not. Work that would
//! otherwise be gated behind having a key stays possible that way.

use std::path::PathBuf;

use clap::Parser;
use miette::{Context, IntoDiagnostic, Result, miette};
use xenolith_xex::{Container, Image, PageKind, Section};

use crate::keys;

/// Where a subcommand reads its image from.
#[derive(Debug, Parser)]
pub(crate) struct Source {
    /// Path to the XEX file, or to a decoded image when `--raw` is given.
    pub(crate) file: PathBuf,

    /// Treat the input as an already decoded image rather than a container.
    ///
    /// Reading one needs no key material, which is what makes work against a
    /// title whose key is not to hand possible at all.
    #[arg(long)]
    pub(crate) raw: bool,

    /// Address a raw image loads at.
    #[arg(long, value_name = "ADDR", default_value = "0x82000000")]
    pub(crate) base: String,

    /// Path to a file holding the static key as 32 hexadecimal digits.
    #[arg(long, value_name = "PATH")]
    pub(crate) key_file: Option<PathBuf>,
}

/// Parses an address or size written in decimal or hexadecimal.
///
/// # Errors
///
/// Returns an error when the text is neither.
pub(crate) fn number(text: &str) -> Result<u32> {
    let trimmed = text.trim();
    let parsed = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .map_or_else(
            || trimmed.parse::<u32>().ok(),
            |hex| u32::from_str_radix(hex, 16).ok(),
        );

    parsed.ok_or_else(|| miette!("{text} is not an address or size this tool understands"))
}

impl Source {
    /// Loads the image this source describes.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read, when a container cannot be
    /// parsed or decoded, or when key material is needed and absent.
    pub(crate) fn load(&self) -> Result<Image> {
        let bytes = std::fs::read(&self.file)
            .into_diagnostic()
            .wrap_err_with(|| format!("reading {}", self.file.display()))?;

        if self.raw {
            let base = number(&self.base)?;
            let size = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
            // Nothing describes the layout of a bare image, so it is treated as
            // one executable span. Saying so is better than inventing a section
            // table.
            let sections = vec![Section {
                start: base,
                size,
                kind: PageKind::Code,
            }];
            return Ok(Image::new(base, bytes, sections));
        }

        let container = Container::parse(&bytes)
            .into_diagnostic()
            .wrap_err_with(|| format!("parsing {} as a XEX container", self.file.display()))?;

        let key = keys::resolve(self.key_file.as_deref())?;
        if container.encryption() == xenolith_xex::EncryptionType::Encrypted && key.is_none() {
            return Err(miette!(
                help = format!(
                    "{}, or pass an already decoded image with --raw",
                    keys::sources_consulted(self.key_file.as_deref())
                ),
                "{} is encrypted and no key material was found",
                self.file.display()
            ));
        }

        container
            .load(key.as_ref())
            .into_diagnostic()
            .wrap_err_with(|| format!("decoding the image of {}", self.file.display()))
    }
}
