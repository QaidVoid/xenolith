//! The lift subcommand.
//!
//! Emits C for the functions analysis found, alongside the interface that C is
//! written against. Some functions will not lift, which is the expected outcome
//! rather than a failure, so the report is the deliverable and the subcommand
//! succeeds either way.
//!
//! What it does not do is claim the result can run. The interface says what the
//! emitted code needs; nothing here provides it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use clap::Parser;
use miette::{Context, IntoDiagnostic, Result, miette};
use xenolith_analysis::analyze;
use xenolith_lift::{RUNTIME_HEADER, declaration_of, lift};

use crate::input::{Source, number};

/// Bytes of C a translation unit holds before the next function starts a new
/// one.
///
/// A retail title emits hundreds of megabytes, which as one unit takes minutes
/// to compile and cannot be built in parallel at all, since a translation unit
/// is the only unit of parallelism a C build has.
const DEFAULT_PART_SIZE: u64 = 4 * 1024 * 1024;

/// Header the emitted units include, holding every declaration.
const DECLARATIONS: &str = "lifted.h";

/// Arguments of the lift subcommand.
#[derive(Debug, Parser)]
pub(crate) struct Args {
    /// Where the image comes from.
    #[command(flatten)]
    pub(crate) source: Source,

    /// Directory to write the emitted C and the runtime header into.
    #[arg(long, value_name = "PATH")]
    pub(crate) out: PathBuf,

    /// Bytes of C per translation unit, before the next function starts one.
    #[arg(long, value_name = "BYTES")]
    pub(crate) part_size: Option<String>,

    /// List the instructions that stopped functions, most blocking first.
    #[arg(long)]
    pub(crate) blockers: bool,

    /// List every function that was not lifted, with what stopped it.
    #[arg(long)]
    pub(crate) unlifted: bool,
}

/// Returns a percentage with three decimal places, avoiding a division by zero.
fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "0.000".to_owned();
    }
    format!(
        "{}.{:03}",
        part * 100 / whole,
        (part * 100_000 / whole) % 1000
    )
}

/// Collects emitted functions into translation units of a bounded size.
///
/// A unit is written as soon as it passes its budget rather than at the end, so
/// that emitting a title holds one unit in memory rather than the whole
/// program.
struct Units {
    /// Directory the units are written into.
    directory: PathBuf,
    /// Bytes a unit reaches before the next function starts a new one.
    budget: u64,
    /// Text of the unit being filled, empty when no unit is open.
    text: String,
    /// Address of the first function in the unit being filled.
    first: u32,
    /// Address of the last function added to the unit being filled.
    last: u32,
    /// Size of each unit written so far, in the order they were written.
    written: Vec<u64>,
}

impl Units {
    /// Starts collecting into a directory.
    fn new(directory: &Path, budget: u64) -> Self {
        Self {
            directory: directory.to_path_buf(),
            budget,
            text: String::new(),
            first: 0,
            last: 0,
            written: Vec::new(),
        }
    }

    /// Adds one function's code, writing the open unit first when it is full.
    ///
    /// The budget is checked between functions, so a unit ends slightly over it
    /// rather than part way through a body.
    fn push(&mut self, address: u32, code: &str) -> Result<()> {
        if self.text.is_empty() {
            self.first = address;
        } else if self.text.len() as u64 >= self.budget {
            self.write()?;
            self.first = address;
        }

        self.last = address;
        self.text.push_str(code);
        self.text.push('\n');
        Ok(())
    }

    /// Writes the open unit, naming it after the addresses it covers.
    fn write(&mut self) -> Result<()> {
        if self.text.is_empty() {
            return Ok(());
        }

        let name = format!("lifted.{:08x}-{:08x}.c", self.first, self.last);
        let path = self.directory.join(&name);
        let source = format!(
            "/* Emitted by xenolith: functions {:#010x} through {:#010x}. */\n\n\
             #include \"{DECLARATIONS}\"\n\n{}",
            self.first, self.last, self.text
        );

        std::fs::write(&path, &source)
            .into_diagnostic()
            .wrap_err_with(|| format!("writing {}", path.display()))?;

        self.written.push(source.len() as u64);
        self.text.clear();
        Ok(())
    }

    /// Writes whatever is still open and reports the sizes written.
    fn finish(mut self) -> Result<Vec<u64>> {
        self.write()?;
        Ok(self.written)
    }
}

/// Removes units a previous run wrote into the same directory.
///
/// Only the emitted naming is matched. A unit left behind by a run that split
/// differently would define the same functions a second time, so leaving it
/// would produce a directory that cannot be built.
fn clear_units(directory: &Path) -> Result<usize> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Ok(0);
    };

    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let named = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("lifted."));
        if named && path.extension().is_some_and(|extension| extension == "c") {
            std::fs::remove_file(entry.path())
                .into_diagnostic()
                .wrap_err_with(|| format!("removing {}", entry.path().display()))?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Returns the makefile that builds the emitted units.
///
/// It stops at an archive. Nothing implements the runtime interface the units
/// are written against, so a link step would fail and reporting that failure as
/// the build's outcome would say less than it seems to.
fn makefile() -> String {
    format!(
        "# Emitted by xenolith. Run with -j to build the units in parallel.\n\
         #\n\
         # This stops at an archive rather than a link. The units call a runtime\n\
         # that xenolith declares and does not implement, so there is nothing to\n\
         # link them against yet.\n\
         \n\
         CC ?= cc\n\
         AR ?= ar\n\
         CFLAGS ?= -O2 -Wall -Wextra -Wno-infinite-recursion\n\
         \n\
         SOURCES := $(wildcard lifted.*.c)\n\
         OBJECTS := $(SOURCES:.c=.o)\n\
         \n\
         all: liblifted.a\n\
         \n\
         liblifted.a: $(OBJECTS)\n\
         \t$(AR) rcs $@ $(OBJECTS)\n\
         \n\
         %.o: %.c {DECLARATIONS} xenolith.h\n\
         \t$(CC) $(CFLAGS) -c -o $@ $<\n\
         \n\
         clean:\n\
         \trm -f $(OBJECTS) liblifted.a\n\
         \n\
         .PHONY: all clean\n"
    )
}

/// Runs the lift subcommand.
///
/// # Errors
///
/// Returns an error when the input cannot be read, decoded, or analyzed, or
/// when the output directory cannot be written. Functions that could not be
/// lifted are reported rather than treated as a failure.
pub(crate) fn run(args: &Args) -> Result<()> {
    let budget = match &args.part_size {
        Some(text) => u64::from(number(text)?),
        None => DEFAULT_PART_SIZE,
    };
    if budget == 0 {
        return Err(miette!(
            help = "a unit has to hold at least one function",
            "a part size of zero would write a unit per byte"
        ));
    }

    let image = args.source.load()?;
    let program = analyze(&image, &[]);

    if program.function_count() == 0 {
        return Err(miette!(
            help = "lifting needs functions, and analysis found none",
            "nothing was discovered to lift"
        ));
    }

    std::fs::create_dir_all(&args.out)
        .into_diagnostic()
        .wrap_err_with(|| format!("making {}", args.out.display()))?;
    clear_units(&args.out)?;
    std::fs::write(args.out.join("xenolith.h"), RUNTIME_HEADER)
        .into_diagnostic()
        .wrap_err_with(|| format!("writing the runtime header into {}", args.out.display()))?;

    let mut lifted = 0u64;
    let mut refused = Vec::new();
    let mut blocking: BTreeMap<&str, u64> = BTreeMap::new();
    let mut referenced: BTreeSet<u32> = BTreeSet::new();
    let mut units = Units::new(&args.out, budget);

    for function in program.functions() {
        referenced.insert(function.start);
        match lift(&image, function) {
            Ok(result) => {
                lifted += 1;
                referenced.extend(result.calls);
                units.push(function.start, &result.code)?;
            }
            Err(unlifted) => {
                *blocking.entry(unlifted.mnemonic).or_default() += 1;
                refused.push(unlifted);
            }
        }
    }

    let sizes = units.finish()?;

    // Everything the emitted code names is declared, which is more than the
    // discovered functions: a call into a register save helper lands partway
    // through one, and discovery does not claim those. Which of them are named
    // is only known once every function has been lifted, so the header is
    // written after the units that include it.
    let mut header = String::from(
        "/* Emitted by xenolith. Every function is written under the\n\
         \x20* instructions it came from, so the two can be read against each\n\
         \x20* other.\n\
         \x20*\n\
         \x20* A guest function that recurses on every path is a statement about\n\
         \x20* the program that was translated rather than about the\n\
         \x20* translation, so a compiler warning that reads intent has none to\n\
         \x20* read here. Build with -Wno-infinite-recursion.\n\
         \x20*/\n\n\
         #ifndef XENOLITH_LIFTED_H\n\
         #define XENOLITH_LIFTED_H\n\n\
         #include \"xenolith.h\"\n\n",
    );
    for address in &referenced {
        header.push_str(&declaration_of(*address));
    }
    header.push_str("\n#endif\n");

    let path = args.out.join(DECLARATIONS);
    std::fs::write(&path, &header)
        .into_diagnostic()
        .wrap_err_with(|| format!("writing {}", path.display()))?;

    let path = args.out.join("Makefile");
    std::fs::write(&path, makefile())
        .into_diagnostic()
        .wrap_err_with(|| format!("writing {}", path.display()))?;

    let total = lifted + refused.len() as u64;
    println!("functions          {total:>10}");
    println!(
        "  lifted           {lifted:>10}  ({} percent)",
        percent(lifted, total)
    );
    println!("  not lifted       {:>10}", refused.len());
    println!("declarations       {:>10}", referenced.len());
    println!("units              {:>10}", sizes.len());
    println!(
        "  largest          {:>10} bytes",
        sizes.iter().copied().max().unwrap_or(0)
    );
    println!("\nwritten to {}", args.out.display());
    println!("build it with make -j, which stops at an archive");
    println!("nothing here can run it yet, since the interface is a declaration");

    if args.blockers {
        // The instruction that appears most is not the one to model next. The
        // one blocking the most functions is, and they are not the same list.
        let mut ranked: Vec<(&str, u64)> = blocking.into_iter().collect();
        ranked.sort_by_key(|(_, count)| std::cmp::Reverse(*count));

        println!("\ninstructions blocking the most functions");
        for (mnemonic, count) in ranked.iter().take(20) {
            println!("  {mnemonic:<14} {count:>8}");
        }
    }

    if args.unlifted {
        println!("\nfunctions that were not lifted");
        for unlifted in &refused {
            println!(
                "  {:#010x}  stopped at {:#010x} on {}",
                unlifted.function, unlifted.address, unlifted.mnemonic
            );
        }
    }

    Ok(())
}
