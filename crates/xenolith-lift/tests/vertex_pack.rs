//! Checking the packed vertex formats against results the hardware gives.
//!
//! The console's own vector extension is not in any instruction set an emulator
//! here implements, so the harness that runs an instruction on emulated
//! hardware cannot reach this one. What stands in for it is a set of recorded
//! starting and finishing registers, which say what the hardware does without
//! saying how, so the C emitted for the instruction is written from the
//! description and then checked against them rather than assumed.
//!
//! The formats absent from this file are absent from the model too. Packing
//! three tens and a two is not here because what its recorded results mean has
//! not been worked out, and neither is the third of the three widths, which is
//! documented as a special case and behaves like neither of the others. Both
//! are refused by the model rather than guessed at, so a title reaching one
//! reports a function it has no code for instead of drawing something wrong.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

use xenolith_lift::RUNTIME_HEADER;
use xenolith_ppc::Instruction;

/// The register the cases pack into.
const DESTINATION: u32 = 4;

/// The register the cases pack from.
const SOURCE: u32 = 3;

/// What the destination holds before an instruction runs.
///
/// A value no case produces, so a lane that comes back holding it was left
/// alone and a lane that does not was written.
const UNTOUCHED: u32 = 0xcdcd_cdcd;

/// Builds one encoding of the instruction that packs a vertex format.
///
/// The layout is the one the title this was built against uses: the format
/// three bits above the width, the width two bits above that, and how far to
/// move the result towards the front of the register down at six. Rebuilding
/// one of the title's own instructions from these fields returns the same word
/// it holds, which is what says the reading is right.
const fn packing(kind: u32, width: u32, places: u32) -> u32 {
    0x1800_0610
        | (DESTINATION << 21)
        | (SOURCE << 11)
        | (kind << 18)
        | (width << 16)
        | (places << 6)
}

/// Builds one encoding of the instruction that unpacks a vertex format.
///
/// The format sits in the field the packer uses for its own, with the low bits
/// set the way the title's own instructions set them.
const fn unpacking(kind: u32) -> u32 {
    0x1800_07f0 | (DESTINATION << 21) | (SOURCE << 11) | (kind << 16)
}

/// One recorded case: what went in, what the instruction was, what came out.
struct Case {
    what: &'static str,
    word: u32,
    from: [u32; 4],
    into: [u32; 4],
}

/// The recorded cases, as four words in and four words out.
fn cases() -> Vec<Case> {
    let mut all = colour_cases();
    all.extend(halfword_cases());
    all.extend(half_precision_cases());
    all.extend(unpack_cases());
    all
}

/// A colour, whose bytes are already sitting in the low bits of each value,
/// and the same colour moved towards the front of the register.
fn colour_cases() -> Vec<Case> {
    let colour = [0x4040_0001, 0x4040_0002, 0x4040_0003, 0x4040_0004];
    let packed = 0x0401_0203;
    let u = UNTOUCHED;

    vec![
        // Anything below the window gives nothing and anything above it gives
        // everything, and a value that is not a number counts as below.
        Case {
            what: "a colour of values far below the window",
            word: packing(0, 1, 0),
            from: [1, 2, 3, 4],
            into: [u, u, u, 0],
        },
        Case {
            what: "a colour of values either side of the window",
            word: packing(0, 1, 0),
            from: [0x4080_0000, 0x4000_0000, 0xc2f6_0000, 0x4b3b_df83],
            into: [u, u, u, 0xffff_0000],
        },
        Case {
            what: "a colour whose alpha is not a number",
            word: packing(0, 1, 0),
            from: [0x4080_0000, 0x4000_0000, 0xc2f6_0000, 0xffff_ffff],
            into: [u, u, u, 0x00ff_0000],
        },
        Case {
            what: "a colour in the window",
            word: packing(0, 1, 0),
            from: colour,
            into: [u, u, u, packed],
        },
        // Moved towards the front of the register a word at a time. The last
        // of these has moved it as far as it goes.
        Case {
            what: "a colour moved one word forward",
            word: packing(0, 1, 1),
            from: colour,
            into: [u, u, packed, u],
        },
        Case {
            what: "a colour moved two words forward",
            word: packing(0, 1, 2),
            from: colour,
            into: [u, packed, u, u],
        },
        Case {
            what: "a colour moved three words forward",
            word: packing(0, 1, 3),
            from: colour,
            into: [packed, u, u, u],
        },
        // Two words asked for where the format fills one, so the word ahead of
        // it is cleared rather than left alone.
        Case {
            what: "a colour given two words",
            word: packing(0, 2, 0),
            from: colour,
            into: [u, u, 0, packed],
        },
        Case {
            what: "a colour given two words, moved one forward",
            word: packing(0, 2, 1),
            from: colour,
            into: [u, 0, packed, u],
        },
        Case {
            what: "a colour given two words, moved two forward",
            word: packing(0, 2, 2),
            from: colour,
            into: [0, packed, u, u],
        },
        // Moved three, the cleared word falls off the front and only the packed
        // one is left.
        Case {
            what: "a colour given two words, moved three forward",
            word: packing(0, 2, 3),
            from: colour,
            into: [packed, u, u, u],
        },
    ]
}

/// Halfwords, read the same way a colour is and saturating to one above the
/// most negative rather than to it.
fn halfword_cases() -> Vec<Case> {
    let u = UNTOUCHED;

    vec![
        Case {
            what: "two halfwords far outside the window",
            word: packing(1, 1, 0),
            from: [0x4381_7e00, 0xc37c_fc00, 0x42a2_3ec8, 0x403d_b757],
            into: [u, u, u, 0x7fff_8001],
        },
        Case {
            what: "two halfwords just outside the window",
            word: packing(1, 1, 0),
            from: [0x412f_df00, 0xc09f_be00, 0x42a2_3ec8, 0x403d_b757],
            into: [u, u, u, 0x7fff_8001],
        },
        Case {
            what: "two halfwords at the ends of the window",
            word: packing(1, 1, 0),
            from: [0x4040_7fff, 0x403f_8001, 0, 0],
            into: [u, u, u, 0x7fff_8001],
        },
        Case {
            what: "two halfwords inside the window",
            word: packing(1, 1, 0),
            from: [0x4040_4000, 0x403f_c000, 0x4040_0003, 0x403f_8001],
            into: [u, u, u, 0x4000_c000],
        },
        Case {
            what: "two halfwords, one just over the top",
            word: packing(1, 1, 0),
            from: [0x4040_fffe, 0x403f_f333, 0x42a2_3ec8, 0x403d_b757],
            into: [u, u, u, 0x7fff_f333],
        },
        // Four of them fill both of the words asked for.
        Case {
            what: "four halfwords",
            word: packing(4, 2, 0),
            from: [0x403f_8001, 0x403f_fff8, 0x4040_007f, 0x4040_0000],
            into: [u, u, 0x8001_fff8, 0x007f_0000],
        },
    ]
}

/// Half precision, which is a conversion rather than a reading of bits.
fn half_precision_cases() -> Vec<Case> {
    let u = UNTOUCHED;

    vec![
        Case {
            what: "two halves of one and a half",
            word: packing(3, 1, 0),
            from: [0x3fc0_0000, 0xbfc0_0000, 0x42a2_3ec8, 0x403d_b757],
            into: [u, u, u, 0x3e00_be00],
        },
        Case {
            what: "two halves of a half",
            word: packing(3, 1, 0),
            from: [0x3f00_0000, 0xbf00_0000, 0, 0],
            into: [u, u, u, 0x3800_b800],
        },
        Case {
            what: "four halves of one and a half",
            word: packing(5, 2, 0),
            from: [0x3fc0_0000, 0xbfc0_0000, 0x3fc0_0000, 0xbfc0_0000],
            into: [u, u, 0x3e00_be00, 0x3e00_be00],
        },
        Case {
            what: "four halves of a half and one",
            word: packing(5, 2, 0),
            from: [0x3f00_0000, 0xbf00_0000, 0x3f80_0000, 0xbf80_0000],
            into: [u, u, 0x3800_b800, 0x3c00_bc00],
        },
    ]
}

/// Unpacking, which is the other direction and the one the harness that runs an
/// instruction on emulated hardware cannot reach either.
///
/// The expected values below are worked out from the reference implementation in
/// the corpus rather than from a console, and the reference stores a guest
/// register reversed in its host variable, so its host lane `n` is guest lane
/// `3 - n`. Every case here is written in guest lanes. The two agree on the lane
/// order for all five formats, which is worth saying because the reference reads
/// the four bytes of a colour through an index list, and the order of that list
/// is the only thing that could have gone the other way.
fn unpack_cases() -> Vec<Case> {
    let mut all = unpack_colour_cases();
    all.extend(unpack_halfword_cases());
    all.extend(unpack_packed_cases());
    all.extend(unpack_half_cases());
    all.extend(unpack_four_half_cases());
    all
}

/// Builds the cases for one format, from a list of what it is and what it
/// leaves behind.
///
/// The source is always the two words the format reads out of, which is the
/// second and third for the four that read a whole word and the third alone for
/// those that read half of one.
fn unpacked(kind: u32, cases: &[(&'static str, [u32; 4], [u32; 4])]) -> Vec<Case> {
    cases
        .iter()
        .map(|(what, from, into)| Case {
            what,
            word: unpacking(kind),
            from: *from,
            into: *into,
        })
        .collect()
}

/// A colour, four bytes out of the last word.
///
/// Each byte goes back into the low bits of one, so the result reads as one plus
/// the byte over eight million. The lanes come out red, green, blue, alpha,
/// which is the order a colour is stored in rather than the order it is named
/// in, and it is the reverse of the order the packing format writes them in.
fn unpack_colour_cases() -> Vec<Case> {
    unpacked(
        0,
        &[
            (
                "a colour of four bytes in the last word",
                [0, 0, 0, 0x0403_0201],
                [0x3f80_0003, 0x3f80_0002, 0x3f80_0001, 0x3f80_0004],
            ),
            (
                "a colour of the ends of the byte range",
                [0, 0, 0, 0x00ff_0000],
                [0x3f80_00ff, 0x3f80_0000, 0x3f80_0000, 0x3f80_0000],
            ),
        ],
    )
}

/// Two halfwords read as three plus each.
///
/// The value rides in the low bits and the two ends of the register are nothing
/// and one. The first case has each halfword at an end of its range, and the
/// reading of three plus each turns those into the two nearest floats either
/// side of three.
fn unpack_halfword_cases() -> Vec<Case> {
    unpacked(
        4,
        &[
            (
                "two halfwords at the ends of the range",
                [0, 0, 0, 0x8000_7fff],
                [0x403f_8000, 0x4040_7fff, 0x0000_0000, 0x3f80_0000],
            ),
            (
                "two halfwords of nothing and of less than nothing",
                [0, 0, 0, 0x0000_8000],
                [0x4040_0000, 0x403f_8000, 0x0000_0000, 0x3f80_0000],
            ),
        ],
    )
}

/// Three tens and a two, where the three are three plus each and the two is
/// one plus it.
///
/// The ends of the ten bit field are here because that is where the sign lives,
/// and both ends of the two bit field because that one carries no sign.
fn unpack_packed_cases() -> Vec<Case> {
    unpacked(
        8,
        &[
            (
                "three tens and a two of the most negative ten",
                [0, 0, 0, 0x7fff_ffff],
                [0x403f_ffff, 0x403f_ffff, 0x403f_ffff, 0x3f80_0001],
            ),
            (
                "three tens and a two of the ends of the two bit field",
                [0, 0, 0, 0xc000_03ff],
                [0x403f_ffff, 0x4040_0000, 0x4040_0000, 0x3f80_0003],
            ),
        ],
    )
}

/// Two of the half sized floats, and the three that are not numbers in ordinary
/// arithmetic: a denormal, which has no bits in its exponent, an infinite one,
/// and one that is not a number at all.
///
/// All three are here because a conversion that has only ever seen ordinary
/// values says nothing about what it does with the other three, and a title
/// reaches them. What it does with them is worth saying plainly, because none of
/// the three answers is the one a reader expects. A denormal comes out as the
/// sign on its own, so a denormal and a zero both become a zero and the
/// magnitude between them is lost. An infinite half comes out as a large finite
/// float, `0x4780_0000` for a positive one, because its exponent is not
/// special-cased and lands in the ordinary range; it does not come out infinite.
/// A half that is not a number stays one, because its mantissa does survive the
/// widening, and `0x7e00` becomes `0x47c0_0000`.
fn unpack_half_cases() -> Vec<Case> {
    unpacked(
        12,
        &[
            (
                "two halves, one the smallest denormal and one less than nothing",
                [0, 0, 0, 0x0001_8001],
                [0x0000_0000, 0x8000_0000, 0x0000_0000, 0x3f80_0000],
            ),
            (
                "two halves of the largest denormal either side of nothing",
                [0, 0, 0, 0x83ff_8003],
                [0x8000_0000, 0x8000_0000, 0x0000_0000, 0x3f80_0000],
            ),
            (
                "two halves, one infinite and one that is not a number",
                [0, 0, 0, 0x7c00_7e00],
                [0x4780_0000, 0x47c0_0000, 0x0000_0000, 0x3f80_0000],
            ),
            (
                "two halves of one and of less than one",
                [0, 0, 0, 0x3c00_bc00],
                [0x3f80_0000, 0xbf80_0000, 0x0000_0000, 0x3f80_0000],
            ),
        ],
    )
}

/// Four of them, filling the whole register, with the same three odd inputs
/// alongside the ordinary ones.
fn unpack_four_half_cases() -> Vec<Case> {
    unpacked(
        20,
        &[
            (
                "four halves of two and of less than two",
                [0, 0, 0x4000_c000, 0x4000_c000],
                [0x4000_0000, 0xc000_0000, 0x4000_0000, 0xc000_0000],
            ),
            (
                "four halves, two denormals, an infinite one and one that is not a number",
                [0, 0, 0x83ff_8003, 0x7c00_7e00],
                [0x8000_0000, 0x8000_0000, 0x4780_0000, 0x47c0_0000],
            ),
        ],
    )
}

/// The encodings the title this was built against actually holds.
///
/// These four are the reason any of this exists. An encoding read wrongly would
/// leave them refused while every case built here still passed, so they are
/// named outright rather than left to the coverage figure to notice.
#[test]
fn the_model_lifts_the_ones_a_title_holds() {
    for word in [0x1921_5ed0u32, 0x19a1_06d0, 0x180d_0ed0, 0x1816_0e90] {
        let instruction = Instruction::decode(word);
        assert!(
            xenolith_lift::is_liftable(instruction, 0),
            "the model cannot lift {word:08x}, which the title holds"
        );
        assert!(
            xenolith_lift::code_for(instruction, 0).is_some(),
            "the model has no code for {word:08x}, which the title holds"
        );
    }
}

/// The encodings the model refuses, and has to keep refusing.
///
/// A refusal leaves the function holding it unlifted, which a title reaching it
/// reports. Quietly packing one of these the way a neighbouring format packs
/// would put wrong numbers in a vertex buffer, and nothing downstream would say
/// so.
#[test]
fn the_model_refuses_what_it_cannot_say() {
    let refused = [
        (
            packing(2, 1, 0),
            "three tens and a two, whose results are unread",
        ),
        (packing(0, 3, 0), "the third width, which is a special case"),
        (packing(0, 0, 0), "no width at all, which is not allowed"),
        (packing(6, 1, 0), "a format outside the six that exist"),
        (packing(4, 1, 0), "four halfwords given room for two"),
    ];

    for (word, what) in refused {
        assert!(
            xenolith_lift::code_for(Instruction::decode(word), 0).is_none(),
            "the model wrote code for {what} ({word:08x}) rather than refusing it"
        );
    }
}

/// A function holding a vertex pack lifts whole and its C compiles.
///
/// The instruction alone is checked above. This is the claim a title depends on:
/// the function it sits in is emitted rather than refused.
#[test]
fn a_function_holding_a_pack_lifts_and_compiles() {
    use xenolith_analysis::analyze;
    use xenolith_lift::Imports;
    use xenolith_xex::{Image, PageKind, Section};

    const BASE: u32 = 0x8200_0000;

    // The pack, then a return.
    let words = [packing(0, 1, 0), 0x4e80_0020];
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
    let size = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    let sections = vec![Section {
        start: BASE,
        size,
        kind: PageKind::Code,
    }];
    let image = Image::new(BASE, bytes, sections).with_entry_point(Some(BASE));

    let program = analyze(&image, &[]);
    let function = program
        .functions()
        .find(|function| function.start == BASE)
        .expect("nothing was discovered at the entry point");
    let lifted = xenolith_lift::lift(&image, function, &Imports::new())
        .expect("a function holding a vertex pack should lift");

    assert!(
        lifted.code.contains("xenolith_pack_colour"),
        "the pack was not emitted: {}",
        lifted.code
    );

    let Some(compiler) = host_compiler() else {
        eprintln!("skipping the compile: no host C compiler is installed");
        return;
    };
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("vertex-pack-function");
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a directory to build in");
    std::fs::write(directory.join("xenolith.h"), RUNTIME_HEADER).expect("writing the header");
    let program = format!(
        "#include \"xenolith.h\"\n\n\
         void xenolith_dispatch(xenolith_context *c, uint8_t *b, uint32_t a, uint32_t f) {{ (void)c; (void)b; (void)a; (void)f; }}\n\
         void xenolith_trap(xenolith_context *c, uint8_t *b, uint32_t a) {{ (void)c; (void)b; (void)a; }}\n\
         void xenolith_import(xenolith_context *c, uint8_t *b, const char *l, uint32_t o, const char *n) {{ (void)c; (void)b; (void)l; (void)o; (void)n; }}\n\
         uint32_t xenolith_reserve32(const uint8_t *b, uint32_t a) {{ return xenolith_load32(b, a); }}\n\
         uint64_t xenolith_reserve64(const uint8_t *b, uint32_t a) {{ return xenolith_load64(b, a); }}\n\
         uint8_t xenolith_conditional32(uint8_t *b, uint32_t a, uint32_t v) {{ xenolith_store32(b, a, v); return 1; }}\n\
         uint8_t xenolith_conditional64(uint8_t *b, uint32_t a, uint64_t v) {{ xenolith_store64(b, a, v); return 1; }}\n\
         uint64_t xenolith_timebase(void) {{ return 0; }}\n\n\
         {}",
        lifted.code
    );
    std::fs::write(directory.join("lifted.c"), &program).expect("writing the code");

    let built = Command::new(compiler)
        .args(["-std=c17", "-c", "-O1", "-I"])
        .arg(&directory)
        .arg("-o")
        .arg(directory.join("lifted.o"))
        .arg(directory.join("lifted.c"))
        .output()
        .expect("running the host compiler");
    assert!(
        built.status.success(),
        "the emitted C did not compile:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
}

/// Returns a host C compiler, or nothing where none is installed.
fn host_compiler() -> Option<&'static str> {
    ["clang", "cc", "gcc"].into_iter().find(|candidate| {
        Command::new(candidate)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    })
}

/// Writes the C that runs every case and prints what each left behind.
fn program(cases: &[Case]) -> Option<String> {
    let mut out = String::from("#include <stdio.h>\n#include \"xenolith.h\"\n\nint main(void) {\n");

    for case in cases {
        let code = xenolith_lift::code_for(Instruction::decode(case.word), 0)?;
        out.push_str("    {\n        xenolith_context state = {0};\n");
        out.push_str("        xenolith_context *ctx = &state;\n");
        for (lane, word) in case.from.iter().enumerate() {
            let _ = writeln!(
                out,
                "        xenolith_vector_set_u32(&state.v[{SOURCE}], {lane}, {word}u);"
            );
        }
        for lane in 0..4 {
            let _ = writeln!(
                out,
                "        xenolith_vector_set_u32(&state.v[{DESTINATION}], {lane}, {UNTOUCHED}u);"
            );
        }
        out.push_str(&code);
        let _ = writeln!(
            out,
            "        for (unsigned lane = 0; lane < 4; lane++) {{ printf(\"%08x \", xenolith_vector_u32(&state.v[{DESTINATION}], lane)); }}"
        );
        out.push_str("        printf(\"\\n\");\n    }\n");
    }

    out.push_str("    return 0;\n}\n");
    Some(out)
}

/// Builds and runs the cases, returning a line of four words for each.
fn on_the_model(cases: &[Case]) -> Result<Vec<Vec<u32>>, String> {
    let Some(compiler) = host_compiler() else {
        return Err("skip: no host C compiler is installed".to_owned());
    };
    let Some(source) = program(cases) else {
        return Err("the model has no code for an encoding under test".to_owned());
    };

    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("vertex-pack");
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).map_err(|error| format!("making a directory: {error}"))?;
    std::fs::write(directory.join("xenolith.h"), RUNTIME_HEADER)
        .map_err(|error| format!("writing the header: {error}"))?;
    std::fs::write(directory.join("pack.c"), source)
        .map_err(|error| format!("writing the program: {error}"))?;

    let built = Command::new(compiler)
        .args(["-std=c17", "-O1", "-I"])
        .arg(&directory)
        .arg("-o")
        .arg(directory.join("pack"))
        .arg(directory.join("pack.c"))
        .arg("-lm")
        .output()
        .map_err(|error| format!("running the host compiler: {error}"))?;
    if !built.status.success() {
        return Err(format!(
            "the program did not build:\n{}",
            String::from_utf8_lossy(&built.stderr)
        ));
    }

    let ran = Command::new(directory.join("pack"))
        .output()
        .map_err(|error| format!("running the program: {error}"))?;

    String::from_utf8_lossy(&ran.stdout)
        .lines()
        .map(|line| {
            line.split_whitespace()
                .map(|word| {
                    u32::from_str_radix(word, 16)
                        .map_err(|error| format!("reading {word} back: {error}"))
                })
                .collect()
        })
        .collect()
}

/// Every recorded case, compared lane by lane against the model.
#[test]
fn the_model_packs_what_the_hardware_packs() {
    let cases = cases();
    let got = match on_the_model(&cases) {
        Ok(got) => got,
        Err(complaint) if complaint.starts_with("skip: ") => {
            eprintln!("{complaint}");
            return;
        }
        Err(complaint) => panic!("{complaint}"),
    };
    assert_eq!(got.len(), cases.len(), "one line per case was expected");

    let mut wrong = Vec::new();
    for (case, line) in cases.iter().zip(&got) {
        if line.as_slice() != case.into.as_slice() {
            wrong.push(format!(
                "{}\n      hardware {:08x?}\n      model    {:08x?}",
                case.what, case.into, line
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "the model packs or unpacks {} of {} cases differently from the recorded values:\n\n  {}",
        wrong.len(),
        cases.len(),
        wrong.join("\n\n  ")
    );
}
