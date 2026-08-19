//! The analyze subcommand.
//!
//! Reports what analysis found: which functions were discovered and how, the
//! shape of their control flow graphs, the register save and restore helpers,
//! and the jump tables behind indirect branches.
//!
//! Every figure that can be incomplete is printed alongside what it is
//! incomplete against. Coverage is given against the executable words that
//! exist, and a branch whose table could not be read is counted and can be
//! listed. A number with nothing to measure it against is not worth printing.

use clap::Parser;
use miette::{Result, miette};
use xenolith_analysis::{
    Function, JumpTable, JumpTables, Origin, Program, analyze, detect, recover,
};
use xenolith_xex::Image;

use crate::input::Source;

/// Arguments of the analyze subcommand.
#[derive(Debug, Parser)]
pub(crate) struct Args {
    /// Where the image comes from.
    #[command(flatten)]
    pub(crate) source: Source,

    /// List every discovered function with its range and how it was found.
    #[arg(long)]
    pub(crate) functions: bool,

    /// List every recovered jump table with its targets.
    #[arg(long)]
    pub(crate) tables: bool,
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

/// Returns how many words of executable section the image holds.
fn executable_words(image: &Image) -> u64 {
    image
        .executable_sections()
        .map(|section| u64::from(section.size) / 4)
        .sum()
}

/// Prints the counts of functions, blocks, and edges.
fn report_discovery(program: &Program, words: u64) {
    let claimed = program.claimed_instructions();
    let blocks: usize = program.functions().map(|f| f.blocks.len()).sum();
    let edges: usize = program
        .functions()
        .map(|f| f.edges().values().map(Vec::len).sum::<usize>())
        .sum();
    let tail_calls: usize = program.functions().map(|f| f.tail_calls.len()).sum();

    println!("functions          {:>10}", program.function_count());
    println!(
        "  entry point      {:>10}",
        program.count_from(Origin::EntryPoint)
    );
    println!(
        "  exported         {:>10}",
        program.count_from(Origin::Root)
    );
    println!(
        "  called           {:>10}",
        program.count_from(Origin::Called)
    );
    println!(
        "  found by shape   {:>10}",
        program.count_from(Origin::Scanned)
    );
    println!("blocks             {blocks:>10}");
    println!("edges              {edges:>10}");
    println!("tail calls         {tail_calls:>10}");
    println!(
        "\nexecutable words   {words:>10}\nclaimed            {claimed:>10}  ({} percent)",
        percent(claimed, words)
    );
}

/// Prints the register save and restore helpers, and any that are missing.
fn report_helpers(image: &Image) {
    let helpers = detect(image);

    println!("\nhelpers            {:>10}", helpers.all().len());
    for helper in helpers.all() {
        println!(
            "  {:#010x}..{:#010x}  {:<16} {:<8} registers {}..{}",
            helper.start,
            helper.end,
            helper.kind.name(),
            helper.direction.name(),
            helper.first_register,
            helper.last_register
        );
    }
    for (kind, direction) in helpers.missing() {
        println!("  not found: {} {}", kind.name(), direction.name());
    }
}

/// Prints the jump table counts, and the tables themselves when asked.
///
/// Resolved branches are counted apart from recovered tables. A table whose
/// every target lies in another function is recovered and resolves nothing, so
/// summing the two would report a graph better connected than it is.
fn report_tables(program: &Program, tables: &JumpTables, list: bool) {
    let considered = tables.considered();
    let recovered = tables.recovered().len();
    let entries: usize = tables.recovered().iter().map(JumpTable::entries).sum();

    let resolved: usize = program
        .functions()
        .map(|function| function.resolved.len())
        .sum();
    let unresolved: usize = program
        .functions()
        .map(Function::unresolved_edge_count)
        .sum();

    println!("\nindirect branches  {considered:>10}");
    println!(
        "  resolved         {resolved:>10}  ({} percent)",
        percent(resolved as u64, considered as u64)
    );
    println!("  unresolved       {unresolved:>10}");
    println!("tables recovered   {recovered:>10}");
    println!("  not recovered    {:>10}", tables.unrecovered().len());
    println!("table entries      {entries:>10}");

    if !list {
        return;
    }

    for table in tables.recovered() {
        println!(
            "\n  branch {:#010x}  index r{}  table {:#010x}  default {}",
            table.branch,
            table.index_register,
            table.table.unwrap_or(0),
            table
                .default
                .map_or_else(|| "none".to_owned(), |address| format!("{address:#010x}"))
        );
        for (at, target) in table.targets.iter().enumerate() {
            println!("    {at:>4}  {target:#010x}");
        }
    }

    for branch in tables.unrecovered() {
        println!("\n  branch {branch:#010x}  not recovered");
    }
}

/// Runs the analyze subcommand.
///
/// # Errors
///
/// Returns an error when the input cannot be read or decoded, or when it holds
/// no executable words to analyze.
pub(crate) fn run(args: &Args) -> Result<()> {
    let image = args.source.load()?.image;

    let words = executable_words(&image);
    if words == 0 {
        return Err(miette!(
            help = "analysis reads code, and this image declares none",
            "the image holds no executable words"
        ));
    }

    let program = analyze(&image, &[]);
    report_discovery(&program, words);
    report_helpers(&image);

    let mut tables = JumpTables::default();
    for function in program.functions() {
        tables.absorb(recover(&image, function));
    }
    report_tables(&program, &tables, args.tables);

    if args.functions {
        println!("\nfunctions");
        for function in program.functions() {
            println!(
                "  {:#010x}..{:#010x}  {:<12} {:>5} blocks",
                function.start,
                function.end(),
                function.origin.name(),
                function.blocks.len()
            );
        }
    }

    Ok(())
}
