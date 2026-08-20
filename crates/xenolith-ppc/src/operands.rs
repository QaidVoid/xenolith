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
//! Two comparisons run here, and they reach different things.
//!
//! The first compares the multiset of values, sorted. That works over every
//! encoding the oracle can name, including the ones it spells with an extended
//! mnemonic that folds an operand into the name, because a sorted comparison
//! does not care which order the two printed them in.
//!
//! Its blind spot is exactly that. It cannot see operands printed in the wrong
//! order, and for a long time this crate printed the logical operations and the
//! shifts with their source and target the wrong way round. Every value was
//! present, so the sorted comparison agreed, and the text said an instruction
//! wrote the register it read.
//!
//! So the second compares the operand text exactly, in order, restricted to
//! encodings where the oracle chose the same mnemonic. Agreeing on the name
//! means neither folded an operand into it, and two disassemblers naming the
//! same instruction should name its operands in the same order.

use std::fmt::Write as _;
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
///
/// This cannot see a field printed as the wrong kind when the number happens to
/// match, such as a shift count of four printed as register four. Only reading
/// the text catches that.
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

/// Returns the operand list alone, in a form that only order can change.
///
/// Three things are normalized away, none of which is a claim about the
/// encoding. Spacing, since the two disassemblers pad differently. The note the
/// oracle writes in angle brackets after a target it recognizes, which is about
/// the file rather than the instruction. And how a number is spelled: one side
/// pads an address to eight digits and the other does not, so every literal is
/// reduced to its value, narrowed to the width the fields really have.
///
/// A number that follows a letter is part of a register name and is left alone.
pub(crate) fn operand_text(text: &str) -> String {
    let body = text
        .split_once(char::is_whitespace)
        .map_or("", |(_, rest)| rest);
    let body = body.split('<').next().unwrap_or("");
    let bytes = body.as_bytes();

    let mut out = String::new();
    let mut at = 0;
    let mut previous = b',';

    while at < bytes.len() {
        let negative = bytes[at] == b'-';
        let start = at + usize::from(negative);
        let begins = start < bytes.len()
            && bytes[start].is_ascii_digit()
            && !previous.is_ascii_alphanumeric();
        if !begins {
            if !bytes[at].is_ascii_whitespace() {
                out.push(char::from(bytes[at]));
                previous = bytes[at];
            }
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
        match u64::from_str_radix(&body[digits..end], radix) {
            Ok(value) => {
                let value = if negative {
                    0u64.wrapping_sub(value)
                } else {
                    value
                };
                let _ = write!(out, "{:x}", value & 0xffff_ffff);
                previous = b'0';
            }
            Err(_) => out.push_str(&body[at..end]),
        }
        at = end;
    }

    out
}

/// Returns the mnemonic alone.
pub(crate) fn mnemonic_of(text: &str) -> &str {
    text.split_whitespace().next().unwrap_or("")
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

    /// Compares operand text, in order, against the oracle over real code.
    ///
    /// Restricted to encodings the oracle named the same way this crate did.
    /// A different name means one of the two folded an operand into it, and
    /// then the operand lists legitimately differ in count and in order. The
    /// same name leaves nothing to explain a difference.
    #[test]
    fn operand_order_agrees_with_the_oracle() {
        let objdump = require_oracle!();

        let Ok(path) = std::env::var("XENOLITH_PPC_CODE") else {
            eprintln!("skipping: XENOLITH_PPC_CODE names no code to compare over");
            return;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("skipping: {path} could not be read");
            return;
        };

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
        let mut named_apart = 0u32;
        let mut disagreements = Vec::new();

        for entry in &decoded {
            if entry.text.starts_with(".long") || entry.text.is_empty() {
                continue;
            }
            let instruction = Instruction::decode(entry.word);
            if instruction.is_unknown() {
                continue;
            }

            let ours = instruction.render(entry.address).to_string();
            if mnemonic_of(&ours) != mnemonic_of(&entry.text) {
                named_apart += 1;
                continue;
            }

            compared += 1;
            if operand_text(&ours) != operand_text(&entry.text) && disagreements.len() < 10 {
                disagreements.push(format!(
                    "{:08x}  ours: {ours:<34} theirs: {}",
                    entry.word, entry.text
                ));
            }
        }

        eprintln!("named the same        {compared:>8}");
        eprintln!("named differently     {named_apart:>8}");
        eprintln!("disagreeing           {:>8}", disagreements.len());
        for line in &disagreements {
            eprintln!("  {line}");
        }

        assert!(
            compared > 1_000,
            "too few encodings named alike to say anything"
        );
        assert!(
            disagreements.is_empty(),
            "operands are printed in a different order from the oracle for {} encodings",
            disagreements.len()
        );
    }

    #[test]
    fn operand_text_keeps_only_what_order_can_change() {
        // Spacing differs between the two and says nothing.
        assert_eq!(
            operand_text("ori     r3,r4,5"),
            operand_text("ori r3, r4, 5")
        );
        // One side pads an address and the other does not.
        assert_eq!(
            operand_text("bl 0x002f0070"),
            operand_text("bl      0x2f0070 <thing>")
        );
        // A negative displacement and its unsigned spelling are one value.
        assert_eq!(
            operand_text("stw r14, -152(r1)"),
            operand_text("stw r14,4294967144(r1)")
        );
        // The digits in a register name are part of the name.
        assert_eq!(operand_text("ori r3, r4, 5"), "r3,r4,5");
        assert_eq!(operand_text("blr"), "");
        // Order is the whole point, so it has to survive all of that.
        assert_ne!(operand_text("ori r3, r4, 5"), operand_text("ori r4, r3, 5"));
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
