//! The disassemble subcommand.
//!
//! Two input shapes are accepted. The usual one is a container, which is
//! decoded through the loader. The other is an image that has already been
//! decoded by something else, which matters because decoding a container needs
//! key material and reading an existing image does not. The coverage sweep is
//! the measurement the instruction table's real world accuracy rests on, so it
//! must not be gated behind having a key.

use std::collections::HashMap;
use std::path::PathBuf;

use clap::Parser;
use miette::{Context, IntoDiagnostic, Result, miette};
use xenolith_ppc::Instruction;
use xenolith_xex::{Container, Image, PageKind, Section};

use crate::keys;

/// How many instructions are printed when no range is given.
const DEFAULT_WINDOW: u32 = 64;

/// Arguments of the disassemble subcommand.
#[derive(Debug, Parser)]
pub(crate) struct Args {
    /// Path to the XEX file, or to a decoded image when `--raw` is given.
    pub(crate) file: PathBuf,

    /// Treat the input as an already decoded image rather than a container.
    ///
    /// Reading one needs no key material, which is what makes a coverage sweep
    /// possible against a title whose key is not to hand.
    #[arg(long)]
    pub(crate) raw: bool,

    /// Address a raw image loads at.
    #[arg(long, value_name = "ADDR", default_value = "0x82000000")]
    pub(crate) base: String,

    /// Address to start disassembling at.
    #[arg(long, value_name = "ADDR")]
    pub(crate) start: Option<String>,

    /// How many bytes to disassemble.
    #[arg(long, value_name = "BYTES", conflicts_with = "end")]
    pub(crate) length: Option<String>,

    /// Address to stop before.
    #[arg(long, value_name = "ADDR")]
    pub(crate) end: Option<String>,

    /// Report coverage over every executable section instead of printing code.
    #[arg(long)]
    pub(crate) sweep: bool,

    /// Disassemble a range that is not marked executable.
    #[arg(long)]
    pub(crate) allow_data: bool,

    /// Path to a file holding the static key as 32 hexadecimal digits.
    #[arg(long, value_name = "PATH")]
    pub(crate) key_file: Option<PathBuf>,
}

/// Parses an address or size written in decimal or hexadecimal.
fn number(text: &str) -> Result<u32> {
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

/// Loads the image to disassemble, from whichever input shape was given.
fn load(args: &Args) -> Result<Image> {
    let bytes = std::fs::read(&args.file)
        .into_diagnostic()
        .wrap_err_with(|| format!("reading {}", args.file.display()))?;

    if args.raw {
        let base = number(&args.base)?;
        let size = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        // Nothing describes the layout of a bare image, so it is treated as one
        // executable span. Saying so is better than inventing a section table.
        let sections = vec![Section {
            start: base,
            size,
            kind: PageKind::Code,
        }];
        return Ok(Image::new(base, bytes, sections));
    }

    let container = Container::parse(&bytes)
        .into_diagnostic()
        .wrap_err_with(|| format!("parsing {} as a XEX container", args.file.display()))?;

    let key = keys::resolve(args.key_file.as_deref())?;
    if container.encryption() == xenolith_xex::EncryptionType::Encrypted && key.is_none() {
        return Err(miette!(
            help = format!(
                "{}, or pass an already decoded image with --raw",
                keys::sources_consulted(args.key_file.as_deref())
            ),
            "{} is encrypted and no key material was found",
            args.file.display()
        ));
    }

    container
        .load(key.as_ref())
        .into_diagnostic()
        .wrap_err_with(|| format!("decoding the image of {}", args.file.display()))
}

/// Resolves the range to disassemble.
fn range(args: &Args, image: &Image) -> Result<(u32, u32)> {
    let start = match &args.start {
        Some(text) => number(text)?,
        None => image.entry_point().unwrap_or_else(|| image.base_address()),
    };

    if start % 4 != 0 {
        return Err(miette!(
            help = "instructions are four bytes and four byte aligned, so a \
                    shifted start produces plausible looking and entirely wrong output",
            "{start:#010x} is not four byte aligned"
        ));
    }

    let end = match (&args.length, &args.end) {
        (Some(text), _) => start.saturating_add(number(text)?),
        (_, Some(text)) => number(text)?,
        _ => start.saturating_add(DEFAULT_WINDOW * 4),
    };

    if end <= start {
        return Err(miette!("the range {start:#010x}..{end:#010x} is empty"));
    }

    Ok((start, end))
}

/// Runs the disassemble subcommand.
///
/// # Errors
///
/// Returns an error when the input cannot be read or decoded, when the range is
/// malformed or unmapped, or when key material is needed and absent.
pub(crate) fn run(args: &Args) -> Result<()> {
    let image = load(args)?;

    if args.sweep {
        return sweep(&image);
    }

    let (start, end) = range(args, &image)?;

    if !args.allow_data {
        let executable = image
            .section_at(start)
            .is_some_and(|section| section.kind.is_executable());
        if !executable {
            return Err(miette!(
                help = "pass --allow-data to disassemble it anyway",
                "{start:#010x} is not in an executable section"
            ));
        }
    }

    let mut address = start;
    while address < end {
        let word = image
            .u32(address)
            .into_diagnostic()
            .wrap_err_with(|| format!("reading {address:#010x}"))?;
        let instruction = Instruction::decode(word);

        let mark = if instruction.is_unknown() { "?" } else { " " };
        println!(
            "{address:#010x}  {word:08x} {mark} {}",
            instruction.render(address)
        );

        address = address.saturating_add(4);
    }

    Ok(())
}

/// Reports how much of the executable code the decoder accounts for.
fn sweep(image: &Image) -> Result<()> {
    let mut decoded = 0u64;
    let mut undecoded = 0u64;
    let mut unknown: HashMap<u32, (u64, u32)> = HashMap::new();

    for section in image.executable_sections() {
        let mut address = section.start;
        while u64::from(address) < section.end() {
            let Ok(word) = image.u32(address) else {
                break;
            };

            if Instruction::decode(word).is_unknown() {
                undecoded += 1;
                let entry = unknown.entry(word).or_insert((0, address));
                entry.0 += 1;
            } else {
                decoded += 1;
            }

            address = address.saturating_add(4);
        }
    }

    let total = decoded + undecoded;
    if total == 0 {
        return Err(miette!("the image holds no executable words to sweep"));
    }

    println!("swept {total} instruction words");
    println!("  decoded    {decoded:>10}  ({}%)", percent(decoded, total));
    println!(
        "  undecoded  {undecoded:>10}  ({}%)",
        percent(undecoded, total)
    );
    println!("  distinct undecoded encodings: {}", unknown.len());

    let mut ranked: Vec<(u32, u64, u32)> = unknown
        .into_iter()
        .map(|(word, (count, example))| (word, count, example))
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    if !ranked.is_empty() {
        println!("\n  most frequent undecoded encodings");
        for (word, count, example) in ranked.iter().take(20) {
            println!(
                "    {word:08x}  primary {:>2}  {count:>9} times  first at {example:#010x}",
                word >> 26
            );
        }
    }

    Ok(())
}

/// Returns a share of a total as a percentage, to three decimal places.
///
/// Kept in integer arithmetic so the figure is exact. These are counts of
/// instruction words, and a coverage number that quietly rounds is the last
/// thing wanted from a measurement meant to expose gaps.
fn percent(part: u64, total: u64) -> String {
    if total == 0 {
        return "0.000".to_owned();
    }
    let thousandths = part.saturating_mul(100_000) / total;
    format!("{}.{:03}", thousandths / 1000, thousandths % 1000)
}
