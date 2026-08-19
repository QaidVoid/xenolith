//! Checking the semantic model against an independently produced corpus.
//!
//! A companion project emitted C++ for the same titles, one statement group per
//! instruction, each preceded by a comment holding that instruction's
//! disassembly. Its output for one title runs to roughly 1.5 million
//! instructions across 25,000 functions, which is more than any hand written
//! test will ever cover and was produced by someone solving the same problem a
//! different way.
//!
//! What it can settle is which registers an instruction touches. What it cannot
//! settle is whether the arithmetic between them is right, because comparing two
//! expressions for equivalence is harder than the problem being solved. That
//! limit is why this file counts disagreements about reads and writes and says
//! nothing about values.
//!
//! No game data is committed. The corpus and the image are supplied through the
//! environment and every test here skips when they are absent.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use xenolith_lift::{Location, effect_of, is_modelled};
use xenolith_ppc::{Instruction, Opcode};
use xenolith_xex::{Container, KeyMaterial};

/// One instruction as the corpus recorded it.
#[derive(Debug)]
struct Emitted {
    address: u32,
    /// Whether the corpus stated this address rather than it being counted.
    ///
    /// A function header and a label both give an address outright. Everything
    /// between them is reached by adding four per instruction, which is only
    /// right if every instruction was emitted. Knowing which is which is what
    /// lets a drift be contained instead of producing wrong comparisons until
    /// the next label happens to arrive.
    stated: bool,
    /// Whether this is the first instruction of a function.
    starts_a_function: bool,
    mnemonic: String,
    reads: BTreeSet<Location>,
    writes: BTreeSet<Location>,
}

/// Returns the corpus directory, if one was supplied.
fn corpus_directory() -> Option<PathBuf> {
    std::env::var_os("XENOLITH_LIFT_CORPUS").map(PathBuf::from)
}

/// Parses a place the corpus names into one this crate knows.
///
/// The condition fields have to be tried before the general purpose registers,
/// since `cr6` starts with a letter that also begins a register name.
fn location_of(name: &str) -> Option<Location> {
    let number = |text: &str| text.parse::<u8>().ok();

    if let Some(rest) = name.strip_prefix("cr") {
        return Some(Location::Condition(number(rest)?));
    }
    if let Some(rest) = name.strip_prefix('r') {
        return Some(Location::General(number(rest)?));
    }
    if let Some(rest) = name.strip_prefix('f') {
        return Some(Location::Floating(number(rest)?));
    }
    if let Some(rest) = name.strip_prefix('v') {
        return Some(Location::Vector(number(rest)?));
    }
    match name {
        "lr" => Some(Location::Link),
        "ctr" => Some(Location::Count),
        "xer" => Some(Location::Exception),
        _ => None,
    }
}

/// Returns every place a statement names, in the order they appear.
fn places(statement: &str) -> Vec<(usize, Location)> {
    let mut found = Vec::new();
    let mut rest = statement;
    let mut consumed = 0;

    while let Some(at) = rest.find("ctx.") {
        let after = &rest[at + 4..];
        let end = after
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(after.len());
        if let Some(location) = location_of(&after[..end]) {
            found.push((consumed + at, location));
        }
        consumed += at + 4;
        rest = after;
    }

    found
}

/// Returns whether a statement assigns to the first place it names.
///
/// An assignment is the only way the corpus writes a register outright. A
/// comparison writes through a method call instead, which is handled separately
/// because it names its destination the same way a read would.
fn assigns_first(statement: &str) -> bool {
    let trimmed = statement.trim_start();

    // A counted branch is lowered as a decrement in place, which writes the
    // register without an assignment anywhere for the test below to find.
    if trimmed.contains("--ctx.") || trimmed.contains("++ctx.") {
        return true;
    }

    let Some(rest) = trimmed.strip_prefix("ctx.") else {
        return false;
    };
    let end = rest.find([' ', '=']).unwrap_or(rest.len());
    let after = rest[end..].trim_start();
    after.starts_with('=') && !after.starts_with("==")
}

/// Reads one corpus file into the instructions it records.
fn parse(text: &str) -> Vec<Emitted> {
    let mut emitted = Vec::new();
    let mut address = None;
    let mut pending: Option<(String, u32, bool, bool)> = None;
    let mut stated = false;
    let mut starting = false;
    let mut reads = BTreeSet::new();
    let mut writes = BTreeSet::new();

    let finish = |pending: &mut Option<(String, u32, bool, bool)>,
                  reads: &mut BTreeSet<Location>,
                  writes: &mut BTreeSet<Location>,
                  emitted: &mut Vec<Emitted>| {
        if let Some((mnemonic, at, stated, starting)) = pending.take() {
            emitted.push(Emitted {
                address: at,
                stated,
                starts_a_function: starting,
                mnemonic,
                reads: std::mem::take(reads),
                writes: std::mem::take(writes),
            });
        }
    };

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("PPC_FUNC_IMPL(__imp__") {
            finish(&mut pending, &mut reads, &mut writes, &mut emitted);
            stated = true;
            starting = true;
            address = rest
                .rsplit_once("sub_")
                .and_then(|(_, tail)| tail.split(')').next())
                .and_then(|hex| u32::from_str_radix(hex, 16).ok());
            continue;
        }

        // A label states the address of what follows. Counting four bytes per
        // instruction would be enough only if every instruction were emitted,
        // and taking the stated address instead means a drift corrects itself
        // at the next label rather than silently comparing the wrong words.
        if let Some(rest) = line.strip_prefix("loc_") {
            if let Some(hex) = rest.strip_suffix(':') {
                if let Ok(at) = u32::from_str_radix(hex, 16) {
                    finish(&mut pending, &mut reads, &mut writes, &mut emitted);
                    address = Some(at);
                    stated = true;
                }
            }
            continue;
        }

        let Some(at) = address else { continue };

        if let Some(text) = line.trim_start().strip_prefix("// ") {
            finish(&mut pending, &mut reads, &mut writes, &mut emitted);
            let mnemonic = text.split_whitespace().next().unwrap_or("").to_owned();
            pending = Some((mnemonic, at, stated, starting));
            stated = false;
            starting = false;
            address = Some(at.saturating_add(4));
            continue;
        }

        if line.starts_with('}') {
            finish(&mut pending, &mut reads, &mut writes, &mut emitted);
            address = None;
            continue;
        }

        if pending.is_none() {
            continue;
        }

        if is_injected(line) {
            continue;
        }

        // A comparison writes the field it names first and reads the rest.
        let named = places(line);
        let comparing = line.contains(".compare<")
            || line.contains(".compare(")
            || line.contains(".setFromMask(");
        // A vector store writes through a pointer to its first argument, which
        // reads like any other mention of that register unless it is known that
        // the call stores rather than loads.
        let storing_through_a_pointer = line.contains("_mm_store_si128((simde__m128i*)ctx.");
        let written = if comparing || storing_through_a_pointer || assigns_first(line) {
            named.first().map(|(_, location)| *location)
        } else {
            None
        };

        for (_, location) in &named {
            if Some(*location) == written {
                writes.insert(*location);
            } else {
                reads.insert(*location);
            }
        }
        // An assignment naming its destination again on the right reads it too.
        if let Some(destination) = written {
            if named.iter().filter(|(_, l)| *l == destination).count() > 1 {
                reads.insert(destination);
            }
        }
    }

    finish(&mut pending, &mut reads, &mut writes, &mut emitted);
    emitted
}

/// Spellings the corpus uses for instructions this crate names differently.
///
/// An extended mnemonic is a convention for writing an instruction, not a
/// different instruction. The corpus writes many of them, so comparing raw
/// spellings would report hundreds of disagreements that are only disagreements
/// about names.
const ALIASES: &[(&str, &str)] = &[
    ("li", "addi"),
    ("lis", "addis"),
    ("la", "addi"),
    ("subi", "addi"),
    ("mr", "or"),
    ("not", "nor"),
    ("nop", "ori"),
    ("bl", "b"),
    ("blr", "bclr"),
    ("blrl", "bclr"),
    ("bctr", "bcctr"),
    ("bctrl", "bcctr"),
    ("cmpw", "cmp"),
    ("cmpd", "cmp"),
    ("cmpwi", "cmpi"),
    ("cmpdi", "cmpi"),
    ("cmplw", "cmpl"),
    ("cmpld", "cmpl"),
    ("cmplwi", "cmpli"),
    ("cmpldi", "cmpli"),
    ("clrlwi", "rlwinm"),
    ("rotlwi", "rlwinm"),
    ("slwi", "rlwinm"),
    ("srwi", "rlwinm"),
    ("inslwi", "rlwimi"),
    ("clrldi", "rldicl"),
    ("rotldi", "rldicl"),
    ("sldi", "rldicr"),
    ("srdi", "rldicl"),
    ("extldi", "rldicr"),
    ("extrdi", "rldicl"),
    ("mflr", "mfspr"),
    ("mfctr", "mfspr"),
    ("mfxer", "mfspr"),
    ("mtlr", "mtspr"),
    ("mtctr", "mtspr"),
    ("mtxer", "mtspr"),
    ("mtcr", "mtcrf"),
    ("lwsync", "sync"),
    ("ptesync", "sync"),
    ("dcbzl", "dcbz"),
    ("twlgei", "twi"),
    ("twllei", "twi"),
    ("tdlgei", "tdi"),
    ("tdllei", "tdi"),
    ("trap", "tw"),
    // A timing hint the console spells as an operation on a register with
    // itself, which is what it is encoded as.
    ("db16cyc", "or"),
];

/// Returns whether a statement was injected rather than lifted.
///
/// The corpus is one project's output, and that project inserts calls of its
/// own into functions it wants to observe. Such a call names registers, and
/// attributing them to the instruction it sits beside would report reads the
/// instruction does not perform. A lifted call is recognizable because it takes
/// the context and the memory base and nothing else.
fn is_injected(statement: &str) -> bool {
    let trimmed = statement.trim_start();
    let Some(open) = trimmed.find('(') else {
        return false;
    };
    let name = &trimmed[..open];

    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || name.is_empty() {
        return false;
    }
    if name.starts_with("PPC_") || name.starts_with("simde") || name.starts_with("__") {
        return false;
    }

    // A lifted call passes the context and the base. Anything else calling into
    // a name of its own is that project's, not the instruction's.
    !trimmed[open..].starts_with("(ctx, base)")
}

/// Returns the instruction a corpus spelling names.
///
/// The record bit is a suffix rather than a different instruction, so it is
/// stripped first. A conditional branch is spelled one way per condition, and
/// every such spelling begins with `b`, which is why they are recognized by
/// shape rather than listed.
fn underlying(spelling: &str) -> &str {
    let base = spelling.trim_end_matches('.').trim_end_matches(['+', '-']);

    if let Some((_, actual)) = ALIASES.iter().find(|(alias, _)| *alias == base) {
        return actual;
    }
    // Everything from beq through bdnzflr is a conditional branch spelled for
    // its condition, and there are far too many to list.
    if base.starts_with('b') && base.len() > 1 {
        if base.ends_with("lr") {
            return "bclr";
        }
        if base.ends_with("ctr") {
            return "bcctr";
        }
        return "bc";
    }
    base
}

/// Loads the image the corpus was produced from, if one was supplied.
macro_rules! supplied_image {
    () => {
        match std::env::var_os("XENOLITH_ANALYSIS_XEX") {
            Some(path) => {
                let bytes = std::fs::read(&path).expect("reading the analysis container");
                let container = Container::parse(&bytes).expect("parsing the container");
                let key = std::env::var("XENOLITH_XEX_KEY")
                    .ok()
                    .map(|text| KeyMaterial::from_hex(text.trim()).expect("the supplied key"));
                container.load(key.as_ref()).expect("decoding the image")
            }
            None => {
                eprintln!("skipping: XENOLITH_ANALYSIS_XEX is not set");
                return;
            }
        }
    };
}

/// Reads every corpus file, or skips the enclosing test.
macro_rules! supplied_corpus {
    () => {
        match corpus_directory() {
            Some(directory) => {
                let mut all = Vec::new();
                let entries = std::fs::read_dir(&directory).expect("reading the corpus directory");
                for entry in entries {
                    let path = entry.expect("a corpus entry").path();
                    if path.extension().is_some_and(|extension| extension == "cpp") {
                        let text = std::fs::read_to_string(&path).expect("reading a corpus file");
                        all.extend(parse(&text));
                    }
                }
                all
            }
            None => {
                eprintln!("skipping: XENOLITH_LIFT_CORPUS is not set");
                return;
            }
        }
    };
}

/// Returns whether the corpus can say anything useful about an instruction.
///
/// The corpus is one project's C, not a description of the instruction set, and
/// some instructions are lowered into forms that name different things than the
/// instruction touches. Comparing those would measure the lowering rather than
/// the model.
fn outside_the_oracle(instruction: Instruction, spelling: &str) -> bool {
    // Control transfer becomes the host language's control flow, which does not
    // name what the instruction consults. A return becomes a return and the
    // link register never appears; a trap is left out entirely.
    if matches!(
        instruction.opcode(),
        Opcode::B
            | Opcode::Bc
            | Opcode::Bclr
            | Opcode::Bcctr
            | Opcode::Sc
            | Opcode::Tw
            | Opcode::Twi
            | Opcode::Td
            | Opcode::Tdi
    ) {
        return true;
    }

    // Reading the condition register is lowered as an accumulation into the
    // destination, so the corpus lists it as read. The instruction does not
    // read it.
    if instruction.opcode() == Opcode::Mfcr {
        return true;
    }

    // The console spells a timing hint as a register combined with itself. That
    // does write the register, with the value it already held, and is modelled
    // as such. The corpus emits nothing for it instead. Both readings are
    // defensible and the difference is recorded here rather than resolved by
    // bending one of them.
    spelling == "db16cyc"
}

/// Reports how much of the corpus the model covers, in instructions and in
/// whole functions.
///
/// The function figure is the one that matters, because lifting is all or
/// nothing per function and a single unmodelled instruction blocks one. It stays
/// near zero until the common instructions are nearly all modelled, and reporting
/// only the instruction figure would make that look like progress it is not.
#[test]
fn reports_how_much_of_the_corpus_is_modelled() {
    let image = supplied_image!();
    let emitted = supplied_corpus!();

    let mut instructions = 0u64;
    let mut modelled = 0u64;
    let mut blocking: BTreeMap<&'static str, u64> = BTreeMap::new();

    // A function is lifted whole or not at all, so what matters is how many hold
    // only instructions that are modelled. Counting instructions instead reads
    // far higher and means much less.
    let mut functions = 0u64;
    let mut whole = 0u64;
    let mut complete = true;
    let mut blockers: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut blocked_by: BTreeMap<&'static str, u64> = BTreeMap::new();

    for record in &emitted {
        if record.starts_a_function {
            if functions > 0 && complete {
                whole += 1;
            }
            for mnemonic in blockers.keys() {
                *blocked_by.entry(mnemonic).or_default() += 1;
            }
            blockers.clear();
            functions += 1;
            complete = true;
        }

        let Ok(word) = image.u32(record.address) else {
            continue;
        };
        let instruction = Instruction::decode(word);
        instructions += 1;
        if is_modelled(instruction.opcode()) {
            modelled += 1;
        } else {
            complete = false;
            *blocking.entry(instruction.opcode().mnemonic()).or_default() += 1;
            *blockers.entry(instruction.opcode().mnemonic()).or_default() += 1;
        }
    }
    if functions > 0 && complete {
        whole += 1;
    }
    for mnemonic in blockers.keys() {
        *blocked_by.entry(mnemonic).or_default() += 1;
    }

    eprintln!("corpus instructions   {instructions:>10}");
    eprintln!(
        "  modelled            {modelled:>10}  ({} percent)",
        modelled * 100 / instructions.max(1)
    );

    eprintln!("corpus functions      {functions:>10}");
    eprintln!(
        "  wholly modelled     {whole:>10}  ({} percent)",
        whole * 100 / functions.max(1)
    );

    let mut ranked: Vec<(&str, u64)> = blocking.into_iter().collect();
    ranked.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    eprintln!("\nmost frequent unmodelled instructions");
    for (mnemonic, count) in ranked.iter().take(10) {
        eprintln!("  {mnemonic:<12} {count:>10}");
    }

    // The instruction that appears most is not the one to model next. The one
    // blocking the most functions is, and they are not the same list.
    let mut worst: Vec<(&str, u64)> = blocked_by.into_iter().collect();
    worst.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    eprintln!("\nunmodelled instructions blocking the most functions");
    for (mnemonic, count) in worst.iter().take(10) {
        eprintln!("  {mnemonic:<12} {count:>10}");
    }

    assert!(instructions > 0, "no corpus instruction matched the image");
}

/// Compares what the model says an instruction touches against what the corpus
/// says, for every instruction the model claims to describe.
#[test]
fn the_model_agrees_with_the_corpus_about_what_is_touched() {
    let image = supplied_image!();
    let emitted = supplied_corpus!();

    let mut checked = 0u64;
    let mut misnamed = 0u64;
    let mut misnamed_example = String::new();
    let mut naming: BTreeMap<(String, &'static str), u64> = BTreeMap::new();
    let mut disagreements: BTreeMap<&'static str, (u64, String)> = BTreeMap::new();

    // Once an address is wrong every address after it is wrong too, until the
    // corpus states one outright. Comparing across a drift reports semantic
    // disagreements that are really disagreements about which word is where, so
    // the run is abandoned at the first symptom and resumed at the next stated
    // address.
    let mut aligned = true;

    for record in &emitted {
        if record.stated {
            aligned = true;
        }
        let Ok(word) = image.u32(record.address) else {
            continue;
        };
        let instruction = Instruction::decode(word);

        // Where the corpus names a different instruction than we decoded, the
        // two are not describing the same thing. Both sides are normalized the
        // same way, because some instructions genuinely carry a dot in their
        // name rather than as a variant bit.
        if underlying(&record.mnemonic) != underlying(instruction.opcode().mnemonic()) {
            aligned = false;
            misnamed += 1;
            *naming
                .entry((record.mnemonic.clone(), instruction.opcode().mnemonic()))
                .or_default() += 1;
            if misnamed_example.is_empty() {
                misnamed_example = format!(
                    "{:#010x} corpus says {} and we decode {}",
                    record.address,
                    record.mnemonic,
                    instruction.opcode().mnemonic()
                );
            }
            continue;
        }

        if !aligned {
            continue;
        }

        if outside_the_oracle(instruction, &record.mnemonic) {
            continue;
        }

        let Some(effect) = effect_of(instruction) else {
            continue;
        };
        checked += 1;

        let ours_reads: BTreeSet<Location> = effect.reads().iter().copied().collect();
        let ours_writes: BTreeSet<Location> = effect.writes().iter().copied().collect();

        // Recording a result is lowered as a comparison against the register
        // just written, so the corpus lists that register as read. That is a
        // dependency inside the instruction rather than a register it reads.
        let mut theirs_reads = record.reads.clone();
        if effect.writes().contains(&Location::Condition(0)) {
            for written in effect.writes() {
                if matches!(written, Location::General(_)) {
                    theirs_reads.remove(written);
                    if record.reads.contains(written) && ours_reads.contains(written) {
                        theirs_reads.insert(*written);
                    }
                }
            }
        }

        if ours_reads == theirs_reads && ours_writes == record.writes {
            continue;
        }

        let entry = disagreements
            .entry(instruction.opcode().mnemonic())
            .or_insert_with(|| (0, String::new()));
        entry.0 += 1;
        if entry.1.is_empty() {
            entry.1 = format!(
                "{:#010x} {}\n      corpus reads {:?} writes {:?}\n      model  reads {:?} writes {:?}",
                record.address,
                record.mnemonic,
                theirs_reads,
                record.writes,
                ours_reads,
                ours_writes
            );
        }
    }

    eprintln!("instructions checked  {checked:>10}");
    eprintln!("named differently     {misnamed:>10}");
    if !misnamed_example.is_empty() {
        eprintln!("  first: {misnamed_example}");
    }
    let mut named: Vec<((String, &str), u64)> = naming.into_iter().collect();
    named.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for ((theirs, ours), count) in named.iter().take(15) {
        eprintln!("  corpus {theirs:<12} we decode {ours:<12} {count:>8}");
    }
    eprintln!("mnemonics disagreeing {:>10}", disagreements.len());

    let mut ranked: Vec<(&str, (u64, String))> = disagreements.into_iter().collect();
    ranked.sort_by_key(|(_, (count, _))| std::cmp::Reverse(*count));
    for (mnemonic, (count, example)) in ranked.iter().take(40) {
        eprintln!("\n  {mnemonic} disagreed {count} times, first at\n      {example}");
    }

    // A handful of records survive misaligned: the corpus is another project's
    // output rather than a description of the instruction set, and where it
    // emits something this reader does not expect, the addresses after it are
    // counted wrong until the next stated one. Those produce comparisons
    // between different instructions, which is a limit of the reading rather
    // than a claim about the model.
    //
    // The bound is there because a real mistake in the model is not subtle. Every
    // one found so far ran to tens of thousands of instructions, because an
    // instruction set is regular and a wrong rule is wrong everywhere it applies.
    let disagreeing: u64 = ranked.iter().map(|(_, (count, _))| count).sum();
    let allowed = checked / 10_000;
    eprintln!("instructions disagreeing {disagreeing:>7}  (allowed {allowed})");

    assert!(
        disagreeing <= allowed,
        "the model and the corpus disagree about {disagreeing} instructions, \
         which is more than misreading the corpus accounts for"
    );
}
