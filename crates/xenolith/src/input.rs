//! Reading an image from whichever input shape a subcommand was given.
//!
//! Two shapes are accepted. The usual one is a container, which is decoded
//! through the loader and carries a real section layout. The other is an image
//! something else already decoded, which matters because decoding a container
//! needs key material and reading an existing image does not. Work that would
//! otherwise be gated behind having a key stays possible that way.

use std::collections::BTreeSet;
use std::path::PathBuf;

use clap::Parser;
use miette::{Context, IntoDiagnostic, Result, miette};
use xenolith_lift::{Imported, Imports};
use xenolith_xex::{
    Container, Image, ImportKind, OptionalHeaderKey, OptionalHeaderValue, PageKind, Section,
    imports,
};

use crate::keys;

/// What was read from an input.
pub(crate) struct Loaded {
    /// The decoded image.
    pub(crate) image: Image,
    /// Import thunks the container declares, or why they could not be read.
    ///
    /// Held unresolved because only lifting needs them. A container whose
    /// records will not decode should still be one the other subcommands can be
    /// pointed at, since they are how anyone would find out why.
    pub(crate) imports: Result<Imports>,
    /// What kernel build the image was made against, or nothing where the
    /// container did not say.
    ///
    /// A title's device layout depends on it, so it is carried to the boot
    /// program rather than read again there, which would mean parsing the
    /// container twice.
    pub(crate) built_against: Option<u32>,
    /// What the container declares the title is allowed to do, or nothing where
    /// it did not say.
    ///
    /// A title asks whether it holds a numbered privilege, and the answer is a
    /// bit in this word. It is read here for the same reason the kernel build
    /// is: the boot program is where a title's startup finds it, and reading it
    /// twice would mean parsing the container twice.
    pub(crate) privileges: Option<u32>,
    /// The kernel's variables the title imports, keyed by the slot each one's
    /// address belongs in.
    ///
    /// A function import has a slot and a thunk, and a variable has only the
    /// slot. Nothing fills that slot unless the boot program does, so a title
    /// reading through it reads the record word the container left there.
    /// Empty where the records would not decode, which lifting reports through
    /// the thunks.
    pub(crate) variables: Imports,
}

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
    /// Loads the image this source describes, and what its container declares.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read, when a container cannot be
    /// parsed or decoded, or when key material is needed and absent.
    pub(crate) fn load(&self) -> Result<Loaded> {
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
            return Ok(Loaded {
                image: Image::new(base, bytes, sections),
                imports: Ok(Imports::new()),
                /* A bare image carries no import libraries, so nothing says
                 * what it was built against, and no header directory, so
                 * nothing says what it is allowed to do. */
                built_against: None,
                privileges: None,
                variables: Imports::new(),
            });
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

        let image = container
            .load(key.as_ref())
            .into_diagnostic()
            .wrap_err_with(|| format!("decoding the image of {}", self.file.display()))?;

        let declared = imports(&image, container.import_libraries())
            .into_diagnostic()
            .wrap_err_with(|| format!("reading the imports of {}", self.file.display()));
        let variables = declared.as_ref().map_or_else(
            |_| Imports::new(),
            |found| {
                let functions: BTreeSet<(&str, u16)> = found
                    .iter()
                    .filter(|import| import.kind == ImportKind::Thunk)
                    .map(|import| (import.library, import.ordinal))
                    .collect();
                found
                    .iter()
                    .filter(|import| {
                        import.kind == ImportKind::Slot
                            && !functions.contains(&(import.library, import.ordinal))
                    })
                    .map(|import| (import.address, imported(import.library, import.ordinal)))
                    .collect()
            },
        );
        let thunks = declared.map(|found| {
            found
                .into_iter()
                .filter(|import| import.kind == ImportKind::Thunk)
                .map(|import| (import.address, imported(import.library, import.ordinal)))
                .collect()
        });

        /* What the image was built against, from the kernel's own import
         * library. A container without that library says nothing rather than
         * nought, because nought is a build number that means something. */
        let built_against = container
            .import_libraries()
            .iter()
            .find(|library| library.name == "xboxkrnl.exe")
            .map(|library| library.version.0);

        /* What the title is allowed to do, from the system flags word in the
         * optional header directory. A container that does not carry one says
         * nothing, which is answered the same way as a word with every bit
         * clear rather than being filled in from a guess. */
        let privileges = container
            .optional_header(OptionalHeaderKey(0x0003_0000))
            .and_then(OptionalHeaderValue::inline);

        Ok(Loaded {
            image,
            imports: thunks,
            built_against,
            privileges,
            variables,
        })
    }
}

/// What an import names, owned so that it outlives the container.
fn imported(library: &str, ordinal: u16) -> Imported {
    Imported {
        library: library.to_owned(),
        ordinal,
    }
}
