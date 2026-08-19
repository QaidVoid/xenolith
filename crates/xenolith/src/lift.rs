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
use std::path::PathBuf;

use clap::Parser;
use miette::{Context, IntoDiagnostic, Result, miette};
use xenolith_analysis::analyze;
use xenolith_lift::{RUNTIME_HEADER, declaration_of, lift};

use crate::input::Source;

/// Arguments of the lift subcommand.
#[derive(Debug, Parser)]
pub(crate) struct Args {
    /// Where the image comes from.
    #[command(flatten)]
    pub(crate) source: Source,

    /// Directory to write the emitted C and the runtime header into.
    #[arg(long, value_name = "PATH")]
    pub(crate) out: PathBuf,

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

/// Runs the lift subcommand.
///
/// # Errors
///
/// Returns an error when the input cannot be read, decoded, or analyzed, or
/// when the output directory cannot be written. Functions that could not be
/// lifted are reported rather than treated as a failure.
pub(crate) fn run(args: &Args) -> Result<()> {
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
    std::fs::write(args.out.join("xenolith.h"), RUNTIME_HEADER)
        .into_diagnostic()
        .wrap_err_with(|| format!("writing the runtime header into {}", args.out.display()))?;

    let mut lifted = 0u64;
    let mut refused = Vec::new();
    let mut blocking: BTreeMap<&str, u64> = BTreeMap::new();
    let mut referenced: BTreeSet<u32> = BTreeSet::new();
    let mut body = String::new();

    for function in program.functions() {
        referenced.insert(function.start);
        match lift(&image, function) {
            Ok(result) => {
                lifted += 1;
                referenced.extend(result.calls);
                body.push_str(&result.code);
                body.push('\n');
            }
            Err(unlifted) => {
                *blocking.entry(unlifted.mnemonic).or_default() += 1;
                refused.push(unlifted);
            }
        }
    }

    // Everything the emitted code names is declared, which is more than the
    // discovered functions: a call into a register save helper lands partway
    // through one, and discovery does not claim those.
    let mut source = String::from("#include \"xenolith.h\"\n\n");
    for address in &referenced {
        source.push_str(&declaration_of(*address));
    }
    source.push('\n');
    source.push_str(&body);

    let path = args.out.join("lifted.c");
    std::fs::write(&path, &source)
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
    println!("\nwritten to {}", args.out.display());
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
