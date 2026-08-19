//! Function discovery against a real title.
//!
//! Discovery reaches what direct calls reach. Anything called only through a
//! pointer is invisible to it, which in compiled game code means everything
//! behind a virtual table. What this measures is how much of a real code
//! section that leaves unclaimed, which is the number that says whether a
//! second pass is worth building and how much it has to find.
//!
//! No game data is committed. The image is supplied through the environment and
//! the test skips when it is absent.

use std::path::PathBuf;

use xenolith_analysis::{JumpTable, Origin, analyze};
use xenolith_xex::{Container, Image, KeyMaterial, PageKind, Section};

/// Returns the image path, base address, and entry point, if all were supplied.
fn supplied_source() -> Option<(PathBuf, u32, Option<u32>)> {
    let path = PathBuf::from(std::env::var_os("XENOLITH_ANALYSIS_IMAGE")?);
    let number = |name: &str| {
        std::env::var(name)
            .ok()
            .and_then(|text| u32::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok())
    };

    Some((
        path,
        number("XENOLITH_ANALYSIS_BASE")?,
        number("XENOLITH_ANALYSIS_ENTRY"),
    ))
}

/// Reads the section layout, if one was supplied.
///
/// Jump tables live in read only data rather than in the code section, so
/// recovery cannot be measured against the code section alone. The layout is
/// given as comma separated `start:end:kind` triples in hexadecimal, which
/// keeps the addresses of a real title out of the repository.
fn supplied_sections(base: u32, size: u32) -> Vec<Section> {
    let Ok(text) = std::env::var("XENOLITH_ANALYSIS_SECTIONS") else {
        return vec![Section {
            start: base,
            size,
            kind: PageKind::Code,
        }];
    };

    let end_of_image = base.saturating_add(size);
    text.split(',')
        .filter_map(|part| {
            let mut fields = part.trim().split(':');
            let number = |field: Option<&str>| {
                u32::from_str_radix(field?.trim().trim_start_matches("0x"), 16).ok()
            };
            let start = number(fields.next())?;
            let end = number(fields.next())?.min(end_of_image);
            let kind = match fields.next()?.trim() {
                "code" => PageKind::Code,
                "data" => PageKind::Data,
                "rodata" => PageKind::ReadOnlyData,
                _ => PageKind::Unknown(0),
            };
            Some(Section {
                start,
                size: end.checked_sub(start)?,
                kind,
            })
        })
        .collect()
}

/// Builds an image over the supplied bytes and section layout.
fn image_of(bytes: Vec<u8>, base: u32, entry: Option<u32>) -> Image {
    let size = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    let sections = supplied_sections(base, size);
    Image::new(base, bytes, sections).with_entry_point(entry)
}

/// Returns how many words of executable section the image holds.
fn executable_words(image: &Image) -> u64 {
    image
        .sections()
        .iter()
        .filter(|section| section.kind.is_executable())
        .map(|section| u64::from(section.size) / 4)
        .sum()
}

/// Loads the supplied image, or skips the enclosing test.
///
/// A container is preferred when one was given, because it describes itself.
macro_rules! supplied_image {
    () => {
        match std::env::var_os("XENOLITH_ANALYSIS_XEX") {
            Some(path) => {
                // A container carries its own section layout and entry point,
                // so neither has to be described by hand and neither can be
                // described wrongly. Decoding one needs key material, supplied
                // through the environment rather than embedded.
                let bytes = std::fs::read(&path).expect("reading the analysis container");
                let container = Container::parse(&bytes).expect("parsing the container");
                let key = std::env::var("XENOLITH_XEX_KEY")
                    .ok()
                    .map(|text| KeyMaterial::from_hex(text.trim()).expect("the supplied key"));
                let image = container.load(key.as_ref()).expect("decoding the image");
                let words = executable_words(&image);
                (image, words)
            }
            None => match supplied_source() {
                Some((path, base, entry)) => {
                    let bytes = std::fs::read(&path).expect("reading the analysis image");
                    let image = image_of(bytes, base, entry);
                    let words = executable_words(&image);
                    (image, words)
                }
                None => {
                    eprintln!("skipping: no analysis image was supplied");
                    return;
                }
            },
        }
    };
}

#[test]
fn reports_what_direct_calls_reach() {
    let (image, words) = supplied_image!();

    let program = analyze(&image, &[]);
    let claimed = program.claimed_instructions();

    eprintln!("functions      {:>10}", program.function_count());
    eprintln!(
        "  entry point  {:>10}",
        program.count_from(Origin::EntryPoint)
    );
    eprintln!("  called       {:>10}", program.count_from(Origin::Called));
    eprintln!("  root         {:>10}", program.count_from(Origin::Root));
    eprintln!("words          {words:>10}");
    eprintln!(
        "claimed        {claimed:>10}  ({}.{:03} percent)",
        claimed * 100 / words.max(1),
        (claimed * 100_000 / words.max(1)) % 1000
    );

    let unresolved = program
        .functions()
        .filter(|f| f.has_unresolved_transfer())
        .count();
    let tail_calls: usize = program.functions().map(|f| f.tail_calls.len()).sum();
    eprintln!("functions with an unresolved transfer {unresolved}");
    eprintln!("tail calls     {tail_calls:>10}");

    assert!(
        program.function_count() > 0,
        "nothing was discovered, so the entry point was not usable"
    );
    assert!(
        claimed <= words,
        "more instructions were claimed than the section holds"
    );
}

/// Functions may share code, but an instruction claimed by two of them must
/// still only be counted once, or coverage would read above what exists.
#[test]
fn claimed_instructions_never_exceed_the_section() {
    let (image, words) = supplied_image!();

    let program = analyze(&image, &[]);

    assert!(program.claimed_instructions() <= words);
}

/// Reports the shape of the graphs discovery produced, so a figure that looks
/// wrong can be investigated rather than assumed.
#[test]
fn reports_graph_shape() {
    let (image, _) = supplied_image!();
    let program = analyze(&image, &[]);

    let blocks: usize = program.functions().map(|f| f.blocks.len()).sum();
    let edges: usize = program
        .functions()
        .map(|f| f.edges().values().map(Vec::len).sum::<usize>())
        .sum();
    let unresolved: usize = program
        .functions()
        .map(xenolith_analysis::Function::unresolved_edge_count)
        .sum();
    let unreachable: usize = program
        .functions()
        .map(|f| f.unreachable_blocks().len())
        .sum();

    eprintln!("blocks         {blocks:>10}");
    eprintln!("edges          {edges:>10}");
    eprintln!("unresolved     {unresolved:>10}");
    eprintln!("unreachable    {unreachable:>10}");

    // A block nothing reaches means the walk pulled in code belonging to
    // somebody else, which is how a call into a shared helper once behaved.
    // One example is printed so the cause can be looked at rather than guessed.
    for function in program.functions() {
        let orphans = function.unreachable_blocks();
        if orphans.is_empty() {
            continue;
        }
        eprintln!(
            "\nfunction {:#010x} has {} unreachable",
            function.start,
            orphans.len()
        );
        for block in &function.blocks {
            let mark = if orphans.iter().any(|o| o.start == block.start) {
                "  <-- unreachable"
            } else {
                ""
            };
            eprintln!(
                "  block {:#010x}..{:#010x}  {:?}{mark}",
                block.start, block.end, block.terminator
            );
        }
        break;
    }

    assert_eq!(
        unreachable, 0,
        "discovery only walks what it reaches, so nothing should be unreachable"
    );
}

/// Reports jump table recovery, and checks one table worked out by hand.
///
/// The hand worked example is at a branch whose setup reads: compare an index
/// against fifteen, take a default when out of range, build a table address,
/// load a byte from it, multiply that by four, add it to a base, and branch.
/// Sixteen entries, and every target inside the function it branches from.
///
/// Reading the first entry by hand gives `0x82109408 + 0x73 * 4`, which lands
/// four bytes below the default. That is what a switch looks like, and it is
/// the arithmetic recovery has to reproduce.
/// A table someone worked out by hand, supplied per title.
struct Expected {
    branch: u32,
    index_register: u8,
    table: u32,
    entries: usize,
    default: u32,
    first_target: u32,
}

/// Reads the hand worked table, if one was supplied for this title.
///
/// Given as colon separated hexadecimal fields: the branch, the index register,
/// the table address, the entry count, the default, and the first target. It is
/// supplied rather than written in, because the harness serves more than one
/// title and an address from one of them means nothing to another.
fn expected_table() -> Option<Expected> {
    let text = std::env::var("XENOLITH_ANALYSIS_TABLE").ok()?;
    let mut fields = text.split(':');
    let mut number =
        || u32::from_str_radix(fields.next()?.trim().trim_start_matches("0x"), 16).ok();

    Some(Expected {
        branch: number()?,
        index_register: u8::try_from(number()?).ok()?,
        table: number()?,
        entries: usize::try_from(number()?).ok()?,
        default: number()?,
        first_target: number()?,
    })
}

#[test]
fn reports_jump_table_recovery() {
    let (image, _) = supplied_image!();
    let program = analyze(&image, &[]);

    let mut all = xenolith_analysis::JumpTables::default();
    for function in program.functions() {
        all.absorb(xenolith_analysis::recover(&image, function));
    }

    let considered = all.considered();
    let recovered = all.recovered().len();
    eprintln!("indirect jumps considered {considered:>8}");
    eprintln!("tables recovered          {recovered:>8}");
    eprintln!("not recovered             {:>8}", all.unrecovered().len());
    if let Some(rate) = (recovered * 100).checked_div(considered) {
        eprintln!("recovery rate             {rate:>7}%");
    }

    let sizes: Vec<usize> = all.recovered().iter().map(JumpTable::entries).collect();
    if !sizes.is_empty() {
        let total: usize = sizes.iter().sum();
        eprintln!(
            "entries: total {total}, smallest {}, largest {}",
            sizes.iter().min().unwrap(),
            sizes.iter().max().unwrap()
        );
    }

    let Some(expected) = expected_table() else {
        eprintln!("skipping the worked example: XENOLITH_ANALYSIS_TABLE is not set");
        return;
    };

    let table = all
        .recovered()
        .iter()
        .find(|table| table.branch == expected.branch)
        .expect("the hand worked example was not recovered");

    eprintln!("\nthe hand worked example at {:#010x}", table.branch);
    eprintln!("  index register  r{}", table.index_register);
    eprintln!("  table           {:#010x}", table.table.unwrap_or(0));
    eprintln!("  entries         {}", table.entries());
    eprintln!("  default         {:#010x}", table.default.unwrap_or(0));

    assert_eq!(table.index_register, expected.index_register);
    assert_eq!(table.table, Some(expected.table));
    assert_eq!(table.entries(), expected.entries);
    assert_eq!(table.default, Some(expected.default));
    assert_eq!(table.targets.first(), Some(&expected.first_target));
}

/// A table read out of the reference file.
#[derive(Debug, PartialEq, Eq)]
struct Reference {
    register: u8,
    default: u32,
    labels: Vec<u32>,
}

/// Reads the tables an existing tool produced for the same title.
///
/// The file is a list of records holding an index register, a default target,
/// and the labels. Only those three fields are read, because they are the ones
/// that can be compared: the address each record is keyed by refers to where
/// that tool considered the switch to begin, which is not the same place.
fn reference_tables(text: &str) -> Vec<Reference> {
    let number = |line: &str| {
        let value = line.split('=').next_back().unwrap_or(line);
        u32::from_str_radix(
            value.trim().trim_end_matches(',').trim_start_matches("0x"),
            16,
        )
        .ok()
    };

    let mut tables = Vec::new();
    let mut register = None;
    let mut default = None;
    let mut labels = Vec::new();
    let mut in_labels = false;

    for line in text.lines().map(str::trim) {
        if line == "[[switch]]" {
            if let (Some(register), Some(default)) = (register.take(), default.take()) {
                tables.push(Reference {
                    register,
                    default,
                    labels: std::mem::take(&mut labels),
                });
            }
            labels.clear();
            in_labels = false;
        } else if let Some(rest) = line.strip_prefix("r = ") {
            register = rest.trim().parse().ok();
        } else if line.starts_with("default = ") {
            default = number(line);
        } else if line.starts_with("labels = [") {
            in_labels = true;
        } else if line == "]" {
            in_labels = false;
        } else if in_labels {
            if let Some(label) = number(line) {
                labels.push(label);
            }
        }
    }

    if let (Some(register), Some(default)) = (register, default) {
        tables.push(Reference {
            register,
            default,
            labels,
        });
    }
    tables
}

/// Compares recovery against the tables an existing tool produced.
///
/// This is the check that says whether recovery is right rather than merely
/// productive. The comparison is on the targets and the default, because a
/// table that agrees on those is the same table however it was keyed.
#[test]
fn recovered_tables_agree_with_the_reference() {
    let (image, _) = supplied_image!();
    let Ok(path) = std::env::var("XENOLITH_ANALYSIS_SWITCHES") else {
        eprintln!("skipping: XENOLITH_ANALYSIS_SWITCHES is not set");
        return;
    };
    let text = std::fs::read_to_string(&path).expect("reading the reference tables");
    let reference = reference_tables(&text);

    let program = analyze(&image, &[]);
    let mut all = xenolith_analysis::JumpTables::default();
    for function in program.functions() {
        all.absorb(xenolith_analysis::recover(&image, function));
    }

    let mut agreed = 0;
    let mut disagreed = Vec::new();
    let mut absent = 0;

    // A function often holds several switches sharing one default, so each
    // recovered table may answer for at most one reference table. Matching many
    // to one would report a correct table as a disagreement against a reference
    // it was never meant to answer.
    let mut claimed = vec![false; all.recovered().len()];

    let mut unmatched = Vec::new();
    for expected in &reference {
        let exact = all.recovered().iter().enumerate().position(|(at, table)| {
            !claimed[at]
                && table.default == Some(expected.default)
                && table.targets == expected.labels
        });
        match exact {
            Some(at) => {
                claimed[at] = true;
                agreed += 1;
            }
            None => unmatched.push(expected),
        }
    }

    for expected in unmatched {
        let sharing = all
            .recovered()
            .iter()
            .enumerate()
            .position(|(at, table)| !claimed[at] && table.default == Some(expected.default));
        match sharing {
            Some(at) => {
                claimed[at] = true;
                if let Some(table) = all.recovered().get(at) {
                    disagreed.push((expected, table));
                }
            }
            None => absent += 1,
        }
    }

    let extra = claimed.iter().filter(|used| !**used).count();
    eprintln!("reference tables      {:>8}", reference.len());
    eprintln!("recovered             {:>8}", all.recovered().len());
    eprintln!("agreed exactly        {agreed:>8}");
    eprintln!("disagreed             {:>8}", disagreed.len());
    eprintln!("in reference only     {absent:>8}");
    eprintln!("recovered only        {extra:>8}");

    // A table only this change finds is something to check rather than a win,
    // because inventing tables is the failure mode that matters most.
    for (at, table) in all.recovered().iter().enumerate() {
        if !claimed.get(at).copied().unwrap_or(true) {
            eprintln!(
                "  only here: branch {:#010x} table {:#010x} {} entries",
                table.branch,
                table.table.unwrap_or(0),
                table.entries()
            );
        }
    }

    for (expected, table) in disagreed.iter().take(3) {
        eprintln!(
            "\ndisagreement at default {:#010x}, branch {:#010x}",
            expected.default, table.branch
        );
        let show = |labels: &[u32]| {
            labels
                .iter()
                .take(5)
                .map(|label| format!("{label:#010x}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        eprintln!(
            "  reference r{:<3} {:>4} labels  {}",
            expected.register,
            expected.labels.len(),
            show(&expected.labels)
        );
        eprintln!(
            "  recovered r{:<3} {:>4} labels  {}",
            table.index_register,
            table.targets.len(),
            show(&table.targets)
        );
    }

    assert!(
        disagreed.is_empty(),
        "recovery produced different targets than the reference for {} tables",
        disagreed.len()
    );
}

/// Reports the largest runs of executable words no function claimed.
///
/// Coverage on its own says how much was missed but nothing about what. These
/// runs are what the shortfall is made of, so classifying the largest of them
/// is what turns a number into a reason.
#[test]
fn reports_the_largest_unclaimed_ranges() {
    let (image, _) = supplied_image!();
    let program = analyze(&image, &[]);

    let mut claimed = std::collections::BTreeSet::new();
    for function in program.functions() {
        for block in &function.blocks {
            let mut address = block.start;
            while address < block.end {
                claimed.insert(address);
                address = address.saturating_add(4);
            }
        }
    }

    let mut runs: Vec<(u32, u32)> = Vec::new();
    for section in image.executable_sections() {
        let mut address = section.start;
        let mut run_start = None;

        while u64::from(address) < section.end() {
            match (claimed.contains(&address), run_start) {
                (false, None) => run_start = Some(address),
                (true, Some(start)) => {
                    runs.push((start, address));
                    run_start = None;
                }
                _ => {}
            }
            address = address.saturating_add(4);
        }
        if let Some(start) = run_start {
            let end = u32::try_from(section.end()).unwrap_or(u32::MAX);
            runs.push((start, end));
        }
    }

    let total: u64 = runs.iter().map(|(s, e)| u64::from(e - s) / 4).sum();
    eprintln!("unclaimed runs   {:>10}", runs.len());
    eprintln!("unclaimed words  {total:>10}");

    // A recovered table names addresses control actually reaches. One that no
    // function claims is code discovery could have walked and did not, which
    // says the shortfall is reachable rather than merely undiscovered.
    let mut targets = std::collections::BTreeSet::new();
    for function in program.functions() {
        for table in xenolith_analysis::recover(&image, function).recovered() {
            targets.extend(table.targets.iter().copied());
            targets.extend(table.default);
        }
    }
    let unclaimed_targets = targets.iter().filter(|t| !claimed.contains(t)).count();
    eprintln!("table targets    {:>10}", targets.len());
    eprintln!("  unclaimed      {unclaimed_targets:>10}");

    runs.sort_by_key(|(start, end)| std::cmp::Reverse(end - start));

    eprintln!("\nlargest unclaimed runs");
    for (start, end) in runs.iter().take(12) {
        // Classifying a run needs to know whether it decodes at all. Padding and
        // data do not, and a missed function does.
        let mut decodable = 0u32;
        let mut words = 0u32;
        let mut address = *start;
        while address < *end {
            if let Ok(word) = image.u32(address) {
                words += 1;
                if !xenolith_ppc::Instruction::decode(word).is_unknown() {
                    decodable += 1;
                }
            }
            address = address.saturating_add(4);
        }

        eprintln!(
            "  {start:#010x}..{end:#010x}  {:>7} words, {:>3}% decode",
            words,
            (decodable * 100).checked_div(words).unwrap_or(0)
        );
    }
}
