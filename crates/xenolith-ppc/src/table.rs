//! The instruction table and the dispatch built from it.
//!
//! Each instruction is declared exactly once. The [`instructions!`] macro
//! expands every declaration into two things: an entry in [`TABLE`], which
//! carries the metadata that text rendering and the consistency tests read, and
//! an arm of [`decode_opcode`], which is what actually runs. Declaring them
//! separately would let the two drift apart.
//!
//! Dispatch is two levels of dense match. The first selects on the primary
//! opcode. The second, for a primary that has extended opcodes, tries each form
//! in the order the forms are declared. Forms are written most specific first,
//! so a wider extended opcode is tested before a narrower one that overlaps it.
//! Getting that order wrong is caught by the test asserting every entry decodes
//! back to itself, rather than being left to review.

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
        primary {
            $( $p_form:ident { $( $p_op:literal => $p_var:ident = $p_mn:literal; )* } )*
        }
        $(
            extended $e_primary:literal {
                $( $e_form:ident { $( $e_xo:literal => $e_var:ident = $e_mn:literal; )* } )*
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
            $( $( #[doc = concat!("`", $p_mn, "`")] $p_var, )* )*
            $( $( $( #[doc = concat!("`", $e_mn, "`")] $e_var, )* )* )*
        }

        /// Every instruction this crate knows, in declaration order.
        pub(crate) const TABLE: &[Entry] = &[
            $( $( Entry {
                opcode: Opcode::$p_var,
                mnemonic: $p_mn,
                form: Form::$p_form,
                mask: Form::$p_form.mask(),
                value: $p_op << 26,
            }, )* )*
            $( $( $( Entry {
                opcode: Opcode::$e_var,
                mnemonic: $e_mn,
                form: Form::$e_form,
                mask: Form::$e_form.mask(),
                value: ($e_primary << 26)
                    | ($e_xo << Form::$e_form.extended_opcode_shift()),
            }, )* )* )*
        ];

        /// Identifies the operation a word encodes.
        pub(crate) const fn decode_opcode(word: u32) -> Opcode {
            match word >> 26 {
                $( $( $p_op => Opcode::$p_var, )* )*
                $( $e_primary => {
                    $(
                        match (word >> Form::$e_form.extended_opcode_shift())
                            & Form::$e_form.extended_opcode_field()
                        {
                            $( $e_xo => return Opcode::$e_var, )*
                            _ => {}
                        }
                    )*
                    Opcode::Unknown
                } )*
                _ => Opcode::Unknown,
            }
        }
    };
}

instructions! {
    primary {
        D {
            2 => Tdi = "tdi";
            3 => Twi = "twi";
            7 => Mulli = "mulli";
            8 => Subfic = "subfic";
            10 => Cmpli = "cmpli";
            11 => Cmpi = "cmpi";
            12 => Addic = "addic";
            13 => AddicRc = "addic.";
            14 => Addi = "addi";
            15 => Addis = "addis";
            24 => Ori = "ori";
            25 => Oris = "oris";
            26 => Xori = "xori";
            27 => Xoris = "xoris";
            28 => Andi = "andi.";
            29 => Andis = "andis.";
            32 => Lwz = "lwz";
            33 => Lwzu = "lwzu";
            34 => Lbz = "lbz";
            35 => Lbzu = "lbzu";
            36 => Stw = "stw";
            37 => Stwu = "stwu";
            38 => Stb = "stb";
            39 => Stbu = "stbu";
            40 => Lhz = "lhz";
            41 => Lhzu = "lhzu";
            42 => Lha = "lha";
            43 => Lhau = "lhau";
            44 => Sth = "sth";
            45 => Sthu = "sthu";
            46 => Lmw = "lmw";
            47 => Stmw = "stmw";
        }
        I {
            18 => B = "b";
        }
        B {
            16 => Bc = "bc";
        }
        SC {
            17 => Sc = "sc";
        }
        M {
            20 => Rlwimi = "rlwimi";
            21 => Rlwinm = "rlwinm";
            23 => Rlwnm = "rlwnm";
        }
    }

    extended 19 {
        XL {
            0 => Mcrf = "mcrf";
            16 => Bclr = "bclr";
            18 => Rfid = "rfid";
            33 => Crnor = "crnor";
            129 => Crandc = "crandc";
            150 => Isync = "isync";
            193 => Crxor = "crxor";
            225 => Crnand = "crnand";
            257 => Crand = "crand";
            274 => Hrfid = "hrfid";
            289 => Creqv = "creqv";
            417 => Crorc = "crorc";
            449 => Cror = "cror";
            528 => Bcctr = "bcctr";
        }
    }

    extended 30 {
        MDS {
            8 => Rldcl = "rldcl";
            9 => Rldcr = "rldcr";
        }
        MD {
            0 => Rldicl = "rldicl";
            1 => Rldicr = "rldicr";
            2 => Rldic = "rldic";
            3 => Rldimi = "rldimi";
        }
    }

    extended 58 {
        DS {
            0 => Ld = "ld";
            1 => Ldu = "ldu";
            2 => Lwa = "lwa";
        }
    }

    extended 62 {
        DS {
            0 => Std = "std";
            1 => Stdu = "stdu";
        }
    }

    extended 31 {
        X {
            0 => Cmp = "cmp";
            4 => Tw = "tw";
            19 => Mfcr = "mfcr";
            20 => Lwarx = "lwarx";
            21 => Ldx = "ldx";
            23 => Lwzx = "lwzx";
            24 => Slw = "slw";
            26 => Cntlzw = "cntlzw";
            27 => Sld = "sld";
            28 => And = "and";
            32 => Cmpl = "cmpl";
            53 => Ldux = "ldux";
            54 => Dcbst = "dcbst";
            55 => Lwzux = "lwzux";
            58 => Cntlzd = "cntlzd";
            60 => Andc = "andc";
            68 => Td = "td";
            83 => Mfmsr = "mfmsr";
            84 => Ldarx = "ldarx";
            86 => Dcbf = "dcbf";
            87 => Lbzx = "lbzx";
            119 => Lbzux = "lbzux";
            124 => Nor = "nor";
            144 => Mtcrf = "mtcrf";
            146 => Mtmsr = "mtmsr";
            149 => Stdx = "stdx";
            150 => Stwcx = "stwcx.";
            151 => Stwx = "stwx";
            178 => Mtmsrd = "mtmsrd";
            181 => Stdux = "stdux";
            183 => Stwux = "stwux";
            214 => Stdcx = "stdcx.";
            215 => Stbx = "stbx";
            246 => Dcbtst = "dcbtst";
            247 => Stbux = "stbux";
            278 => Dcbt = "dcbt";
            279 => Lhzx = "lhzx";
            284 => Eqv = "eqv";
            311 => Lhzux = "lhzux";
            316 => Xor = "xor";
            339 => Mfspr = "mfspr";
            341 => Lwax = "lwax";
            343 => Lhax = "lhax";
            371 => Mftb = "mftb";
            373 => Lwaux = "lwaux";
            375 => Lhaux = "lhaux";
            407 => Sthx = "sthx";
            412 => Orc = "orc";
            439 => Sthux = "sthux";
            444 => Or = "or";
            467 => Mtspr = "mtspr";
            476 => Nand = "nand";
            512 => Mcrxr = "mcrxr";
            532 => Ldbrx = "ldbrx";
            533 => Lswx = "lswx";
            534 => Lwbrx = "lwbrx";
            536 => Srw = "srw";
            539 => Srd = "srd";
            597 => Lswi = "lswi";
            598 => Sync = "sync";
            660 => Stdbrx = "stdbrx";
            661 => Stswx = "stswx";
            662 => Stwbrx = "stwbrx";
            725 => Stswi = "stswi";
            790 => Lhbrx = "lhbrx";
            792 => Sraw = "sraw";
            794 => Srad = "srad";
            824 => Srawi = "srawi";
            854 => Eieio = "eieio";
            918 => Sthbrx = "sthbrx";
            922 => Extsh = "extsh";
            954 => Extsb = "extsb";
            982 => Icbi = "icbi";
            986 => Extsw = "extsw";
            1014 => Dcbz = "dcbz";
        }
        XS {
            413 => Sradi = "sradi";
        }
        XO {
            8 => Subfc = "subfc";
            9 => Mulhdu = "mulhdu";
            10 => Addc = "addc";
            11 => Mulhwu = "mulhwu";
            40 => Subf = "subf";
            73 => Mulhd = "mulhd";
            75 => Mulhw = "mulhw";
            104 => Neg = "neg";
            136 => Subfe = "subfe";
            138 => Adde = "adde";
            200 => Subfze = "subfze";
            202 => Addze = "addze";
            232 => Subfme = "subfme";
            233 => Mulld = "mulld";
            234 => Addme = "addme";
            235 => Mullw = "mullw";
            266 => Add = "add";
            457 => Divdu = "divdu";
            459 => Divwu = "divwu";
            489 => Divd = "divd";
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
                     (masks {:#010x} and {:#010x}, values {:#010x} and {:#010x})",
                    a.mnemonic,
                    b.mnemonic,
                    a.mask,
                    b.mask,
                    a.value,
                    b.value
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
    /// one declaration but are otherwise independent code. It is also what
    /// catches a form declared in the wrong order within a primary opcode.
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
