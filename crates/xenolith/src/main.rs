//! Command line front end for the xenolith static recompiler.

mod inspect;
mod keys;

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
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Inspect(args) => inspect::run(&args),
    }
}
