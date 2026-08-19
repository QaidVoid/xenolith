//! Checking operand extraction against a second, independent disassembler.
//!
//! The existing differential harness asks `llvm-mc` what an encoding is and
//! compares the mnemonic. That says nothing about the operands, which is where
//! this crate has made most of its mistakes: a displacement read two bytes off
//! because the low bits carry an opcode, a rotate whose mask bounds were never
//! printed, a six bit field whose top bit sits away from the rest.
//!
//! GNU `objdump` is a different implementation by different people, and it
//! prints the values it extracted. Comparing those values catches a field read
//! from the wrong place, which comparing names cannot.
//!
//! Operand order is deliberately not compared. This crate prints operands in
//! encoding order and an assembler prints them in the order it accepts, and the
//! two differ for whole families. What is compared is the multiset of values,
//! which is order independent and still pins every field.

use std::io::Write as _;
use std::process::Command;

/// Environment variable naming the cross binutils prefix.
///
/// Given rather than searched for, because the tools are installed under a name
/// that says which target they were built for and there is no one right guess.
const PREFIX: &str = "XENOLITH_PPC_BINUTILS";

/// Returns the objdump to compare against, if one was named and works.
pub(crate) fn oracle() -> Option<String> {
    let prefix = std::env::var(PREFIX).ok()?;
    let objdump = format!("{prefix}objdump");

    Command::new(&objdump)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|_| objdump)
}

/// One line of the oracle's output.
#[derive(Debug)]
pub(crate) struct Decoded {
    /// Where the oracle decoded this word, which a branch target is relative to.
    pub address: u32,
    pub word: u32,
    pub text: String,
}

/// Returns what the oracle makes of each word, in order.
///
/// The words are handed over as a raw image rather than as assembly, since the
/// point is to compare decoding rather than assembling. Big endian is stated
/// outright: a raw blob carries no byte order, and the default is the host's.
pub(crate) fn decode(objdump: &str, words: &[u32]) -> Vec<Decoded> {
    let Ok(mut file) = tempfile() else {
        return Vec::new();
    };
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for word in words {
        bytes.extend_from_slice(&word.to_be_bytes());
    }
    if file.0.write_all(&bytes).is_err() {
        return Vec::new();
    }
    let _ = file.0.flush();

    let Ok(output) = Command::new(objdump)
        .args([
            "-D",
            "-b",
            "binary",
            "-EB",
            "-m",
            "powerpc:common64",
            "-M",
            "power8",
        ])
        .arg(&file.1)
        .output()
    else {
        return Vec::new();
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut decoded = Vec::new();

    for line in text.lines() {
        let Some((left, right)) = line.split_once(':') else {
            continue;
        };
        let Ok(address) = u32::from_str_radix(left.trim(), 16) else {
            continue;
        };
        let mut fields = right.trim().splitn(2, '\t');
        let Some(hex) = fields.next() else { continue };
        let bytes: Vec<u8> = hex
            .split_whitespace()
            .filter_map(|byte| u8::from_str_radix(byte, 16).ok())
            .collect();
        if bytes.len() != 4 {
            continue;
        }
        decoded.push(Decoded {
            address,
            word: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            text: fields.next().unwrap_or("").trim().to_owned(),
        });
    }

    decoded
}

/// Returns a file to write the words into, and its path.
fn tempfile() -> std::io::Result<(std::fs::File, std::path::PathBuf)> {
    let path = std::env::temp_dir().join(format!("xenolith-operands-{}.bin", std::process::id()));
    let file = std::fs::File::create(&path)?;
    Ok((file, path))
}

/// Returns every integer an operand list names, sorted.
///
/// Sorted rather than in order, because this crate prints operands in encoding
/// order and an assembler prints them in the order it accepts. What both agree
/// on is which values the fields hold.
pub(crate) fn values(text: &str) -> Vec<u64> {
    let body = text
        .split_once(char::is_whitespace)
        .map_or("", |(_, rest)| rest);
    let mut found: Vec<u64> = Vec::new();
    let bytes = body.as_bytes();
    let mut at = 0;

    while at < bytes.len() {
        let negative = bytes[at] == b'-';
        let start = at + usize::from(negative);
        if start >= bytes.len() || !bytes[start].is_ascii_digit() {
            at += 1;
            continue;
        }

        let (radix, digits) = if bytes[start..].starts_with(b"0x") {
            (16, start + 2)
        } else {
            (10, start)
        };
        let mut end = digits;
        while end < bytes.len() && (bytes[end] as char).is_digit(radix) {
            end += 1;
        }

        // Read unsigned, because the oracle sign extends a negative address to
        // sixty four bits and the result does not fit a signed one. Every value
        // is then narrowed to thirty two bits, which is the width the address
        // space and every immediate field actually have, so the two spellings
        // of the same address come out the same.
        if let Ok(value) = u64::from_str_radix(&body[digits..end], radix) {
            let value = if negative {
                0u64.wrapping_sub(value)
            } else {
                value
            };
            found.push(value & 0xffff_ffff);
        }
        at = end;
    }

    found.sort_unstable();
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Instruction;

    /// Skips the enclosing test when no cross binutils was named.
    macro_rules! require_oracle {
        () => {
            match oracle() {
                Some(objdump) => objdump,
                None => {
                    eprintln!("skipping: {PREFIX} does not name a working objdump");
                    return;
                }
            }
        };
    }

    /// Spellings where the oracle folds an operand into the instruction's name.
    ///
    /// An extended mnemonic can carry a field that this crate prints separately:
    /// `slwi` implies the mask a rotate would state, and `cmplwi` implies the
    /// width a compare would. Those are the same instruction written two ways,
    /// so the values legitimately differ in count.
    fn folds_an_operand(theirs: &str) -> bool {
        // The record bit is a suffix on the name rather than a different
        // instruction, so it is stripped before the name is recognized.
        let name = theirs
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches('.')
            .trim_end_matches(['+', '-']);
        let folding = [
            "slwi", "srwi", "clrlwi", "clrrwi", "rotlwi", "rotrwi", "inslwi", "insrwi", "extlwi",
            "extrwi", "clrlsldi", "sldi", "srdi", "clrldi", "clrrdi", "rotldi", "extldi", "extrdi",
            "cmpw", "cmpwi", "cmplw", "cmplwi", "cmpd", "cmpdi", "cmpld", "cmpldi", "mr", "not",
            "nop", "li", "lis", "la", "subi", "mtcr", "mfcr", "trap", "tw", "twi", "td", "tdi",
            "mflr", "mtlr", "mfctr", "mtctr", "mfxer", "mtxer", "mfmsr", "mttb", "mftb", "mftbu",
            // The ordering instructions fold the field saying how far the
            // ordering reaches into their name.
            "lwsync", "ptesync", "msync", "sync", "isync", "eieio", "hwsync", "vmr", "vnot",
        ];
        if folding.contains(&name) {
            return true;
        }
        // A trap is spelled for the comparison it traps on and a cache hint for
        // the hint it carries, both of which fold a field into the name.
        if (name.starts_with("tw") || name.starts_with("td") || name.starts_with("dcbt"))
            && !matches!(name, "tw" | "twi" | "td" | "tdi" | "dcbt" | "dcbtst")
        {
            return true;
        }

        // Every conditional branch is spelled for the condition it tests, which
        // folds both of the fields choosing that condition into the name.
        name.starts_with('b') && name != "b" && name != "bl" && name != "ba" && name != "bla"
    }

    /// Compares operand values against the oracle over a stretch of real code.
    ///
    /// Real code rather than generated words, because the fields that go wrong
    /// are the ones real compilers use, and a uniform sample over the encoding
    /// space spends almost all of itself on encodings nothing emits.
    #[test]
    fn operand_values_agree_with_the_oracle() {
        let objdump = require_oracle!();

        let Ok(path) = std::env::var("XENOLITH_PPC_CODE") else {
            eprintln!("skipping: XENOLITH_PPC_CODE names no code to compare over");
            return;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("skipping: {path} could not be read");
            return;
        };

        // One distinct word is worth as much as a thousand copies of it, and a
        // real title repeats itself heavily.
        let mut seen = std::collections::BTreeSet::new();
        let words: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|chunk| u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .filter(|word| seen.insert(*word))
            .take(20_000)
            .collect();

        let decoded = decode(&objdump, &words);
        assert!(!decoded.is_empty(), "the oracle decoded nothing");

        let mut compared = 0u32;
        let mut folded = 0u32;
        let mut disagreements = Vec::new();

        for entry in &decoded {
            if entry.text.starts_with(".long") || entry.text.is_empty() {
                continue;
            }
            let instruction = Instruction::decode(entry.word);
            if instruction.is_unknown() {
                continue;
            }
            if folds_an_operand(&entry.text) {
                folded += 1;
                continue;
            }

            // Rendered where the oracle decoded it, since a branch names a
            // target relative to the instruction rather than an offset.
            let ours = instruction.render(entry.address).to_string();
            compared += 1;
            if values(&ours) != values(&entry.text) && disagreements.len() < 10 {
                disagreements.push(format!(
                    "{:08x}  ours: {ours:<34} theirs: {}",
                    entry.word, entry.text
                ));
            }
        }

        eprintln!("distinct words        {:>8}", words.len());
        eprintln!("compared              {compared:>8}");
        eprintln!("spelled with a fold   {folded:>8}");
        eprintln!("disagreeing           {:>8}", disagreements.len());
        for line in &disagreements {
            eprintln!("  {line}");
        }

        assert!(
            disagreements.is_empty(),
            "operand values differ from the oracle for {} encodings",
            disagreements.len()
        );
    }

    #[test]
    fn values_are_read_in_any_base_and_sign() {
        // Narrowed to the width an address and an immediate field really have,
        // so that a negative displacement and its unsigned spelling agree.
        assert_eq!(values("stw r14, -152(r1)"), [1, 14, 0xffff_ff68]);
        assert_eq!(values("bc 12, 2, 0x82090034"), [2, 12, 0x8209_0034]);
        assert_eq!(values("blr"), []);
    }
}
