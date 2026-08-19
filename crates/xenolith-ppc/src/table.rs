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
        extension {
            $( $x_form:ident { $( $x_value:literal => $x_var:ident = $x_mn:literal; )* } )*
        }
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
            $( $( #[doc = concat!("`", $x_mn, "`")] $x_var, )* )*
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
            $( $( Entry {
                opcode: Opcode::$x_var,
                mnemonic: $x_mn,
                form: Form::$x_form,
                mask: Form::$x_form.mask(),
                value: $x_value,
            }, )* )*
        ];

        /// Identifies an instruction of the console's vector extension.
        ///
        /// These forms scatter their identifying bits instead of gathering them
        /// into one field, so each is matched against its own mask rather than
        /// by reading a field out of the word. Forms are tried widest mask
        /// first, the same precedence the ambiguity test enforces.
        const fn decode_console_extension(word: u32) -> Opcode {
            $(
                match word & Form::$x_form.mask() {
                    $( $x_value => return Opcode::$x_var, )*
                    _ => {}
                }
            )*
            Opcode::Unknown
        }

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
                    decode_console_extension(word)
                } )*
                _ => decode_console_extension(word),
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
            48 => Lfs = "lfs";
            49 => Lfsu = "lfsu";
            50 => Lfd = "lfd";
            51 => Lfdu = "lfdu";
            52 => Stfs = "stfs";
            53 => Stfsu = "stfsu";
            54 => Stfd = "stfd";
            55 => Stfdu = "stfdu";
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

    extended 4 {
        VX {
            0 => Vaddubm = "vaddubm";
            2 => Vmaxub = "vmaxub";
            4 => Vrlb = "vrlb";
            10 => Vaddfp = "vaddfp";
            12 => Vmrghb = "vmrghb";
            14 => Vpkuhum = "vpkuhum";
            64 => Vadduhm = "vadduhm";
            66 => Vmaxuh = "vmaxuh";
            68 => Vrlh = "vrlh";
            74 => Vsubfp = "vsubfp";
            76 => Vmrghh = "vmrghh";
            78 => Vpkuwum = "vpkuwum";
            128 => Vadduwm = "vadduwm";
            130 => Vmaxuw = "vmaxuw";
            132 => Vrlw = "vrlw";
            140 => Vmrghw = "vmrghw";
            142 => Vpkuhus = "vpkuhus";
            206 => Vpkuwus = "vpkuwus";
            258 => Vmaxsb = "vmaxsb";
            260 => Vslb = "vslb";
            268 => Vmrglb = "vmrglb";
            270 => Vpkshus = "vpkshus";
            266 => Vrefp = "vrefp";
            322 => Vmaxsh = "vmaxsh";
            324 => Vslh = "vslh";
            330 => Vrsqrtefp = "vrsqrtefp";
            332 => Vmrglh = "vmrglh";
            334 => Vpkswus = "vpkswus";
            386 => Vmaxsw = "vmaxsw";
            388 => Vslw = "vslw";
            394 => Vexptefp = "vexptefp";
            396 => Vmrglw = "vmrglw";
            398 => Vpkshss = "vpkshss";
            452 => Vsl = "vsl";
            458 => Vlogefp = "vlogefp";
            462 => Vpkswss = "vpkswss";
            512 => Vaddubs = "vaddubs";
            514 => Vminub = "vminub";
            516 => Vsrb = "vsrb";
            522 => Vrfin = "vrfin";
            524 => Vspltb = "vspltb";
            526 => Vupkhsb = "vupkhsb";
            576 => Vadduhs = "vadduhs";
            578 => Vminuh = "vminuh";
            580 => Vsrh = "vsrh";
            586 => Vrfiz = "vrfiz";
            588 => Vsplth = "vsplth";
            590 => Vupkhsh = "vupkhsh";
            640 => Vadduws = "vadduws";
            642 => Vminuw = "vminuw";
            644 => Vsrw = "vsrw";
            650 => Vrfip = "vrfip";
            652 => Vspltw = "vspltw";
            654 => Vupklsb = "vupklsb";
            708 => Vsr = "vsr";
            714 => Vrfim = "vrfim";
            718 => Vupklsh = "vupklsh";
            768 => Vaddsbs = "vaddsbs";
            770 => Vminsb = "vminsb";
            772 => Vsrab = "vsrab";
            778 => Vcfux = "vcfux";
            780 => Vspltisb = "vspltisb";
            782 => Vpkpx = "vpkpx";
            832 => Vaddshs = "vaddshs";
            834 => Vminsh = "vminsh";
            836 => Vsrah = "vsrah";
            842 => Vcfsx = "vcfsx";
            844 => Vspltish = "vspltish";
            846 => Vupkhpx = "vupkhpx";
            896 => Vaddsws = "vaddsws";
            898 => Vminsw = "vminsw";
            900 => Vsraw = "vsraw";
            906 => Vctuxs = "vctuxs";
            908 => Vspltisw = "vspltisw";
            970 => Vctsxs = "vctsxs";
            974 => Vupklpx = "vupklpx";
            1024 => Vsububm = "vsububm";
            1026 => Vavgub = "vavgub";
            1028 => Vand = "vand";
            1034 => Vmaxfp = "vmaxfp";
            1036 => Vslo = "vslo";
            1088 => Vsubuhm = "vsubuhm";
            1090 => Vavguh = "vavguh";
            1092 => Vandc = "vandc";
            1098 => Vminfp = "vminfp";
            1100 => Vsro = "vsro";
            1152 => Vsubuwm = "vsubuwm";
            1154 => Vavguw = "vavguw";
            1156 => Vor = "vor";
            1220 => Vxor = "vxor";
            1282 => Vavgsb = "vavgsb";
            1284 => Vnor = "vnor";
            1346 => Vavgsh = "vavgsh";
            1410 => Vavgsw = "vavgsw";
            1536 => Vsububs = "vsububs";
            1540 => Mfvscr = "mfvscr";
            1600 => Vsubuhs = "vsubuhs";
            1604 => Mtvscr = "mtvscr";
            1664 => Vsubuws = "vsubuws";
            1792 => Vsubsbs = "vsubsbs";
            1856 => Vsubshs = "vsubshs";
            1920 => Vsubsws = "vsubsws";
        }
        VC {
            6 => Vcmpequb = "vcmpequb";
            70 => Vcmpequh = "vcmpequh";
            134 => Vcmpequw = "vcmpequw";
            198 => Vcmpeqfp = "vcmpeqfp";
            454 => Vcmpgefp = "vcmpgefp";
            518 => Vcmpgtub = "vcmpgtub";
            582 => Vcmpgtuh = "vcmpgtuh";
            646 => Vcmpgtuw = "vcmpgtuw";
            710 => Vcmpgtfp = "vcmpgtfp";
            774 => Vcmpgtsb = "vcmpgtsb";
            838 => Vcmpgtsh = "vcmpgtsh";
            902 => Vcmpgtsw = "vcmpgtsw";
        }
        VA {
            32 => Vmhaddshs = "vmhaddshs";
            33 => Vmhraddshs = "vmhraddshs";
            34 => Vmladduhm = "vmladduhm";
            36 => Vmsumubm = "vmsumubm";
            37 => Vmsummbm = "vmsummbm";
            38 => Vmsumuhm = "vmsumuhm";
            39 => Vmsumuhs = "vmsumuhs";
            40 => Vmsumshm = "vmsumshm";
            41 => Vmsumshs = "vmsumshs";
            42 => Vsel = "vsel";
            43 => Vperm = "vperm";
            44 => Vsldoi = "vsldoi";
            46 => Vmaddfp = "vmaddfp";
            47 => Vnmsubfp = "vnmsubfp";
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

    extended 59 {
        A {
            18 => Fdivs = "fdivs";
            20 => Fsubs = "fsubs";
            21 => Fadds = "fadds";
            22 => Fsqrts = "fsqrts";
            24 => Fres = "fres";
            25 => Fmuls = "fmuls";
            26 => Frsqrtes = "frsqrtes";
            28 => Fmsubs = "fmsubs";
            29 => Fmadds = "fmadds";
            30 => Fnmsubs = "fnmsubs";
            31 => Fnmadds = "fnmadds";
        }
    }

    extended 63 {
        X {
            0 => Fcmpu = "fcmpu";
            12 => Frsp = "frsp";
            14 => Fctiw = "fctiw";
            15 => Fctiwz = "fctiwz";
            32 => Fcmpo = "fcmpo";
            38 => Mtfsb1 = "mtfsb1";
            40 => Fneg = "fneg";
            64 => Mcrfs = "mcrfs";
            70 => Mtfsb0 = "mtfsb0";
            72 => Fmr = "fmr";
            134 => Mtfsfi = "mtfsfi";
            136 => Fnabs = "fnabs";
            264 => Fabs = "fabs";
            583 => Mffs = "mffs";
            711 => Mtfsf = "mtfsf";
            814 => Fctid = "fctid";
            815 => Fctidz = "fctidz";
            846 => Fcfid = "fcfid";
        }
        A {
            18 => Fdiv = "fdiv";
            20 => Fsub = "fsub";
            21 => Fadd = "fadd";
            22 => Fsqrt = "fsqrt";
            23 => Fsel = "fsel";
            24 => Fre = "fre";
            25 => Fmul = "fmul";
            26 => Frsqrte = "frsqrte";
            28 => Fmsub = "fmsub";
            29 => Fmadd = "fmadd";
            30 => Fnmsub = "fnmsub";
            31 => Fnmadd = "fnmadd";
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
            6 => Lvsl = "lvsl";
            7 => Lvebx = "lvebx";
            19 => Mfcr = "mfcr";
            20 => Lwarx = "lwarx";
            21 => Ldx = "ldx";
            23 => Lwzx = "lwzx";
            24 => Slw = "slw";
            26 => Cntlzw = "cntlzw";
            27 => Sld = "sld";
            28 => And = "and";
            32 => Cmpl = "cmpl";
            38 => Lvsr = "lvsr";
            39 => Lvehx = "lvehx";
            71 => Lvewx = "lvewx";
            53 => Ldux = "ldux";
            54 => Dcbst = "dcbst";
            55 => Lwzux = "lwzux";
            58 => Cntlzd = "cntlzd";
            60 => Andc = "andc";
            68 => Td = "td";
            83 => Mfmsr = "mfmsr";
            103 => Lvx = "lvx";
            84 => Ldarx = "ldarx";
            86 => Dcbf = "dcbf";
            87 => Lbzx = "lbzx";
            119 => Lbzux = "lbzux";
            124 => Nor = "nor";
            135 => Stvebx = "stvebx";
            144 => Mtcrf = "mtcrf";
            146 => Mtmsr = "mtmsr";
            149 => Stdx = "stdx";
            150 => Stwcx = "stwcx.";
            151 => Stwx = "stwx";
            167 => Stvehx = "stvehx";
            178 => Mtmsrd = "mtmsrd";
            199 => Stvewx = "stvewx";
            181 => Stdux = "stdux";
            183 => Stwux = "stwux";
            214 => Stdcx = "stdcx.";
            215 => Stbx = "stbx";
            231 => Stvx = "stvx";
            246 => Dcbtst = "dcbtst";
            247 => Stbux = "stbux";
            278 => Dcbt = "dcbt";
            279 => Lhzx = "lhzx";
            284 => Eqv = "eqv";
            311 => Lhzux = "lhzux";
            316 => Xor = "xor";
            339 => Mfspr = "mfspr";
            341 => Lwax = "lwax";
            342 => Dst = "dst";
            343 => Lhax = "lhax";
            371 => Mftb = "mftb";
            359 => Lvxl = "lvxl";
            373 => Lwaux = "lwaux";
            374 => Dstst = "dstst";
            375 => Lhaux = "lhaux";
            407 => Sthx = "sthx";
            412 => Orc = "orc";
            439 => Sthux = "sthux";
            487 => Stvxl = "stvxl";
            444 => Or = "or";
            467 => Mtspr = "mtspr";
            476 => Nand = "nand";
            512 => Mcrxr = "mcrxr";
            519 => Lvlx = "lvlx";
            532 => Ldbrx = "ldbrx";
            535 => Lfsx = "lfsx";
            551 => Lvrx = "lvrx";
            567 => Lfsux = "lfsux";
            599 => Lfdx = "lfdx";
            631 => Lfdux = "lfdux";
            663 => Stfsx = "stfsx";
            679 => Stvrx = "stvrx";
            695 => Stfsux = "stfsux";
            727 => Stfdx = "stfdx";
            759 => Stfdux = "stfdux";
            983 => Stfiwx = "stfiwx";
            533 => Lswx = "lswx";
            534 => Lwbrx = "lwbrx";
            536 => Srw = "srw";
            539 => Srd = "srd";
            597 => Lswi = "lswi";
            598 => Sync = "sync";
            647 => Stvlx = "stvlx";
            660 => Stdbrx = "stdbrx";
            661 => Stswx = "stswx";
            662 => Stwbrx = "stwbrx";
            725 => Stswi = "stswi";
            775 => Lvlxl = "lvlxl";
            790 => Lhbrx = "lhbrx";
            792 => Sraw = "sraw";
            794 => Srad = "srad";
            807 => Lvrxl = "lvrxl";
            822 => Dss = "dss";
            824 => Srawi = "srawi";
            854 => Eieio = "eieio";
            903 => Stvlxl = "stvlxl";
            918 => Sthbrx = "sthbrx";
            922 => Extsh = "extsh";
            935 => Stvrxl = "stvrxl";
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

    extension {
        Vx128Un {
            0x1800_0330 => Vrfim128 = "vrfim128";
            0x1800_0370 => Vrfin128 = "vrfin128";
            0x1800_0380 => Vupkhsb128 = "vupkhsb128";
            0x1800_03b0 => Vrfip128 = "vrfip128";
            0x1800_03c0 => Vupklsb128 = "vupklsb128";
            0x1800_03f0 => Vrfiz128 = "vrfiz128";
            0x1800_0630 => Vrefp128 = "vrefp128";
            0x1800_0670 => Vrsqrtefp128 = "vrsqrtefp128";
            0x1800_06b0 => Vexptefp128 = "vexptefp128";
            0x1800_06f0 => Vlogefp128 = "vlogefp128";
        }
        Vx128Ls {
            0x1000_0003 => Lvsl128 = "lvsl128";
            0x1000_0043 => Lvsr128 = "lvsr128";
            0x1000_0083 => Lvewx128 = "lvewx128";
            0x1000_00c3 => Lvx128 = "lvx128";
            0x1000_01c3 => Stvx128 = "stvx128";
            0x1000_02c3 => Lvxl128 = "lvxl128";
            0x1000_0303 => Stewx128 = "stewx128";
            0x1000_03c3 => Stvxl128 = "stvxl128";
            0x1000_0403 => Lvlx128 = "lvlx128";
            0x1000_0443 => Lvrx128 = "lvrx128";
            0x1000_0503 => Stvlx128 = "stvlx128";
            0x1000_0543 => Stvrx128 = "stvrx128";
            0x1000_0603 => Lvlxl128 = "lvlxl128";
            0x1000_0643 => Lvrxl128 = "lvrxl128";
            0x1000_0703 => Stvlxl128 = "stvlxl128";
            0x1000_0743 => Stvrxl128 = "stvrxl128";
        }
        Vx128Cv {
            0x1800_0230 => Vcfpsxws128 = "vcfpsxws128";
            0x1800_0270 => Vcfpuxws128 = "vcfpuxws128";
            0x1800_02b0 => Vcsxwfp128 = "vcsxwfp128";
            0x1800_02f0 => Vcuxwfp128 = "vcuxwfp128";
            0x1800_0730 => Vspltw128 = "vspltw128";
            0x1800_0770 => Vspltisw128 = "vspltisw128";
            0x1800_07f0 => Vupkd3d128 = "vupkd3d128";
        }
        Vx128 {
            0x1400_0010 => Vaddfp128 = "vaddfp128";
            0x1400_0050 => Vsubfp128 = "vsubfp128";
            0x1400_0090 => Vmulfp128 = "vmulfp128";
            0x1400_00d0 => Vmaddfp128 = "vmaddfp128";
            0x1400_0110 => Vmaddcfp128 = "vmaddcfp128";
            0x1400_0150 => Vnmsubfp128 = "vnmsubfp128";
            0x1400_0190 => Vmsum3fp128 = "vmsum3fp128";
            0x1400_01d0 => Vmsum4fp128 = "vmsum4fp128";
            0x1400_0200 => Vpkshss128 = "vpkshss128";
            0x1400_0210 => Vand128 = "vand128";
            0x1400_0240 => Vpkshus128 = "vpkshus128";
            0x1400_0280 => Vpkswss128 = "vpkswss128";
            0x1400_02c0 => Vpkswus128 = "vpkswus128";
            0x1400_02d0 => Vor128 = "vor128";
            0x1400_0300 => Vpkuhum128 = "vpkuhum128";
            0x1400_0310 => Vxor128 = "vxor128";
            0x1400_0340 => Vpkuhus128 = "vpkuhus128";
            0x1400_0350 => Vsel128 = "vsel128";
            0x1400_0380 => Vpkuwum128 = "vpkuwum128";
            0x1400_0390 => Vslo128 = "vslo128";
            0x1400_03c0 => Vpkuwus128 = "vpkuwus128";
            0x1800_00d0 => Vslw128 = "vslw128";
            0x1800_0150 => Vsraw128 = "vsraw128";
            0x1800_01d0 => Vsrw128 = "vsrw128";
            0x1800_0280 => Vmaxfp128 = "vmaxfp128";
            0x1800_02c0 => Vminfp128 = "vminfp128";
            0x1800_0300 => Vmrghw128 = "vmrghw128";
            0x1800_0340 => Vmrglw128 = "vmrglw128";
        }
        Vx128Ri {
            0x1800_0610 => Vpkd3d128 = "vpkd3d128";
            0x1800_0710 => Vrlimi128 = "vrlimi128";
        }
        Vx128Cmp {
            0x1800_0000 => Vcmpeqfp128 = "vcmpeqfp128";
            0x1800_0080 => Vcmpgefp128 = "vcmpgefp128";
            0x1800_0100 => Vcmpgtfp128 = "vcmpgtfp128";
            0x1800_0180 => Vcmpbfp128 = "vcmpbfp128";
            0x1800_0200 => Vcmpequw128 = "vcmpequw128";
        }
        Vx128Pwi {
            0x1800_0210 => Vpermwi128 = "vpermwi128";
        }
        Vx128Perm {
            0x1400_0000 => Vperm128 = "vperm128";
        }
        Vx128Sd {
            0x1000_0010 => Vsldoi128 = "vsldoi128";
        }
    }
}

impl Opcode {
    /// Returns the table entry for this opcode, if it has one.
    ///
    /// The macro emits variants in the same order as it emits table entries,
    /// with the unknown variant first, so the discriminant indexes the table
    /// directly. Searching for it instead would put a scan of the whole table
    /// behind every operand and control flow query, which the analysis stages
    /// make once per instruction over a whole image.
    pub(crate) fn entry(self) -> Option<&'static Entry> {
        TABLE.get((self as usize).checked_sub(1)?)
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

    /// Returns every operation the decoder recognizes, in declaration order.
    ///
    /// The unknown one is not among them, since it names the absence of an
    /// operation rather than one. This is what lets a later stage say which
    /// instructions it does not yet handle by comparing against the whole set
    /// rather than against the ones it happened to meet.
    pub fn all() -> impl Iterator<Item = Self> {
        TABLE.iter().map(|entry| entry.opcode)
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

    /// The direct indexing above is only sound while the two orders agree, and
    /// nothing in the macro forces that beyond it emitting both from the same
    /// declarations. This is what holds it.
    #[test]
    fn opcode_discriminants_index_the_table() {
        for (index, entry) in TABLE.iter().enumerate() {
            assert_eq!(
                entry.opcode as usize,
                index + 1,
                "{} sits at table index {index} but has discriminant {}",
                entry.mnemonic,
                entry.opcode as usize
            );
        }
        assert_eq!(Opcode::Unknown as usize, 0);
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
