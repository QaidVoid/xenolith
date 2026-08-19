//! The instruction table and the dispatch built from it.
//!
//! Each instruction is declared exactly once. The [`instructions!`] macro
//! expands every declaration into two things: an entry in [`TABLE`], which
//! carries the metadata that text rendering and the consistency tests read, and
//! an arm of [`decode_opcode`], which is what actually runs. Declaring them
//! separately would let the two drift apart.
//!
//! Dispatch is two dense matches. The first selects on the primary opcode, and
//! the second on the extended opcode of the primary that has one. Within a
//! primary the ten-bit extended opcode is tried before the nine-bit one, so the
//! more specific form wins, which is the same precedence the ambiguity test
//! enforces over the table.

use crate::form::Form;

/// One instruction's metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Entry {
    /// The operation this entry identifies.
    pub opcode: Opcode,
    /// Mnemonic used when rendering the instruction as text.
    pub mnemonic: &'static str,
    /// Encoding form, which decides how operands are laid out.
    pub form: Form,
    /// Bits that identify the instruction.
    pub mask: u32,
    /// Values those bits take for this instruction.
    pub value: u32,
}

/// Declares instructions, expanding each into a table entry and a dispatch arm.
macro_rules! instructions {
    (
        d {
            $( $d_primary:literal => $d_variant:ident = $d_mnemonic:literal; )*
        }
        $(
            extended $e_primary:literal {
                x { $( $x_xo:literal => $x_variant:ident = $x_mnemonic:literal; )* }
                xo { $( $o_xo:literal => $o_variant:ident = $o_mnemonic:literal; )* }
            }
        )*
    ) => {
        /// An operation the decoder recognizes.
        ///
        /// Carries no operands. Those stay in the instruction word and are read
        /// through accessors, which keeps a decoded instruction small enough to
        /// hold millions of without thinking about it.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[non_exhaustive]
        pub enum Opcode {
            /// The word encodes no instruction this crate recognizes.
            Unknown,
            $( #[doc = concat!("`", $d_mnemonic, "`")] $d_variant, )*
            $( $( #[doc = concat!("`", $x_mnemonic, "`")] $x_variant, )* )*
            $( $( #[doc = concat!("`", $o_mnemonic, "`")] $o_variant, )* )*
        }

        /// Every instruction this crate knows, in declaration order.
        pub(crate) const TABLE: &[Entry] = &[
            $( Entry {
                opcode: Opcode::$d_variant,
                mnemonic: $d_mnemonic,
                form: Form::D,
                mask: Form::D.mask(),
                value: $d_primary << 26,
            }, )*
            $( $( Entry {
                opcode: Opcode::$x_variant,
                mnemonic: $x_mnemonic,
                form: Form::X,
                mask: Form::X.mask(),
                value: ($e_primary << 26) | ($x_xo << 1),
            }, )* )*
            $( $( Entry {
                opcode: Opcode::$o_variant,
                mnemonic: $o_mnemonic,
                form: Form::XO,
                mask: Form::XO.mask(),
                value: ($e_primary << 26) | ($o_xo << 1),
            }, )* )*
        ];

        /// Identifies the operation a word encodes.
        pub(crate) const fn decode_opcode(word: u32) -> Opcode {
            match word >> 26 {
                $( $d_primary => Opcode::$d_variant, )*
                $( $e_primary => match (word >> 1) & 0x3ff {
                    $( $x_xo => Opcode::$x_variant, )*
                    _ => match (word >> 1) & 0x1ff {
                        $( $o_xo => Opcode::$o_variant, )*
                        _ => Opcode::Unknown,
                    },
                }, )*
                _ => Opcode::Unknown,
            }
        }
    };
}

instructions! {
    d {
        10 => Cmpli = "cmpli";
        11 => Cmpi = "cmpi";
        14 => Addi = "addi";
        15 => Addis = "addis";
        24 => Ori = "ori";
        25 => Oris = "oris";
        32 => Lwz = "lwz";
        33 => Lwzu = "lwzu";
        34 => Lbz = "lbz";
        36 => Stw = "stw";
        37 => Stwu = "stwu";
        38 => Stb = "stb";
        40 => Lhz = "lhz";
        44 => Sth = "sth";
    }

    extended 31 {
        x {
            0 => Cmp = "cmp";
            23 => Lwzx = "lwzx";
            24 => Slw = "slw";
            28 => And = "and";
            32 => Cmpl = "cmpl";
            151 => Stwx = "stwx";
            316 => Xor = "xor";
            444 => Or = "or";
            536 => Srw = "srw";
        }
        xo {
            40 => Subf = "subf";
            75 => Mulhw = "mulhw";
            235 => Mullw = "mullw";
            266 => Add = "add";
            491 => Divw = "divw";
        }
    }
}

impl Opcode {
    /// Returns the table entry for this opcode, if it has one.
    pub(crate) fn entry(self) -> Option<&'static Entry> {
        TABLE.iter().find(|entry| entry.opcode == self)
    }

    /// Returns the mnemonic used when rendering this opcode as text.
    #[must_use]
    pub fn mnemonic(self) -> &'static str {
        self.entry().map_or("<unknown>", |entry| entry.mnemonic)
    }

    /// Returns the encoding form of this opcode, if it has one.
    #[must_use]
    pub fn form(self) -> Option<Form> {
        self.entry().map(|entry| entry.form)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns whether two entries can both match the same word.
    fn overlap(a: &Entry, b: &Entry) -> bool {
        let shared = a.mask & b.mask;
        a.value & shared == b.value & shared
    }

    /// Returns whether `a` is strictly more specific than `b`.
    fn refines(a: &Entry, b: &Entry) -> bool {
        a.mask & b.mask == b.mask && a.mask != b.mask
    }

    /// The table must be unambiguous. Two entries may only overlap when one
    /// mask strictly refines the other, in which case the specific entry wins.
    ///
    /// This matters most where VMX128 shares primary opcode space with standard
    /// `AltiVec`. Without it, correctness would quietly depend on the order the
    /// declarations happen to be written in.
    #[test]
    fn no_two_entries_are_ambiguous() {
        for (i, a) in TABLE.iter().enumerate() {
            for b in TABLE.iter().skip(i + 1) {
                if !overlap(a, b) {
                    continue;
                }
                assert!(
                    refines(a, b) || refines(b, a),
                    "{} and {} overlap without one refining the other \
                     (masks {:#010x} and {:#010x})",
                    a.mnemonic,
                    b.mnemonic,
                    a.mask,
                    b.mask
                );
            }
        }
    }

    #[test]
    fn no_two_entries_share_an_opcode() {
        for (i, a) in TABLE.iter().enumerate() {
            for b in TABLE.iter().skip(i + 1) {
                assert_ne!(a.opcode, b.opcode, "{} declared twice", a.mnemonic);
            }
        }
    }

    #[test]
    fn no_two_entries_share_a_mnemonic() {
        for (i, a) in TABLE.iter().enumerate() {
            for b in TABLE.iter().skip(i + 1) {
                assert_ne!(a.mnemonic, b.mnemonic, "{} declared twice", a.mnemonic);
            }
        }
    }

    /// Every entry's own canonical encoding must decode back to it. This is
    /// what binds the dispatch to the table, since the two are generated from
    /// one declaration but are otherwise independent code.
    #[test]
    fn every_entry_decodes_back_to_itself() {
        for entry in TABLE {
            assert_eq!(
                decode_opcode(entry.value),
                entry.opcode,
                "{} does not decode back to itself from {:#010x}",
                entry.mnemonic,
                entry.value
            );
        }
    }

    /// Operand bits must not change which instruction was decoded.
    #[test]
    fn operand_bits_do_not_affect_dispatch() {
        for entry in TABLE {
            for fill in [0x0000_0000, 0xffff_ffff, 0xaaaa_aaaa, 0x5555_5555u32] {
                let word = entry.value | (fill & !entry.mask);
                assert_eq!(
                    decode_opcode(word),
                    entry.opcode,
                    "{} lost its identity with operand bits {fill:#010x}",
                    entry.mnemonic
                );
            }
        }
    }

    #[test]
    fn every_entry_identifies_its_own_value() {
        for entry in TABLE {
            assert_eq!(
                entry.value & entry.mask,
                entry.value,
                "{} carries identifying bits outside its own mask",
                entry.mnemonic
            );
        }
    }

    #[test]
    fn the_mask_of_each_entry_matches_its_form() {
        for entry in TABLE {
            assert_eq!(
                entry.mask,
                entry.form.mask(),
                "{} carries a mask its form does not describe",
                entry.mnemonic
            );
        }
    }

    #[test]
    fn opcode_metadata_is_reachable() {
        for entry in TABLE {
            assert_eq!(entry.opcode.mnemonic(), entry.mnemonic);
            assert_eq!(entry.opcode.form(), Some(entry.form));
        }

        assert_eq!(Opcode::Unknown.mnemonic(), "<unknown>");
        assert_eq!(Opcode::Unknown.form(), None);
    }
}
