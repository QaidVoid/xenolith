//! Command line front end for the xenolith static recompiler.

mod analyze;
mod disasm;
mod input;
mod inspect;
mod keys;
mod lift;

use clap::{Parser, Subcommand};
use miette::Result;

/// Turns Xbox 360 executables into native code.
#[derive(Debug, Parser)]
#[command(name = "xenolith", version, about, long_about = None)]
struct Cli {
    /// The operation to perform.
    #[command(subcommand)]
    command: Command,
}

/// Operations the tool supports.
#[derive(Debug, Subcommand)]
enum Command {
    /// Summarize an Xbox 360 executable.
    Inspect(inspect::Args),
    /// Disassemble a range of an Xbox 360 executable.
    Disasm(disasm::Args),
    /// Report the functions, blocks, and jump tables of an executable.
    Analyze(analyze::Args),
    /// Emit C for the functions of an executable.
    Lift(lift::Args),
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Inspect(args) => inspect::run(&args),
        Command::Disasm(args) => disasm::run(&args),
        Command::Analyze(args) => analyze::run(&args),
        Command::Lift(args) => lift::run(&args),
    }
}
