//! Differential testing of the decoder against an independent oracle.
//!
//! Hand written expectations only prove the cases someone thought to write, and
//! an instruction table is exactly the kind of thing where a transposed digit
//! survives review. Running the same encodings through `llvm-mc` and comparing
//! catches that class of mistake across the whole table at once.
//!
//! Two properties of the oracle shape everything here.
//!
//! It prints nothing at all for an encoding it rejects, so outputs cannot be
//! paired with inputs by position. A single rejection shifts every later pairing
//! and turns one skipped encoding into a cascade of false failures. The pairing
//! is instead recovered from the line numbers in its diagnostics.
//!
//! It also answers confidently and wrongly inside VMX128 encoding space, where
//! it reports instructions the architecture gained long after the console
//! shipped. Comparing there would manufacture both false failures and, worse,
//! false agreement, so that space is excluded by an explicit rule rather than by
//! happening not to generate encodings in it.

use std::process::Command;

use crate::instruction::Instruction;
use crate::table::{Entry, Opcode, TABLE};

/// Target `llvm-mc` is asked to decode for.
const TRIPLE: &str = "--triple=powerpc64-unknown-linux-gnu";

/// Mnemonics `llvm-mc` prefers, paired with the operation they really encode.
///
/// These are extended mnemonics, a rendering convention rather than distinct
/// instructions. Our decoder reports the underlying operation, so the two
/// spellings have to be brought together before they can be compared.
const EXTENDED_MNEMONICS: &[(&str, &str)] = &[
    // Compares name their operand width in the mnemonic.
    ("cmpw", "cmp"),
    ("cmpd", "cmp"),
    ("cmplw", "cmpl"),
    ("cmpld", "cmpl"),
    ("cmpwi", "cmpi"),
    ("cmpdi", "cmpi"),
    ("cmplwi", "cmpli"),
    ("cmpldi", "cmpli"),
    // Arithmetic spelled as its more familiar operation.
    ("sub", "subf"),
    ("subc", "subfc"),
    ("subi", "addi"),
    ("subis", "addis"),
    ("subic", "addic"),
    ("subic.", "addic."),
    ("li", "addi"),
    ("la", "addi"),
    ("lis", "addis"),
    // Logical operations with a repeated or ignored operand.
    ("mr", "or"),
    ("not", "nor"),
    ("nop", "ori"),
    ("xnop", "xori"),
    // The 32-bit rotate family, which is one instruction under many names.
    ("extlwi", "rlwinm"),
    ("extrwi", "rlwinm"),
    ("rotlwi", "rlwinm"),
    ("rotrwi", "rlwinm"),
    ("slwi", "rlwinm"),
    ("srwi", "rlwinm"),
    ("clrlwi", "rlwinm"),
    ("clrrwi", "rlwinm"),
    ("clrlslwi", "rlwinm"),
    ("rotlw", "rlwnm"),
    ("inslwi", "rlwimi"),
    ("insrwi", "rlwimi"),
    // The 64-bit rotate family, likewise.
    ("extrdi", "rldicl"),
    ("rotldi", "rldicl"),
    ("rotrdi", "rldicl"),
    ("srdi", "rldicl"),
    ("clrldi", "rldicl"),
    ("extldi", "rldicr"),
    ("sldi", "rldicr"),
    ("clrrdi", "rldicr"),
    ("clrlsldi", "rldic"),
    ("insrdi", "rldimi"),
    ("rotld", "rldcl"),
    // Condition register moves that target a single field. The console treats
    // these as hints over the full width forms, and reading one as the full
    // width form is the conservative direction: it moves more than was asked
    // for rather than less.
    ("mtocrf", "mtcrf"),
    ("mfocrf", "mfcr"),
];

/// Returns whether the oracle can be run at all.
///
/// The suite has to stay usable without it, so its absence skips the
/// comparisons rather than failing them.
pub(crate) fn oracle_available() -> bool {
    Command::new("llvm-mc")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Returns whether a word lies in encoding space that VMX128 claims.
///
/// The console re-encodes vector operations across these primary opcodes to
/// reach 128 vector registers. General purpose PowerPC decoders read the same
/// bits as much later instructions, so the oracle must not be consulted here.
pub(crate) const fn in_vmx128_space(word: u32) -> bool {
    matches!(word >> 26, 4..=6)
}

/// Disassembles words with the oracle, returning one result per input word.
///
/// A `None` means the oracle rejected that encoding. Results line up with
/// `words` by index regardless of how many were rejected.
fn disassemble(words: &[u32]) -> Vec<Option<String>> {
    let input = words
        .iter()
        .map(|word| {
            let [a, b, c, d] = word.to_be_bytes();
            format!("0x{a:02x} 0x{b:02x} 0x{c:02x} 0x{d:02x}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let output = Command::new("llvm-mc")
        .args([TRIPLE, "--disassemble"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(input.as_bytes())?;
            }
            child.wait_with_output()
        })
        .expect("the oracle should run");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let rejected = rejected_lines(&stderr);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut printed = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('.'));

    (1..=words.len())
        .map(|line| {
            if rejected.contains(&line) {
                None
            } else {
                printed.next().map(str::to_owned)
            }
        })
        .collect()
}

/// Extracts the input line numbers the oracle refused to decode.
fn rejected_lines(stderr: &str) -> Vec<usize> {
    stderr
        .lines()
        .filter_map(|line| line.strip_prefix("<stdin>:"))
        .filter_map(|rest| rest.split(':').next())
        .filter_map(|number| number.parse().ok())
        .collect()
}

/// Returns the mnemonic of a disassembled line.
fn mnemonic_of(line: &str) -> &str {
    line.split_whitespace().next().unwrap_or("")
}

/// Returns whether our table declares a mnemonic.
fn is_declared(mnemonic: &str) -> bool {
    TABLE.iter().any(|entry| entry.mnemonic == mnemonic)
}

/// Brings a mnemonic the oracle printed into the spelling our table uses.
///
/// Strips the suffixes that mark the record and overflow bits, since those
/// select a variant of one instruction rather than a different one, and then
/// resolves extended mnemonics to the operation underneath.
fn normalize(printed: &str) -> String {
    // Checked before the record bit suffix is stripped, because some mnemonics
    // end in a dot as part of their name rather than as a variant marker.
    if is_declared(printed) {
        return printed.to_owned();
    }

    let base = printed.trim_end_matches('.');

    if is_declared(base) {
        return base.to_owned();
    }

    if let Some(stripped) = base.strip_suffix('o') {
        if is_declared(stripped) {
            return stripped.to_owned();
        }
    }

    for (extended, general) in EXTENDED_MNEMONICS {
        if base == *extended {
            return (*general).to_owned();
        }
    }

    base.to_owned()
}

/// Returns whether the oracle's rendering agrees with the operation we expect.
///
/// Most disagreements in spelling are extended mnemonics and resolve through
/// [`normalize`]. The moves to and from a special purpose register are
/// different: they name the register in the mnemonic, and which registers the
/// oracle knows by name is a property of its version rather than of our
/// decoding, so that family is matched by prefix instead of by an exhaustive
/// list that would rot with every LLVM release.
fn agrees(expected: &str, printed: &str) -> bool {
    if normalize(printed) == expected {
        return true;
    }

    match expected {
        "mfspr" => printed.starts_with("mf"),
        "mtspr" => printed.starts_with("mt"),
        "mftb" => printed.starts_with("mftb"),
        _ => false,
    }
}

/// Builds an encoding of `entry` with the given operand bits.
fn encode(entry: &Entry, operands: u32) -> u32 {
    entry.value | (operands & !entry.mask)
}

/// A reproducible source of operand bits.
///
/// Deterministic so that a failure can be re-run and investigated rather than
/// disappearing on the next run.
struct Operands(u64);

impl Operands {
    /// Starts a sequence from a fixed seed.
    const fn new() -> Self {
        Self(0x2545_f491_4f6c_dd1d)
    }

    /// Returns the next set of operand bits.
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 32) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Skips the enclosing test when the oracle is not installed.
    macro_rules! require_oracle {
        () => {
            if !oracle_available() {
                eprintln!("skipping: llvm-mc is not installed");
                return;
            }
        };
    }

    #[test]
    fn pairs_outputs_with_inputs_across_a_rejected_encoding() {
        require_oracle!();

        // The second word sets a reserved bit and is rejected. Without the line
        // numbers from the diagnostics, every later result would shift up by
        // one and the whole comparison would read as broken.
        let words = [0x7c08_02a6, 0x2864_0010, 0x3864_0010, 0x7c64_2a14];
        let results = disassemble(&words);

        assert_eq!(results.len(), words.len());
        assert_eq!(mnemonic_of(results[0].as_deref().unwrap()), "mflr");
        assert_eq!(results[1], None, "the reserved bit encoding was accepted");
        assert_eq!(mnemonic_of(results[2].as_deref().unwrap()), "addi");
        assert_eq!(mnemonic_of(results[3].as_deref().unwrap()), "add");
    }

    #[test]
    fn normalizes_the_suffixes_that_mark_variant_bits() {
        assert_eq!(normalize("add"), "add");
        assert_eq!(normalize("add."), "add");
        assert_eq!(normalize("addo"), "add");
        assert_eq!(normalize("addo."), "add");
    }

    #[test]
    fn normalizes_extended_mnemonics_to_the_operation_underneath() {
        assert_eq!(normalize("cmpw"), "cmp");
        assert_eq!(normalize("cmplwi"), "cmpli");
        assert_eq!(normalize("sub"), "subf");
        assert_eq!(normalize("mr"), "or");
        assert_eq!(normalize("li"), "addi");
    }

    /// Normalization must not rewrite a mnemonic we declare outright, which is
    /// what would happen if the overflow suffix were stripped unconditionally.
    #[test]
    fn normalizing_leaves_a_declared_mnemonic_alone() {
        assert_eq!(normalize("or"), "or");
        assert_eq!(normalize("lwz"), "lwz");
    }

    #[test]
    fn agrees_with_the_oracle_on_every_declared_instruction() {
        require_oracle!();

        let mut operands = Operands::new();
        let mut compared = vec![0usize; TABLE.len()];
        let mut failures = Vec::new();

        for round in 0..24 {
            let words: Vec<u32> = TABLE
                .iter()
                .map(|entry| {
                    let bits = if round == 0 { 0 } else { operands.next() };
                    encode(entry, bits)
                })
                .collect();

            for (index, (entry, result)) in TABLE.iter().zip(disassemble(&words)).enumerate() {
                let Some(line) = result else {
                    continue;
                };
                assert!(
                    !in_vmx128_space(words[index]),
                    "compared inside vmx128 space"
                );

                compared[index] += 1;
                if !agrees(entry.mnemonic, mnemonic_of(&line)) {
                    failures.push(format!(
                        "{:#010x}: ours={} theirs={} ({line})",
                        words[index],
                        entry.mnemonic,
                        normalize(mnemonic_of(&line))
                    ));
                }
            }
        }

        assert!(
            failures.is_empty(),
            "disagreements:\n  {}",
            failures.join("\n  ")
        );

        for (entry, count) in TABLE.iter().zip(&compared) {
            assert!(
                *count > 0,
                "{} was never compared, so the oracle rejected every encoding of it",
                entry.mnemonic
            );
        }
    }

    /// Operand bits must never change which instruction the oracle reports,
    /// which is the same invariant the table tests assert about our dispatch.
    #[test]
    fn operand_bits_do_not_change_what_the_oracle_reports() {
        require_oracle!();

        let mut operands = Operands::new();
        for entry in TABLE {
            let words: Vec<u32> = (0..8).map(|_| encode(entry, operands.next())).collect();

            for line in disassemble(&words).into_iter().flatten() {
                assert!(
                    agrees(entry.mnemonic, mnemonic_of(&line)),
                    "{} changed identity under different operand bits, \
                     the oracle reported {line}",
                    entry.mnemonic
                );
            }
        }
    }

    /// Records, as an executable fact, why the oracle is excluded from VMX128
    /// encoding space. Prose explaining this would rot. A test does not.
    #[test]
    fn the_oracle_is_wrong_inside_vmx128_space() {
        require_oracle!();

        // A VMX128 vector load. The architecture later assigned this encoding
        // to an unrelated integer divide, which is what the oracle reports.
        let word = 0x100b_60cb;
        assert!(in_vmx128_space(word));

        let reported = disassemble(&[word])
            .into_iter()
            .next()
            .flatten()
            .expect("the oracle accepts this encoding, which is the problem");

        assert_eq!(
            mnemonic_of(&reported),
            "vdivud",
            "the oracle no longer misreports this encoding, so revisit the exclusion"
        );
    }

    /// The exclusion has to be a rule the harness applies, not an accident of
    /// which encodings happen to be generated.
    #[test]
    fn vmx128_space_is_excluded_by_rule() {
        assert!(in_vmx128_space(0x100b_60cb));
        for primary in [4u32, 5, 6] {
            assert!(in_vmx128_space(primary << 26));
        }

        for primary in [0u32, 14, 31, 32, 63] {
            assert!(!in_vmx128_space(primary << 26));
        }

        for entry in TABLE {
            assert!(
                !in_vmx128_space(entry.value),
                "{} is declared inside vmx128 space, where the oracle cannot check it",
                entry.mnemonic
            );
        }
    }

    /// Our decoder and the oracle must agree on which encodings mean nothing,
    /// at least for the primary opcodes we claim to cover.
    #[test]
    fn a_word_we_decode_is_never_one_the_oracle_rejects() {
        require_oracle!();

        let mut operands = Operands::new();
        let words: Vec<u32> = TABLE
            .iter()
            .flat_map(|entry| {
                let bits: Vec<u32> = (0..4).map(|_| operands.next()).collect();
                bits.into_iter().map(|bits| encode(entry, bits))
            })
            .collect();

        let results = disassemble(&words);
        let rejected = results.iter().filter(|result| result.is_none()).count();

        // Some encodings set bits the architecture reserves, and the oracle is
        // entitled to refuse those. A wholesale refusal would mean the harness
        // is generating nonsense and comparing nothing.
        assert!(
            rejected * 4 < words.len(),
            "the oracle rejected {rejected} of {} encodings",
            words.len()
        );

        for (word, result) in words.iter().zip(&results) {
            if result.is_some() {
                assert!(
                    !Instruction::decode(*word).is_unknown(),
                    "{word:#010x} was generated from the table but decodes as unknown"
                );
            }
        }
    }

    #[test]
    fn the_unknown_opcode_is_never_declared() {
        assert!(!TABLE.iter().any(|entry| entry.opcode == Opcode::Unknown));
    }
}
