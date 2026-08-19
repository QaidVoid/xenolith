//! The inspect subcommand, which summarizes a container in readable form.
//!
//! The summary is derived from the headers alone, so it works on an encrypted
//! title with no key available. Decoding the image is a separate opt in, since
//! that is the only part that actually needs one.

use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::Parser;
use miette::{Context, IntoDiagnostic, Result, miette};
use xenolith_xex::{
    CompressionType, Container, EncryptionType, ExecutionInfo, Image, ImportKind, PageKind,
    Section, imports,
};

use crate::keys;

/// Arguments of the inspect subcommand.
#[derive(Debug, Parser)]
pub(crate) struct Args {
    /// Path to the XEX file to summarize.
    pub(crate) file: PathBuf,

    /// Path to a file holding the static key as 32 hexadecimal digits.
    #[arg(long, value_name = "PATH")]
    pub(crate) key_file: Option<PathBuf>,

    /// Decode the image as well as reading its headers.
    #[arg(long)]
    pub(crate) decode: bool,

    /// List every import, with its address, library, ordinal, and kind.
    ///
    /// What a record names is written in the image rather than in the headers,
    /// so this decodes the image and needs whatever key material that takes.
    #[arg(long)]
    pub(crate) imports: bool,
}

/// Runs the inspect subcommand.
///
/// # Errors
///
/// Returns an error when the file cannot be read, when it is not a container
/// this tool understands, or when decoding was requested and could not be done.
pub(crate) fn run(args: &Args) -> Result<()> {
    let bytes = std::fs::read(&args.file)
        .into_diagnostic()
        .wrap_err_with(|| format!("reading {}", args.file.display()))?;

    let container = Container::parse(&bytes)
        .into_diagnostic()
        .wrap_err_with(|| format!("parsing {} as a XEX container", args.file.display()))?;

    println!("{}", args.file.display());
    print_identity(&container);
    print_layout(&container, bytes.len());
    print_sections(&container.sections());
    print_imports(&container);

    if args.decode || args.imports {
        let image = decode(args, &container)?;
        if args.decode {
            print_decoded(&image);
        }
        if args.imports {
            print_import_records(&container, &image)?;
        }
    }

    Ok(())
}

/// Prints the title identity recorded in the container.
fn print_identity(container: &Container<'_>) {
    println!("\n  format            {:?}", container.format());

    let Some(ExecutionInfo {
        title_id,
        media_id,
        version,
        base_version,
        disc_number,
        disc_count,
        ..
    }) = container.execution_info()
    else {
        println!("  title             no execution metadata");
        return;
    };

    println!("  title id          {title_id:#010x}");
    println!("  media id          {media_id:#010x}");
    println!("  version           {version} (base {base_version})");
    println!("  disc              {disc_number} of {disc_count}");
}

/// Prints where the image loads and how it is stored.
fn print_layout(container: &Container<'_>, file_size: usize) {
    let security = container.security_info();

    println!(
        "\n  file size         {}",
        human_size(u64::try_from(file_size).unwrap_or(u64::MAX))
    );
    println!("  base address      {:#010x}", security.load_address());
    println!(
        "  image size        {:#x} ({})",
        security.image_size(),
        human_size(u64::from(security.image_size()))
    );

    match container.entry_point() {
        Some(entry) => println!("  entry point       {entry:#010x}"),
        None => println!("  entry point       not recorded"),
    }

    println!(
        "  encryption        {}",
        match container.encryption() {
            EncryptionType::None => "none".to_owned(),
            EncryptionType::Encrypted => "encrypted".to_owned(),
            EncryptionType::Unknown(value) => format!("unrecognized ({value})"),
        }
    );

    let compression = container.compression();
    let detail = match compression {
        CompressionType::Basic => {
            let blocks = container
                .file_format_info()
                .map_or(0, |info| info.basic_blocks().len());
            format!(" ({blocks} blocks)")
        }
        CompressionType::Unknown(value) => format!(" ({value})"),
        _ => String::new(),
    };
    let support = if compression.is_supported() {
        ""
    } else {
        "   [NOT SUPPORTED YET, decoding this file will fail]"
    };
    println!(
        "  compression       {}{detail}{support}",
        compression.name()
    );
}

/// Prints the section table with its address ranges and permissions.
fn print_sections(sections: &[Section]) {
    println!("\n  sections ({})", sections.len());

    for section in sections {
        let permissions = section.permissions().map_or_else(
            || "???".to_owned(),
            |allowed| {
                let mut flags = String::new();
                flags.push(if allowed.read { 'r' } else { '-' });
                flags.push(if allowed.write { 'w' } else { '-' });
                flags.push(if allowed.execute { 'x' } else { '-' });
                flags
            },
        );

        let kind = match section.kind {
            PageKind::Code => "code".to_owned(),
            PageKind::Data => "data".to_owned(),
            PageKind::ReadOnlyData => "rodata".to_owned(),
            PageKind::Unknown(value) => format!("unknown({value})"),
        };

        println!(
            "    {:#010x}..{:#010x}  {:>9}  {:<8} {permissions}",
            section.start,
            section.end(),
            human_size(u64::from(section.size)),
            kind
        );
    }
}

/// Prints the libraries the image imports from.
fn print_imports(container: &Container<'_>) {
    let libraries = container.import_libraries();
    println!("\n  imports ({})", libraries.len());

    for library in libraries {
        println!(
            "    {:<16} {:>4} imports   version {} (min {})",
            library.name,
            library.imports.len(),
            library.version,
            library.min_version
        );
    }
}

/// Decodes the image.
fn decode(args: &Args, container: &Container<'_>) -> Result<Image> {
    let key = keys::resolve(args.key_file.as_deref())?;

    if container.encryption() == EncryptionType::Encrypted && key.is_none() {
        return Err(miette!(
            help = keys::sources_consulted(args.key_file.as_deref()),
            "{} is encrypted and no key material was found",
            args.file.display()
        ));
    }

    container
        .load(key.as_ref())
        .into_diagnostic()
        .wrap_err_with(|| format!("decoding the image of {}", args.file.display()))
}

/// Reports what came out of decoding.
fn print_decoded(image: &Image) {
    println!("\n  decoded");
    println!(
        "    size            {}",
        human_size(u64::try_from(image.size()).unwrap_or(u64::MAX))
    );
    println!("    base address    {:#010x}", image.base_address());
    println!(
        "    executable      {} sections",
        image.executable_sections().count()
    );
}

/// Lists what every import record names.
fn print_import_records(container: &Container<'_>, image: &Image) -> Result<()> {
    let found = imports(image, container.import_libraries())
        .into_diagnostic()
        .wrap_err("reading the import records")?;

    let mut counts: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for import in &found {
        let entry = counts.entry(import.library).or_default();
        match import.kind {
            ImportKind::Thunk => entry.0 += 1,
            ImportKind::Slot => entry.1 += 1,
        }
    }

    println!("\n  import records ({})", found.len());
    for (library, (thunks, slots)) in &counts {
        println!("    {library:<16} {thunks:>4} thunks   {slots:>4} slots");
    }

    println!();
    for import in &found {
        let kind = match import.kind {
            ImportKind::Thunk => "thunk",
            ImportKind::Slot => "slot",
        };
        println!(
            "    {:#010x}  {kind:<6} {:<16} ordinal {}",
            import.address, import.library, import.ordinal
        );
    }

    Ok(())
}

/// Renders a byte count in the largest unit that keeps it readable.
fn human_size(bytes: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("MiB", 1 << 20), ("KiB", 1 << 10), ("B", 1)];

    for (unit, scale) in UNITS {
        if bytes >= scale {
            if scale == 1 {
                return format!("{bytes} {unit}");
            }
            let whole = bytes / scale;
            let tenths = (bytes % scale) * 10 / scale;
            return format!("{whole}.{tenths} {unit}");
        }
    }

    format!("{bytes} B")
}
