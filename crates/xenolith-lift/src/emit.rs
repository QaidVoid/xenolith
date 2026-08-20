//! Turning a discovered function into C.
//!
//! What an instruction touches is checkable against an independent corpus. What
//! it computes is not, because comparing two expressions for equivalence is
//! harder than the problem being solved. The two are therefore kept apart: an
//! instruction may have an effect and still have no code, and a function is
//! emitted only when every instruction in it has both.
//!
//! The emitted C is deliberately plain. Blocks become labels and edges become
//! branches to them, because that is what the control flow graph says and
//! anything else is a transformation that would have to be justified. Each
//! instruction is emitted under its own disassembly, so the two can be read
//! against each other by someone looking for a mistake, which is the only
//! review this code can get until it can be run.

use std::fmt::Write as _;

use xenolith_analysis::{Function, Terminator};
use xenolith_ppc::{FlowKind, Instruction, Opcode};
use xenolith_xex::Image;

use crate::effect::effect_of;

/// Bytes one instruction occupies.
const INSTRUCTION_SIZE: u32 = 4;

/// A function turned into C, and what that C refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lifted {
    /// The C itself.
    pub code: String,
    /// Addresses the emitted code calls.
    ///
    /// Not every one is a discovered function. A call into a register save
    /// helper lands partway through one, and a tail call or a switch may leave
    /// for somewhere discovery did not claim. All of them still have to be
    /// declared or the C will not compile, so the emitter reports what it named
    /// rather than leaving the caller to guess.
    pub calls: std::collections::BTreeSet<u32>,
}

/// An imported function a thunk stands for.
///
/// The container names an import by ordinal within a library and never by name,
/// so this is everything known about which function is meant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// Name of the library it comes from, such as `xboxkrnl.exe`.
    pub library: String,
    /// Ordinal within that library.
    pub ordinal: u16,
}

/// Import thunks, keyed by the address of the thunk.
///
/// The name is owned rather than borrowed because the container it came from is
/// read and dropped well before anything is emitted.
pub type Imports = std::collections::BTreeMap<u32, Imported>;

/// Why a function was not emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unlifted {
    /// Where the function begins.
    pub function: u32,
    /// The instruction that stopped it.
    pub address: u32,
    /// What that instruction is.
    pub mnemonic: &'static str,
}

/// Returns the name a lifted function goes by.
///
/// Derived from the address so that a caller can be matched to a callee without
/// a table on the side.
#[must_use]
pub fn name_of(address: u32) -> String {
    format!("sub_{address:08x}")
}

/// Returns the declaration of a lifted function.
///
/// Every discovered function is declared, whether or not it could be lifted,
/// because a function that lifts may still call one that did not. Leaving those
/// out would fail to compile; leaving them in fails to link, which is the truer
/// statement of what is missing.
#[must_use]
pub fn declaration_of(address: u32) -> String {
    format!(
        "void {}(xenolith_context *ctx, uint8_t *base);\n",
        name_of(address)
    )
}

/// Returns the label a block goes by.
fn label_of(address: u32) -> String {
    format!("loc_{address:08x}")
}

/// Returns the vector register an instruction names.
///
/// The console's extension reaches further than the ordinary field can hold and
/// splits the number across the word. The instructions it extends still use the
/// ordinary field.
fn vector_register(instruction: Instruction) -> u8 {
    if instruction
        .form()
        .is_some_and(xenolith_ppc::Form::is_console_extension)
    {
        instruction.vector_d()
    } else {
        instruction.rt()
    }
}

/// Returns the mask a word rotate keeps, given where it begins and ends.
///
/// Bits are numbered from the most significant, and a mask whose end comes
/// before its beginning wraps around rather than being empty.
fn rotate_mask(begin: u8, end: u8) -> u32 {
    let bit = |at: u8| 1u32 << (31 - u32::from(at.min(31)));
    let mut mask = 0u32;
    let mut at = begin;

    loop {
        mask |= bit(at);
        if at == end {
            break;
        }
        at = if at == 31 { 0 } else { at + 1 };
    }

    mask
}

/// Writes the statements that set a condition field from a comparison.
///
/// The less than expression is given rather than derived, because an unsigned
/// comparison against zero is never true and a compiler asked for warnings says
/// so once for every occurrence.
fn compare(into: &mut String, field: u8, left: &str, right: &str, less: &str) {
    let _ = writeln!(
        into,
        "    ctx->cr[{field}].lt = {less};\n    \
         ctx->cr[{field}].gt = ({left}) > ({right});\n    \
         ctx->cr[{field}].eq = ({left}) == ({right});\n    \
         ctx->cr[{field}].so = (uint8_t)(ctx->xer >> 31) & 1;"
    );
}

/// Returns the bits a field mask selects, four per field.
///
/// The mask's most significant bit names the first field, which is the most
/// significant four bits of the register it selects within.
fn nibble_mask(mask: u8) -> u64 {
    let mut bits = 0u64;
    for field in 0..8u32 {
        if mask & (1 << (7 - field)) != 0 {
            bits |= 0xfu64 << (28 - 4 * field);
        }
    }
    bits
}

/// Returns how many bytes a byte reversed access moves.
fn byte_reversed_width(opcode: Opcode) -> Option<u32> {
    Some(match opcode {
        Opcode::Lhbrx | Opcode::Sthbrx => 2,
        Opcode::Lwbrx | Opcode::Stwbrx => 4,
        Opcode::Ldbrx | Opcode::Stdbrx => 8,
        _ => return None,
    })
}

/// Returns the condition register bit a number names.
///
/// Bits are numbered across the whole register rather than within a field, so
/// the field is the number divided by four and the bit within it the remainder,
/// in the order less than, greater than, equal, summary overflow.
fn condition_bit(number: u32) -> String {
    let name = match number & 3 {
        0 => "lt",
        1 => "gt",
        2 => "eq",
        _ => "so",
    };
    format!("ctx->cr[{}].{name}", number >> 2)
}

/// Returns the effective address of an indexed access.
///
/// Register zero names no register in this position rather than naming the
/// register numbered zero, which is why the base is dropped rather than read.
fn indexed_address(ra: u32, rb: u32) -> String {
    if ra == 0 {
        format!("(uint32_t)ctx->r[{rb}]")
    } else {
        format!("(uint32_t)ctx->r[{ra}] + (uint32_t)ctx->r[{rb}]")
    }
}

/// Returns the vector registers an instruction names, as destination and three
/// sources.
///
/// The console's extension reaches four times as many registers by scattering
/// the extra bits across the word, so which bits hold a register number depends
/// on the form rather than on the position.
///
/// Its forms carry three register fields where the standard ones carry four, so
/// an instruction of its own needing three sources has to take one of them from
/// the field it writes. The destination is therefore also the third source.
fn vector_operands(instruction: Instruction) -> (u32, u32, u32, u32) {
    if instruction
        .form()
        .is_some_and(xenolith_ppc::Form::is_console_extension)
    {
        (
            u32::from(instruction.vector_d()),
            u32::from(instruction.vector_a()),
            u32::from(instruction.vector_b()),
            u32::from(instruction.vector_d()),
        )
    } else {
        (
            u32::from(instruction.rt()),
            u32::from(instruction.ra()),
            u32::from(instruction.rb()),
            (instruction.word() >> 6) & 0x1f,
        )
    }
}

/// Writes a lane by lane operation through a temporary.
///
/// The temporary is not something to optimize away. An instruction may name its
/// destination as one of its sources, and a merge or a splat reads lanes the
/// loop has already passed. Building the result somewhere else makes that
/// impossible rather than making it depend on the order the lanes are visited
/// in.
fn vector_lanes(out: &mut String, destination: u32, count: u32, width: &str, body: &str) {
    let _ = writeln!(out, "    {{ xenolith_vector t;");
    let _ = writeln!(
        out,
        "    for (unsigned lane = 0; lane < {count}; lane++) {{"
    );
    let _ = writeln!(
        out,
        "        xenolith_vector_set_{width}(&t, lane, {body});"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    ctx->v[{destination}] = t; }}");
}

/// Returns the lane width a vector operation works at, as the name its
/// accessors go by and how many of them fit in a register.
fn lane_width(width: u32) -> (&'static str, u32) {
    match width {
        1 => ("u8", 16),
        2 => ("u16", 8),
        8 => ("u64", 2),
        _ => ("u32", 4),
    }
}

/// Returns the C for a vector instruction, if it can be written.
///
/// The families are tried in turn and each answers only for the opcodes it
/// knows. They were once written to fall through to one another, which twice
/// left a family unreachable when a new one was spliced into the middle: the
/// instructions it covered went back to being refused and the coverage figure
/// was the only thing that noticed.
fn vector_code(instruction: Instruction) -> Option<String> {
    vector_bitwise(instruction)
        .or_else(|| vector_arrangement(instruction))
        .or_else(|| vector_float(instruction))
        .or_else(|| vector_convert(instruction))
        .or_else(|| vector_compare(instruction))
        .or_else(|| vector_integer(instruction))
        .or_else(|| vector_pack(instruction))
        .or_else(|| vector_access(instruction))
}

/// Returns the C for a vector instruction that combines lanes bit by bit.
fn vector_bitwise(instruction: Instruction) -> Option<String> {
    let (d, a, b, c) = vector_operands(instruction);
    let mut out = String::new();

    let at = |register: u32, width: &str, lane: &str| {
        format!("xenolith_vector_{width}(&ctx->v[{register}], {lane})")
    };

    match instruction.opcode() {
        // The bitwise operations are the same whatever width the lanes are
        // read at, so they are done a word at a time.
        Opcode::Vand | Opcode::Vand128 => {
            let body = format!("{} & {}", at(a, "u32", "lane"), at(b, "u32", "lane"));
            vector_lanes(&mut out, d, 4, "u32", &body);
        }
        Opcode::Vandc => {
            let body = format!("{} & ~{}", at(a, "u32", "lane"), at(b, "u32", "lane"));
            vector_lanes(&mut out, d, 4, "u32", &body);
        }
        Opcode::Vor | Opcode::Vor128 => {
            let body = format!("{} | {}", at(a, "u32", "lane"), at(b, "u32", "lane"));
            vector_lanes(&mut out, d, 4, "u32", &body);
        }
        Opcode::Vnor => {
            let body = format!("~({} | {})", at(a, "u32", "lane"), at(b, "u32", "lane"));
            vector_lanes(&mut out, d, 4, "u32", &body);
        }
        Opcode::Vxor | Opcode::Vxor128 => {
            let body = format!("{} ^ {}", at(a, "u32", "lane"), at(b, "u32", "lane"));
            vector_lanes(&mut out, d, 4, "u32", &body);
        }
        // Every set bit of the control takes the second source and every clear
        // bit takes the first, which is a choice per bit rather than per lane.
        Opcode::Vsel | Opcode::Vsel128 => {
            // The third source of a console form is the register it writes,
            // which the operands already report, so both spellings read it the
            // same way.
            let control = c;
            let body = format!(
                "({} & {}) | ({} & ~{})",
                at(b, "u32", "lane"),
                at(control, "u32", "lane"),
                at(a, "u32", "lane"),
                at(control, "u32", "lane")
            );
            vector_lanes(&mut out, d, 4, "u32", &body);
        }

        _ => return None,
    }

    Some(out)
}

/// Returns the C for a vector instruction that moves lanes rather than
/// computing across them.
fn vector_arrangement(instruction: Instruction) -> Option<String> {
    let (d, a, b, _) = vector_operands(instruction);
    let mut out = String::new();
    let immediate = u32::from(instruction.ra());

    let at = |register: u32, width: &str, lane: &str| {
        format!("xenolith_vector_{width}(&ctx->v[{register}], {lane})")
    };

    match instruction.opcode() {
        // A five bit signed immediate reaches every lane, sign extended to
        // whatever width the lane is.
        Opcode::Vspltisb | Opcode::Vspltish | Opcode::Vspltisw | Opcode::Vspltisw128 => {
            let width = match instruction.opcode() {
                Opcode::Vspltisb => 1,
                Opcode::Vspltish => 2,
                _ => 4,
            };
            let (name, count) = lane_width(width);
            let value = sign_extended(immediate, 5);
            let body = format!("(uint{}_t)(int{}_t)({value})", width * 8, width * 8);
            vector_lanes(&mut out, d, count, name, &body);
        }
        // One lane of a source reaches every lane.
        Opcode::Vspltb | Opcode::Vsplth | Opcode::Vspltw | Opcode::Vspltw128 => {
            let width = match instruction.opcode() {
                Opcode::Vspltb => 1,
                Opcode::Vsplth => 2,
                _ => 4,
            };
            let (name, count) = lane_width(width);
            let lane = immediate % count;
            let body = at(b, name, &lane.to_string());
            vector_lanes(&mut out, d, count, name, &body);
        }

        // A merge interleaves one half of each source, which is entirely about
        // which lane goes where.
        Opcode::Vmrghb
        | Opcode::Vmrghh
        | Opcode::Vmrghw
        | Opcode::Vmrghw128
        | Opcode::Vmrglb
        | Opcode::Vmrglh
        | Opcode::Vmrglw
        | Opcode::Vmrglw128 => {
            let width = match instruction.opcode() {
                Opcode::Vmrghb | Opcode::Vmrglb => 1,
                Opcode::Vmrghh | Opcode::Vmrglh => 2,
                _ => 4,
            };
            let (name, count) = lane_width(width);
            let low = matches!(
                instruction.opcode(),
                Opcode::Vmrglb | Opcode::Vmrglh | Opcode::Vmrglw | Opcode::Vmrglw128
            );
            let offset = if low { count / 2 } else { 0 };
            let source = format!("lane / 2 + {offset}");
            let body = format!(
                "(lane % 2 == 0) ? {} : {}",
                at(a, name, &source),
                at(b, name, &source)
            );
            vector_lanes(&mut out, d, count, name, &body);
        }

        _ => return None,
    }

    Some(out)
}

/// Returns the C for a vector instruction that converts, rounds, estimates, or
/// compares in single precision.
fn vector_convert(instruction: Instruction) -> Option<String> {
    let (d, a, b, _) = vector_operands(instruction);
    let mut out = String::new();
    let scale = u32::from(instruction.ra());

    let at = |register: u32, width: &str, lane: &str| {
        format!("xenolith_vector_{width}(&ctx->v[{register}], {lane})")
    };

    match instruction.opcode() {
        // Fixed point to single precision, divided by a power of two the
        // encoding names.
        Opcode::Vcfsx | Opcode::Vcfux | Opcode::Vcsxwfp128 | Opcode::Vcuxwfp128 => {
            let signed = matches!(instruction.opcode(), Opcode::Vcfsx | Opcode::Vcsxwfp128);
            let value = if signed {
                format!("(float)(int32_t){}", at(b, "u32", "lane"))
            } else {
                format!("(float){}", at(b, "u32", "lane"))
            };
            let divisor = two_to_the(scale);
            let body = format!("({value}) / {divisor}f");
            vector_lanes(&mut out, d, 4, "f32", &body);
        }
        // Single precision to fixed point, multiplied by the same and clamped
        // rather than wrapped.
        Opcode::Vctsxs | Opcode::Vctuxs | Opcode::Vcfpsxws128 | Opcode::Vcfpuxws128 => {
            let signed = matches!(instruction.opcode(), Opcode::Vctsxs | Opcode::Vcfpsxws128);
            let clamp = if signed {
                "xenolith_saturate_signed"
            } else {
                "xenolith_saturate_unsigned"
            };
            let multiplier = two_to_the(scale);
            let body = format!(
                "(uint32_t){clamp}({} * {multiplier}f)",
                at(b, "f32", "lane")
            );
            vector_lanes(&mut out, d, 4, "u32", &body);
        }
        // The roundings, each named for the direction it takes.
        Opcode::Vrfin
        | Opcode::Vrfin128
        | Opcode::Vrfiz
        | Opcode::Vrfiz128
        | Opcode::Vrfip
        | Opcode::Vrfip128
        | Opcode::Vrfim
        | Opcode::Vrfim128 => {
            let toward = match instruction.opcode() {
                Opcode::Vrfin | Opcode::Vrfin128 => "__builtin_nearbyintf",
                Opcode::Vrfiz | Opcode::Vrfiz128 => "__builtin_truncf",
                Opcode::Vrfip | Opcode::Vrfip128 => "__builtin_ceilf",
                _ => "__builtin_floorf",
            };
            let body = format!("{toward}({})", at(b, "f32", "lane"));
            vector_lanes(&mut out, d, 4, "f32", &body);
        }
        // The estimates. Hardware gives a few significant bits and this gives
        // all of them, which is a difference two implementations of an estimate
        // are allowed to have and the differential cannot compare exactly.
        Opcode::Vrefp
        | Opcode::Vrefp128
        | Opcode::Vrsqrtefp
        | Opcode::Vrsqrtefp128
        | Opcode::Vexptefp
        | Opcode::Vexptefp128
        | Opcode::Vlogefp
        | Opcode::Vlogefp128 => {
            let source = at(b, "f32", "lane");
            let body = match instruction.opcode() {
                Opcode::Vrefp | Opcode::Vrefp128 => format!("1.0f / ({source})"),
                Opcode::Vrsqrtefp | Opcode::Vrsqrtefp128 => {
                    format!("1.0f / __builtin_sqrtf({source})")
                }
                Opcode::Vexptefp | Opcode::Vexptefp128 => format!("__builtin_exp2f({source})"),
                _ => format!("__builtin_log2f({source})"),
            };
            vector_lanes(&mut out, d, 4, "f32", &body);
        }
        // A dot product the console added, whose result reaches every lane.
        Opcode::Vmsum3fp128 | Opcode::Vmsum4fp128 => {
            let count = if instruction.opcode() == Opcode::Vmsum3fp128 {
                3
            } else {
                4
            };
            let terms: Vec<String> = (0..count)
                .map(|lane| {
                    let lane = lane.to_string();
                    format!("{} * {}", at(a, "f32", &lane), at(b, "f32", &lane))
                })
                .collect();
            let _ = writeln!(out, "    {{ float dot = {};", terms.join(" + "));
            let _ = writeln!(out, "    xenolith_vector t;");
            let _ = writeln!(out, "    for (unsigned lane = 0; lane < 4; lane++) {{");
            let _ = writeln!(out, "        xenolith_vector_set_f32(&t, lane, dot);");
            let _ = writeln!(out, "    }}");
            let _ = writeln!(out, "    ctx->v[{d}] = t; }}");
        }

        _ => return None,
    }

    Some(out)
}

/// How an integer lane operation combines its operands.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Integer {
    /// Wraps rather than stopping at the end of the range.
    Add,
    Subtract,
    /// Stops at the end of the range rather than wrapping.
    AddSaturating,
    SubtractSaturating,
    ShiftLeft,
    ShiftRight,
    Rotate,
    Maximum,
    Minimum,
    /// Adds and halves, rounding away from zero.
    Average,
}

/// Returns the lane width, whether the lanes are signed, and what an integer
/// operation does to them.
#[allow(clippy::too_many_lines)]
fn integer_operation(opcode: Opcode) -> Option<(u32, bool, Integer)> {
    use Integer::{
        Add, AddSaturating, Average, Maximum, Minimum, Rotate, ShiftLeft, ShiftRight, Subtract,
        SubtractSaturating,
    };

    Some(match opcode {
        Opcode::Vaddubm => (1, false, Add),
        Opcode::Vadduhm => (2, false, Add),
        Opcode::Vadduwm => (4, false, Add),
        Opcode::Vsububm => (1, false, Subtract),
        Opcode::Vsubuhm => (2, false, Subtract),
        Opcode::Vsubuwm => (4, false, Subtract),

        Opcode::Vaddubs => (1, false, AddSaturating),
        Opcode::Vadduhs => (2, false, AddSaturating),
        Opcode::Vadduws => (4, false, AddSaturating),
        Opcode::Vaddsbs => (1, true, AddSaturating),
        Opcode::Vaddshs => (2, true, AddSaturating),
        Opcode::Vaddsws => (4, true, AddSaturating),
        Opcode::Vsububs => (1, false, SubtractSaturating),
        Opcode::Vsubuhs => (2, false, SubtractSaturating),
        Opcode::Vsubuws => (4, false, SubtractSaturating),
        Opcode::Vsubsbs => (1, true, SubtractSaturating),
        Opcode::Vsubshs => (2, true, SubtractSaturating),
        Opcode::Vsubsws => (4, true, SubtractSaturating),

        Opcode::Vslb => (1, false, ShiftLeft),
        Opcode::Vslh => (2, false, ShiftLeft),
        Opcode::Vslw | Opcode::Vslw128 => (4, false, ShiftLeft),
        Opcode::Vsrb => (1, false, ShiftRight),
        Opcode::Vsrh => (2, false, ShiftRight),
        Opcode::Vsrw | Opcode::Vsrw128 => (4, false, ShiftRight),
        Opcode::Vsrab => (1, true, ShiftRight),
        Opcode::Vsrah => (2, true, ShiftRight),
        Opcode::Vsraw | Opcode::Vsraw128 => (4, true, ShiftRight),
        Opcode::Vrlb => (1, false, Rotate),
        Opcode::Vrlh => (2, false, Rotate),
        Opcode::Vrlw => (4, false, Rotate),

        Opcode::Vmaxub => (1, false, Maximum),
        Opcode::Vmaxuh => (2, false, Maximum),
        Opcode::Vmaxuw => (4, false, Maximum),
        Opcode::Vmaxsb => (1, true, Maximum),
        Opcode::Vmaxsh => (2, true, Maximum),
        Opcode::Vmaxsw => (4, true, Maximum),
        Opcode::Vminub => (1, false, Minimum),
        Opcode::Vminuh => (2, false, Minimum),
        Opcode::Vminuw => (4, false, Minimum),
        Opcode::Vminsb => (1, true, Minimum),
        Opcode::Vminsh => (2, true, Minimum),
        Opcode::Vminsw => (4, true, Minimum),

        Opcode::Vavgub => (1, false, Average),
        Opcode::Vavguh => (2, false, Average),
        Opcode::Vavguw => (4, false, Average),
        Opcode::Vavgsb => (1, true, Average),
        Opcode::Vavgsh => (2, true, Average),
        Opcode::Vavgsw => (4, true, Average),

        _ => return None,
    })
}

/// Returns the source width, whether the source is read as signed, and whether
/// the narrower result is clamped and to which signedness.
fn pack_operation(opcode: Opcode) -> Option<(u32, bool, Option<bool>)> {
    Some(match opcode {
        Opcode::Vpkuhum | Opcode::Vpkuhum128 => (2, false, None),
        Opcode::Vpkuwum | Opcode::Vpkuwum128 => (4, false, None),
        Opcode::Vpkuhus | Opcode::Vpkuhus128 => (2, false, Some(false)),
        Opcode::Vpkuwus | Opcode::Vpkuwus128 => (4, false, Some(false)),
        Opcode::Vpkshss | Opcode::Vpkshss128 => (2, true, Some(true)),
        Opcode::Vpkshus | Opcode::Vpkshus128 => (2, true, Some(false)),
        Opcode::Vpkswss | Opcode::Vpkswss128 => (4, true, Some(true)),
        Opcode::Vpkswus | Opcode::Vpkswus128 => (4, true, Some(false)),
        _ => return None,
    })
}

/// Returns the source width and which half of it an unpack widens.
fn unpack_operation(opcode: Opcode) -> Option<(u32, bool)> {
    Some(match opcode {
        Opcode::Vupkhsb | Opcode::Vupkhsb128 => (1, false),
        Opcode::Vupklsb | Opcode::Vupklsb128 => (1, true),
        Opcode::Vupkhsh => (2, false),
        Opcode::Vupklsh => (2, true),
        _ => return None,
    })
}

/// Returns the C for unpacking a packed vertex format, if the format is one
/// this project has settled rather than taken from elsewhere.
fn vector_vertex_unpack(instruction: Instruction) -> Option<String> {
    let (d, _, b, _) = vector_operands(instruction);
    let mut out = String::new();

    // Unpacking a packed vertex format. The type field selects which, and only
    // one of them is settled well enough to write down.
    //
    // Type zero takes the last word as four bytes and puts each into the low
    // mantissa bits of one, giving a float between one and one plus a part in
    // eight million. The game's own constant table proves it: what it multiplies
    // the result by is 32896.5039, and two to the twenty third over two hundred
    // and fifty five is 32896.502, the same number to the precision a float
    // holds. Nothing else about that instruction would produce that constant.
    //
    // The byte order falls out of the same reading. A colour is stored with
    // alpha first, and the lanes come out red, green, blue, alpha, which is the
    // ordering a wrong reading would not have produced.
    if instruction.opcode() == Opcode::Vupkd3d128 {
        if (instruction.word() >> 16) & 0x1f != 0 {
            return None;
        }
        let _ = writeln!(out, "    {{ xenolith_vector t;");
        for (lane, byte) in [13u32, 14, 15, 12].into_iter().enumerate() {
            let _ = writeln!(
                out,
                "    xenolith_vector_set_u32(&t, {lane}, xenolith_vector_u8(&ctx->v[{b}], {byte}) | 0x3f800000u);"
            );
        }
        let _ = writeln!(out, "    ctx->v[{d}] = t; }}");
        return Some(out);
    }

    None
}

/// Returns the C for a console form that rearranges words by an immediate.
///
/// Where those immediates sit was derived rather than read. Each form's mask
/// leaves exactly one arrangement of the free bits that fits the fields the
/// instruction needs, and the values so assembled match what an independently
/// produced corpus reports for every occurrence in a whole title.
fn vector_console_arrangement(instruction: Instruction) -> Option<String> {
    let (d, _, b, _) = vector_operands(instruction);
    let mut out = String::new();

    // Four words chosen by two bits each, out of an immediate the form splits
    // across the word. Which end holds which half is settled by the mask
    // leaving exactly one other place for it, and by the immediates so
    // assembled matching what an independent corpus reports for all hundred
    // occurrences in one title.
    if instruction.opcode() == Opcode::Vpermwi128 {
        let immediate =
            (((instruction.word() >> 6) & 7) << 5) | ((instruction.word() >> 16) & 0x1f);
        let _ = writeln!(out, "    {{ xenolith_vector t;");
        for lane in 0..4u32 {
            let from = (immediate >> (6 - 2 * lane)) & 3;
            let _ = writeln!(
                out,
                "    xenolith_vector_set_u32(&t, {lane}, xenolith_vector_u32(&ctx->v[{b}], {from}));"
            );
        }
        let _ = writeln!(out, "    ctx->v[{d}] = t; }}");
        return Some(out);
    }

    // Rotating one operand by whole words and inserting the lanes a mask
    // selects, leaving the rest of the destination alone.
    if instruction.opcode() == Opcode::Vrlimi128 {
        let mask = (instruction.word() >> 16) & 0xf;
        let rotate = (instruction.word() >> 6) & 3;
        let _ = writeln!(out, "    {{ xenolith_vector t = ctx->v[{d}];");
        for lane in 0..4u32 {
            if mask & (1 << (3 - lane)) == 0 {
                continue;
            }
            let from = (lane + rotate) % 4;
            let _ = writeln!(
                out,
                "    xenolith_vector_set_u32(&t, {lane}, xenolith_vector_u32(&ctx->v[{b}], {from}));"
            );
        }
        let _ = writeln!(out, "    ctx->v[{d}] = t; }}");
        return Some(out);
    }

    None
}

/// Returns the C for a vector instruction that narrows, widens, or slides.
fn vector_pack(instruction: Instruction) -> Option<String> {
    let (d, a, b, _) = vector_operands(instruction);
    let mut out = String::new();
    if let Some(code) = vector_console_arrangement(instruction) {
        return Some(code);
    }
    if let Some(code) = vector_vertex_unpack(instruction) {
        return Some(code);
    }

    if let Some((source, signed, saturate)) = pack_operation(instruction.opcode()) {
        let (from, _) = lane_width(source);
        let (into, count) = lane_width(source / 2);
        let bits = source * 4;
        let half = count / 2;
        let read = |register: u32, lane: &str| {
            let read = format!("xenolith_vector_{from}(&ctx->v[{register}], {lane})");
            if signed {
                format!("(int64_t)(int{}_t){read}", source * 8)
            } else {
                format!("(int64_t){read}")
            }
        };
        // The first half of the result comes from the first operand and the
        // second from the second, which is what packing two vectors into one
        // means.
        let value = format!(
            "(lane < {half} ? {} : {})",
            read(a, "lane"),
            read(b, &format!("lane - {half}"))
        );
        let body = match saturate {
            None => format!("(uint{bits}_t){value}"),
            Some(true) => {
                let high = (1i64 << (bits - 1)) - 1;
                format!(
                    "(uint{bits}_t)xenolith_clamp({value}, {}, {high})",
                    -high - 1
                )
            }
            Some(false) => {
                let high = (1u64 << bits) - 1;
                format!("(uint{bits}_t)xenolith_clamp_unsigned({value}, {high})")
            }
        };
        vector_lanes(&mut out, d, count, into, &body);
        return Some(out);
    }

    if let Some((source, low)) = unpack_operation(instruction.opcode()) {
        let (from, lanes) = lane_width(source);
        let (into, count) = lane_width(source * 2);
        let offset = if low { count } else { 0 };
        let _ = lanes;
        let body = format!(
            "(uint{0}_t)(int{0}_t)(int{1}_t)xenolith_vector_{from}(&ctx->v[{b}], lane + {offset})",
            source * 16,
            source * 8
        );
        vector_lanes(&mut out, d, count, into, &body);
        return Some(out);
    }

    // The whole register moves, so the amount is taken once rather than lane
    // by lane. It sits in the last lane of the second operand, in the low bits
    // for a shift by bits and above them for a shift by bytes.
    if matches!(
        instruction.opcode(),
        Opcode::Vsl | Opcode::Vsr | Opcode::Vslo | Opcode::Vslo128 | Opcode::Vsro
    ) {
        let left = matches!(
            instruction.opcode(),
            Opcode::Vsl | Opcode::Vslo | Opcode::Vslo128
        );
        let whole = matches!(
            instruction.opcode(),
            Opcode::Vslo | Opcode::Vslo128 | Opcode::Vsro
        );
        let amount = if whole {
            format!("((xenolith_vector_u8(&ctx->v[{b}], 15) >> 3) & 0xf) * 8")
        } else {
            format!("xenolith_vector_u8(&ctx->v[{b}], 15) & 7")
        };
        let direction = if left { "left" } else { "right" };
        let _ = writeln!(out, "    {{ xenolith_vector t;");
        let _ = writeln!(
            out,
            "    xenolith_vector_shift_{direction}(&t, &ctx->v[{a}], (unsigned)({amount}));"
        );
        let _ = writeln!(out, "    ctx->v[{d}] = t; }}");
        return Some(out);
    }

    // Sliding a window across the two operands laid end to end.
    if matches!(instruction.opcode(), Opcode::Vsldoi | Opcode::Vsldoi128) {
        let shift = (instruction.word() >> 6) & 0xf;
        let body = format!(
            "(lane + {shift} < 16) ? xenolith_vector_u8(&ctx->v[{a}], lane + {shift}) : xenolith_vector_u8(&ctx->v[{b}], lane + {shift} - 16)"
        );
        vector_lanes(&mut out, d, 16, "u8", &body);
        return Some(out);
    }

    None
}

/// Returns the C for a vector instruction that works on integer lanes.
fn vector_integer(instruction: Instruction) -> Option<String> {
    let (d, a, b, _) = vector_operands(instruction);
    let (width, signed, operation) = integer_operation(instruction.opcode())?;

    let (name, count) = lane_width(width);
    let bits = width * 8;
    let read = |register: u32| {
        let lane = format!("xenolith_vector_{name}(&ctx->v[{register}], lane)");
        if signed {
            format!("(int64_t)(int{bits}_t){lane}")
        } else {
            format!("(int64_t){lane}")
        }
    };
    let (left, right) = (read(a), read(b));

    // The shifts and rotates take their count from the matching lane of the
    // second operand rather than treating it as a value.
    let places = format!(
        "(unsigned)(xenolith_vector_{name}(&ctx->v[{b}], lane) & {})",
        bits - 1
    );

    let body = match operation {
        Integer::Add => format!("(uint{bits}_t)({left} + {right})"),
        Integer::Subtract => format!("(uint{bits}_t)({left} - {right})"),
        Integer::AddSaturating | Integer::SubtractSaturating => {
            let sign = if operation == Integer::AddSaturating {
                '+'
            } else {
                '-'
            };
            let sum = format!("{left} {sign} {right}");
            if signed {
                let high = (1i64 << (bits - 1)) - 1;
                let low = -(1i64 << (bits - 1));
                format!("(uint{bits}_t)xenolith_clamp({sum}, {low}, {high})")
            } else {
                let high = (1u64 << bits) - 1;
                format!("(uint{bits}_t)xenolith_clamp_unsigned({sum}, {high})")
            }
        }
        Integer::ShiftLeft => format!("(uint{bits}_t)({left} << {places})"),
        Integer::ShiftRight => format!("(uint{bits}_t)({left} >> {places})"),
        Integer::Rotate => format!(
            "(uint{bits}_t)(({left} << {places}) | ((uint{bits}_t){left} >> (({bits} - {places}) & {})))",
            bits - 1
        ),
        Integer::Maximum => format!("(uint{bits}_t)({left} > {right} ? {left} : {right})"),
        Integer::Minimum => format!("(uint{bits}_t)({left} < {right} ? {left} : {right})"),
        Integer::Average => format!("(uint{bits}_t)(({left} + {right} + 1) >> 1)"),
    };

    let mut out = String::new();
    vector_lanes(&mut out, d, count, name, &body);
    Some(out)
}

/// Returns the C for a vector instruction that reaches memory or permutes
/// bytes.
///
/// The console added loads that take whichever part of a sixteen byte block
/// falls on one side of an address, so that a pair of them fetches a vector
/// from anywhere. Each writes zeroes over the part it did not take, which is
/// what lets the two be combined with a bitwise or.
fn vector_access(instruction: Instruction) -> Option<String> {
    let (d, a, b, c) = vector_operands(instruction);
    let (ra, rb) = (u32::from(instruction.ra()), u32::from(instruction.rb()));
    let mut out = String::new();

    let effective = if ra == 0 {
        format!("(uint32_t)ctx->r[{rb}]")
    } else {
        format!("(uint32_t)ctx->r[{ra}] + (uint32_t)ctx->r[{rb}]")
    };

    match instruction.opcode() {
        // The control vectors a permute is driven by, which say how far a
        // vector has to move to become aligned.
        Opcode::Lvsl | Opcode::Lvsl128 | Opcode::Lvsr | Opcode::Lvsr128 => {
            let value = if matches!(instruction.opcode(), Opcode::Lvsl | Opcode::Lvsl128) {
                "sh + lane"
            } else {
                "16 - sh + lane"
            };
            let _ = writeln!(out, "    {{ uint32_t ea = {effective};");
            let _ = writeln!(out, "    unsigned sh = ea & 0xf; xenolith_vector t;");
            let _ = writeln!(out, "    for (unsigned lane = 0; lane < 16; lane++) {{");
            let _ = writeln!(
                out,
                "        xenolith_vector_set_u8(&t, lane, (uint8_t)({value}));"
            );
            let _ = writeln!(out, "    }}");
            let _ = writeln!(out, "    ctx->v[{d}] = t; }}");
        }
        // Whichever part of the block lies at or after the address, moved to
        // the front, with zeroes behind it.
        Opcode::Lvlx | Opcode::Lvlx128 | Opcode::Lvlxl | Opcode::Lvlxl128 => {
            unaligned_load(&mut out, d, &effective, "lane + sh < 16", "ea + lane");
        }
        // Whichever part lies before it, moved to the back, with zeroes ahead.
        Opcode::Lvrx | Opcode::Lvrx128 | Opcode::Lvrxl | Opcode::Lvrxl128 => {
            unaligned_load(&mut out, d, &effective, "lane + sh >= 16", "ea - 16 + lane");
        }
        Opcode::Stvlx | Opcode::Stvlx128 | Opcode::Stvlxl | Opcode::Stvlxl128 => {
            unaligned_store(&mut out, d, &effective, "lane + sh < 16", "ea + lane");
        }
        Opcode::Stvrx | Opcode::Stvrx128 | Opcode::Stvrxl | Opcode::Stvrxl128 => {
            unaligned_store(&mut out, d, &effective, "lane + sh >= 16", "ea - 16 + lane");
        }
        // One word, into or out of the lane the address selects. The rest of
        // the register is left alone, which the instruction leaves undefined
        // and is the least surprising of the things undefined allows.
        Opcode::Lvewx | Opcode::Lvewx128 => {
            let _ = writeln!(out, "    {{ uint32_t ea = ({effective}) & ~3u;");
            let _ = writeln!(
                out,
                "    xenolith_vector_set_u32(&ctx->v[{d}], (ea >> 2) & 3, xenolith_load32(base, ea)); }}"
            );
        }
        Opcode::Stvewx | Opcode::Stewx128 => {
            let _ = writeln!(out, "    {{ uint32_t ea = ({effective}) & ~3u;");
            let _ = writeln!(
                out,
                "    xenolith_store32(base, ea, xenolith_vector_u32(&ctx->v[{d}], (ea >> 2) & 3)); }}"
            );
        }
        // Every byte of the result is chosen by the matching byte of a third
        // register, which is the one family that reads its indices out of a
        // register rather than out of the encoding.
        Opcode::Vperm | Opcode::Vperm128 => {
            let c = if instruction.opcode() == Opcode::Vperm128 {
                // Three bits of its own, so the control is one of the first
                // eight registers rather than any of the hundred and twenty
                // eight.
                (instruction.word() >> 6) & 7
            } else {
                c
            };
            let _ = writeln!(out, "    {{ xenolith_vector t;");
            let _ = writeln!(out, "    for (unsigned lane = 0; lane < 16; lane++) {{");
            let _ = writeln!(
                out,
                "        unsigned pick = xenolith_vector_u8(&ctx->v[{c}], lane) & 0x1f;"
            );
            let _ = writeln!(
                out,
                "        xenolith_vector_set_u8(&t, lane, pick < 16 ? xenolith_vector_u8(&ctx->v[{a}], pick) : xenolith_vector_u8(&ctx->v[{b}], pick - 16));"
            );
            let _ = writeln!(out, "    }}");
            let _ = writeln!(out, "    ctx->v[{d}] = t; }}");
        }

        _ => return None,
    }

    Some(out)
}

/// Writes an unaligned load, which takes the bytes a condition selects and
/// zeroes the rest.
fn unaligned_load(out: &mut String, destination: u32, effective: &str, taken: &str, from: &str) {
    let _ = writeln!(out, "    {{ uint32_t ea = {effective};");
    let _ = writeln!(out, "    unsigned sh = ea & 0xf; xenolith_vector t;");
    let _ = writeln!(out, "    for (unsigned lane = 0; lane < 16; lane++) {{");
    let _ = writeln!(
        out,
        "        xenolith_vector_set_u8(&t, lane, ({taken}) ? xenolith_load8(base, {from}) : 0);"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    ctx->v[{destination}] = t; }}");
}

/// Writes an unaligned store, which writes only the bytes a condition selects.
fn unaligned_store(out: &mut String, source: u32, effective: &str, taken: &str, to: &str) {
    let _ = writeln!(out, "    {{ uint32_t ea = {effective};");
    let _ = writeln!(out, "    unsigned sh = ea & 0xf;");
    let _ = writeln!(out, "    for (unsigned lane = 0; lane < 16; lane++) {{");
    let _ = writeln!(out, "        if ({taken}) {{");
    let _ = writeln!(
        out,
        "            xenolith_store8(base, {to}, xenolith_vector_u8(&ctx->v[{source}], lane));"
    );
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    }}");
}

/// Returns the C for a vector comparison.
///
/// Each lane becomes all ones or all zeroes, and the recording form also writes
/// the seventh condition field with whether every lane matched and whether none
/// did. That field is what a branch after one reads, so leaving it out would
/// produce code that compiles and takes the wrong path.
fn vector_compare(instruction: Instruction) -> Option<String> {
    let (d, a, b, _) = vector_operands(instruction);
    let (width, floating, test) = comparison(instruction.opcode())?;
    let (name, count) = lane_width(width);
    let bits = width * 8;

    let read = |register: u32| {
        if floating {
            format!("xenolith_vector_f32(&ctx->v[{register}], lane)")
        } else if test == ">s" {
            format!("(int{bits}_t)xenolith_vector_{name}(&ctx->v[{register}], lane)")
        } else {
            format!("xenolith_vector_{name}(&ctx->v[{register}], lane)")
        }
    };
    let operator = match test {
        "==" => "==",
        ">=" => ">=",
        _ => ">",
    };

    let records = instruction.word() & records_at(instruction) != 0;
    let mut out = String::new();

    if records {
        let _ = writeln!(out, "    {{ xenolith_vector t; unsigned all = 1, none = 1;");
    } else {
        let _ = writeln!(out, "    {{ xenolith_vector t;");
    }
    let _ = writeln!(
        out,
        "    for (unsigned lane = 0; lane < {count}; lane++) {{"
    );
    let _ = writeln!(
        out,
        "        uint{bits}_t r = (({}) {operator} ({})) ? (uint{bits}_t)~(uint{bits}_t)0 : 0;",
        read(a),
        read(b)
    );
    if records {
        let _ = writeln!(out, "        if (r) {{ none = 0; }} else {{ all = 0; }}");
    }
    let _ = writeln!(out, "        xenolith_vector_set_{name}(&t, lane, r);");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    ctx->v[{d}] = t;");
    if records {
        let _ = writeln!(out, "    ctx->cr[6].lt = (uint8_t)all;");
        let _ = writeln!(out, "    ctx->cr[6].gt = 0;");
        let _ = writeln!(out, "    ctx->cr[6].eq = (uint8_t)none;");
        let _ = writeln!(out, "    ctx->cr[6].so = 0;");
    }
    let _ = writeln!(out, "    }}");

    Some(out)
}

/// Returns the bit that makes a comparison record into a condition field.
///
/// The standard forms spend the bit every other form spends on a record bit.
/// The console's forms cannot: their identifying mask claims it. What that mask
/// leaves free is three bits, two of which the first source takes, and the one
/// left over is this. It is clear in every occurrence in either title, so what
/// is checked here is that a comparison which does not record leaves the field
/// alone.
fn records_at(instruction: Instruction) -> u32 {
    if instruction
        .form()
        .is_some_and(xenolith_ppc::Form::is_console_extension)
    {
        1 << 6
    } else {
        1 << 10
    }
}

/// Returns the lane width, whether the lanes are floats, and which test a
/// comparison applies.
fn comparison(opcode: Opcode) -> Option<(u32, bool, &'static str)> {
    Some(match opcode {
        Opcode::Vcmpeqfp | Opcode::Vcmpeqfp128 => (4, true, "=="),
        Opcode::Vcmpgtfp | Opcode::Vcmpgtfp128 => (4, true, ">"),
        Opcode::Vcmpgefp | Opcode::Vcmpgefp128 => (4, true, ">="),
        Opcode::Vcmpequw | Opcode::Vcmpequw128 => (4, false, "=="),
        Opcode::Vcmpequb => (1, false, "=="),
        Opcode::Vcmpequh => (2, false, "=="),
        Opcode::Vcmpgtub => (1, false, ">"),
        Opcode::Vcmpgtuh => (2, false, ">"),
        Opcode::Vcmpgtuw => (4, false, ">"),
        Opcode::Vcmpgtsb => (1, false, ">s"),
        Opcode::Vcmpgtsh => (2, false, ">s"),
        Opcode::Vcmpgtsw => (4, false, ">s"),
        _ => return None,
    })
}

/// Returns `2` raised to a power, written so a compiler folds it.
fn two_to_the(power: u32) -> String {
    let mut value = 1.0f64;
    for _ in 0..power.min(31) {
        value *= 2.0;
    }
    format!("{value:.1}")
}

/// Returns the C for a vector instruction that computes in single precision.
fn vector_float(instruction: Instruction) -> Option<String> {
    let (d, a, b, c) = vector_operands(instruction);
    let mut out = String::new();

    let at = |register: u32, width: &str, lane: &str| {
        format!("xenolith_vector_{width}(&ctx->v[{register}], {lane})")
    };

    match instruction.opcode() {
        // Four lanes of single precision, done as the host's floats.
        Opcode::Vaddfp | Opcode::Vaddfp128 => {
            let body = format!("{} + {}", at(a, "f32", "lane"), at(b, "f32", "lane"));
            vector_lanes(&mut out, d, 4, "f32", &body);
        }
        Opcode::Vsubfp | Opcode::Vsubfp128 => {
            let body = format!("{} - {}", at(a, "f32", "lane"), at(b, "f32", "lane"));
            vector_lanes(&mut out, d, 4, "f32", &body);
        }
        Opcode::Vmulfp128 => {
            let body = format!("{} * {}", at(a, "f32", "lane"), at(b, "f32", "lane"));
            vector_lanes(&mut out, d, 4, "f32", &body);
        }
        Opcode::Vmaxfp | Opcode::Vmaxfp128 | Opcode::Vminfp | Opcode::Vminfp128 => {
            let keep = if matches!(instruction.opcode(), Opcode::Vmaxfp | Opcode::Vmaxfp128) {
                ">"
            } else {
                "<"
            };
            let body = format!(
                "({0} {keep} {1}) ? {0} : {1}",
                at(a, "f32", "lane"),
                at(b, "f32", "lane")
            );
            vector_lanes(&mut out, d, 4, "f32", &body);
        }
        // The standard form multiplies its first and third operands and adds
        // the second. The console's has one register field fewer, so it
        // multiplies its two sources and adds the register it writes.
        // The console carries a third fused form whose multiply takes the
        // register it writes rather than adding it.
        Opcode::Vmaddcfp128 => {
            let body = format!(
                "__builtin_fmaf({}, {}, {})",
                at(a, "f32", "lane"),
                at(d, "f32", "lane"),
                at(b, "f32", "lane")
            );
            vector_lanes(&mut out, d, 4, "f32", &body);
        }
        Opcode::Vmaddfp | Opcode::Vmaddfp128 | Opcode::Vnmsubfp | Opcode::Vnmsubfp128 => {
            let console = matches!(
                instruction.opcode(),
                Opcode::Vmaddfp128 | Opcode::Vnmsubfp128
            );
            let (left, right, addend) = if console { (a, b, c) } else { (a, c, b) };
            let (left, right, addend) = (
                at(left, "f32", "lane"),
                at(right, "f32", "lane"),
                at(addend, "f32", "lane"),
            );
            // The multiply and the add round once between them rather than
            // twice, so writing them as two operations gives an answer one
            // place out for some inputs. Hardware said so, four runs in.
            let body = if matches!(instruction.opcode(), Opcode::Vmaddfp | Opcode::Vmaddfp128) {
                format!("__builtin_fmaf({left}, {right}, {addend})")
            } else {
                format!("-__builtin_fmaf({left}, {right}, -({addend}))")
            };
            vector_lanes(&mut out, d, 4, "f32", &body);
        }

        _ => return None,
    }

    Some(out)
}

/// Returns an immediate sign extended from `bits` wide.
fn sign_extended(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    #[allow(clippy::cast_possible_wrap)]
    let wide = value as i32;
    (wide << shift) >> shift
}

/// Returns the C for one instruction, if it can be written.
///
/// Returns `None` for anything this crate cannot express, which is a normal
/// outcome and stops the whole function rather than producing a hole.
#[allow(clippy::too_many_lines)]
fn code_of(instruction: Instruction, address: u32) -> Option<String> {
    let (rt, ra, rb) = (
        u32::from(instruction.rt()),
        u32::from(instruction.ra()),
        u32::from(instruction.rb()),
    );
    let displacement = instruction.displacement();
    let immediate = u32::from(instruction.immediate());
    let mut out = String::new();

    // The record bit compares the result against zero afterwards, so the value
    // is computed first and the comparison appended.
    //
    // A conditional store carries a set low bit as part of its spelling rather
    // than as a record bit, and it writes the condition field itself from
    // whether the store happened. Letting the comparison run as well would
    // overwrite that with a test of the value stored, which is how this was
    // found: the retry after one never stopped retrying.
    let records = (instruction
        .form()
        .is_some_and(xenolith_ppc::Form::has_record_bit)
        && instruction.record_bit()
        || matches!(instruction.opcode(), Opcode::Andi | Opcode::Andis))
        && !matches!(instruction.opcode(), Opcode::Stwcx | Opcode::Stdcx);

    let recorded = match instruction.opcode() {
        Opcode::And
        | Opcode::Andc
        | Opcode::Nand
        | Opcode::Nor
        | Opcode::Or
        | Opcode::Orc
        | Opcode::Xor
        | Opcode::Eqv
        | Opcode::Slw
        | Opcode::Srw
        | Opcode::Sraw
        | Opcode::Srawi
        | Opcode::Rlwinm
        | Opcode::Rlwimi
        | Opcode::Rlwnm
        | Opcode::Extsb
        | Opcode::Extsh
        | Opcode::Extsw
        | Opcode::Cntlzw
        | Opcode::Andi
        | Opcode::Andis => ra,
        _ => rt,
    };

    match instruction.opcode() {
        // Nothing at all, however it is spelled.
        Opcode::Ori if rt == 0 && ra == 0 && immediate == 0 => {}
        Opcode::Sync | Opcode::Isync | Opcode::Eieio => {}

        Opcode::Addi if ra == 0 => {
            let _ = writeln!(
                out,
                "    ctx->r[{rt}] = (uint64_t)(int64_t)({displacement});"
            );
        }
        Opcode::Addis if ra == 0 => {
            let value = i64::from(displacement) << 16;
            let _ = writeln!(out, "    ctx->r[{rt}] = (uint64_t)(int64_t)({value});");
        }
        Opcode::Addi => {
            let _ = writeln!(
                out,
                "    ctx->r[{rt}] = ctx->r[{ra}] + (uint64_t)(int64_t)({displacement});"
            );
        }
        Opcode::Addic | Opcode::AddicRc => {
            let _ = writeln!(out, "    {{ uint64_t left = ctx->r[{ra}];");
            let _ = writeln!(
                out,
                "    uint64_t sum = left + (uint64_t)(int64_t)({displacement});"
            );
            let _ = writeln!(
                out,
                "    ctx->xer = (ctx->xer & ~0x20000000ull) | (sum < left ? 0x20000000ull : 0ull);"
            );
            let _ = writeln!(out, "    ctx->r[{rt}] = sum; }}");
        }
        Opcode::Addis => {
            let value = i64::from(displacement) << 16;
            let _ = writeln!(
                out,
                "    ctx->r[{rt}] = ctx->r[{ra}] + (uint64_t)(int64_t)({value});"
            );
        }
        Opcode::Add => {
            let _ = writeln!(out, "    ctx->r[{rt}] = ctx->r[{ra}] + ctx->r[{rb}];");
        }
        Opcode::Subf => {
            let _ = writeln!(out, "    ctx->r[{rt}] = ctx->r[{rb}] - ctx->r[{ra}];");
        }
        Opcode::Subfic => {
            let _ = writeln!(
                out,
                "    {{ uint64_t from = (uint64_t)(int64_t)({displacement});"
            );
            let _ = writeln!(
                out,
                "    ctx->xer = (ctx->xer & ~0x20000000ull) | (from >= ctx->r[{ra}] ? 0x20000000ull : 0ull);"
            );
            let _ = writeln!(out, "    ctx->r[{rt}] = from - ctx->r[{ra}]; }}");
        }
        Opcode::Neg => {
            let _ = writeln!(
                out,
                "    ctx->r[{rt}] = (uint64_t)(-(int64_t)ctx->r[{ra}]);"
            );
        }
        Opcode::Mulli => {
            let _ = writeln!(
                out,
                "    ctx->r[{rt}] = (uint64_t)((int64_t)ctx->r[{ra}] * (int64_t)({displacement}));"
            );
        }
        Opcode::Mullw => {
            let _ = writeln!(
                out,
                "    ctx->r[{rt}] = (uint64_t)(int64_t)((int64_t)(int32_t)ctx->r[{ra}] * (int64_t)(int32_t)ctx->r[{rb}]);"
            );
        }
        Opcode::Mulld => {
            let _ = writeln!(
                out,
                "    ctx->r[{rt}] = (uint64_t)((int64_t)ctx->r[{ra}] * (int64_t)ctx->r[{rb}]);"
            );
        }
        Opcode::Divw => {
            let _ = writeln!(
                out,
                "    ctx->r[{rt}] = (uint64_t)(uint32_t)((int32_t)ctx->r[{rb}] == 0 ? 0 : (int32_t)ctx->r[{ra}] / (int32_t)ctx->r[{rb}]);"
            );
        }
        Opcode::Divwu => {
            let _ = writeln!(
                out,
                "    ctx->r[{rt}] = (uint64_t)(uint32_t)((uint32_t)ctx->r[{rb}] == 0 ? 0 : (uint32_t)ctx->r[{ra}] / (uint32_t)ctx->r[{rb}]);"
            );
        }

        Opcode::And => {
            let _ = writeln!(out, "    ctx->r[{ra}] = ctx->r[{rt}] & ctx->r[{rb}];");
        }
        Opcode::Or => {
            let _ = writeln!(out, "    ctx->r[{ra}] = ctx->r[{rt}] | ctx->r[{rb}];");
        }
        Opcode::Xor => {
            let _ = writeln!(out, "    ctx->r[{ra}] = ctx->r[{rt}] ^ ctx->r[{rb}];");
        }
        Opcode::Andc => {
            let _ = writeln!(out, "    ctx->r[{ra}] = ctx->r[{rt}] & ~ctx->r[{rb}];");
        }
        Opcode::Orc => {
            let _ = writeln!(out, "    ctx->r[{ra}] = ctx->r[{rt}] | ~ctx->r[{rb}];");
        }
        Opcode::Nand => {
            let _ = writeln!(out, "    ctx->r[{ra}] = ~(ctx->r[{rt}] & ctx->r[{rb}]);");
        }
        Opcode::Nor => {
            let _ = writeln!(out, "    ctx->r[{ra}] = ~(ctx->r[{rt}] | ctx->r[{rb}]);");
        }
        Opcode::Eqv => {
            let _ = writeln!(out, "    ctx->r[{ra}] = ~(ctx->r[{rt}] ^ ctx->r[{rb}]);");
        }
        Opcode::Andi | Opcode::Ori | Opcode::Xori => {
            let operator = match instruction.opcode() {
                Opcode::Andi => "&",
                Opcode::Ori => "|",
                _ => "^",
            };
            let _ = writeln!(
                out,
                "    ctx->r[{ra}] = ctx->r[{rt}] {operator} {immediate}u;"
            );
        }
        Opcode::Andis | Opcode::Oris | Opcode::Xoris => {
            let operator = match instruction.opcode() {
                Opcode::Andis => "&",
                Opcode::Oris => "|",
                _ => "^",
            };
            let value = u64::from(immediate) << 16;
            let _ = writeln!(
                out,
                "    ctx->r[{ra}] = ctx->r[{rt}] {operator} {value}ull;"
            );
        }
        Opcode::Extsb => {
            let _ = writeln!(
                out,
                "    ctx->r[{ra}] = (uint64_t)(int64_t)(int8_t)ctx->r[{rt}];"
            );
        }
        Opcode::Extsh => {
            let _ = writeln!(
                out,
                "    ctx->r[{ra}] = (uint64_t)(int64_t)(int16_t)ctx->r[{rt}];"
            );
        }
        Opcode::Extsw => {
            let _ = writeln!(
                out,
                "    ctx->r[{ra}] = (uint64_t)(int64_t)(int32_t)ctx->r[{rt}];"
            );
        }
        Opcode::Slw => {
            let _ = writeln!(
                out,
                "    ctx->r[{ra}] = (ctx->r[{rb}] & 0x3f) < 32 ? (uint32_t)(ctx->r[{rt}] << (ctx->r[{rb}] & 0x3f)) : 0;"
            );
        }
        Opcode::Srw => {
            let _ = writeln!(
                out,
                "    ctx->r[{ra}] = (ctx->r[{rb}] & 0x3f) < 32 ? ((uint32_t)ctx->r[{rt}] >> (ctx->r[{rb}] & 0x3f)) : 0;"
            );
        }
        Opcode::Rlwnm => {
            let mask = rotate_mask(instruction.mask_begin(), instruction.mask_end());
            writeln!(out, "    {{ uint32_t value = (uint32_t)ctx->r[{rt}];").ok()?;
            writeln!(out, "    uint32_t places = (uint32_t)ctx->r[{rb}] & 31u;").ok()?;
            writeln!(
                out,
                "    uint32_t rotated = places == 0u ? value : ((value << places) | (value >> (32u - places)));"
            )
            .ok()?;
            writeln!(out, "    ctx->r[{ra}] = rotated & {mask}u; }}").ok()?;
        }
        Opcode::Rlwinm | Opcode::Rlwimi => {
            let places = u32::from(instruction.shift_amount());
            let mask = rotate_mask(instruction.mask_begin(), instruction.mask_end());
            let rotated = if places == 0 {
                format!("(uint32_t)ctx->r[{rt}]")
            } else {
                format!(
                    "(((uint32_t)ctx->r[{rt}] << {places}) | ((uint32_t)ctx->r[{rt}] >> {}))",
                    32 - places
                )
            };
            if instruction.opcode() == Opcode::Rlwimi {
                let _ = writeln!(
                    out,
                    "    ctx->r[{ra}] = ((uint32_t)ctx->r[{ra}] & ~{mask}u) | ({rotated} & {mask}u);"
                );
            } else {
                let _ = writeln!(out, "    ctx->r[{ra}] = {rotated} & {mask}u;");
            }
        }

        Opcode::Cmp | Opcode::Cmpi => {
            let field = rt >> 2;
            let long = rt & 1 == 1;
            let left = if long {
                format!("(int64_t)ctx->r[{ra}]")
            } else {
                format!("(int64_t)(int32_t)ctx->r[{ra}]")
            };
            let right = if instruction.opcode() == Opcode::Cmp {
                if long {
                    format!("(int64_t)ctx->r[{rb}]")
                } else {
                    format!("(int64_t)(int32_t)ctx->r[{rb}]")
                }
            } else {
                format!("(int64_t)({displacement})")
            };
            let less = format!("({left}) < ({right})");
            compare(
                &mut out,
                u8::try_from(field).unwrap_or(0),
                &left,
                &right,
                &less,
            );
        }
        Opcode::Cmpl | Opcode::Cmpli => {
            let field = rt >> 2;
            let long = rt & 1 == 1;
            let left = if long {
                format!("(uint64_t)ctx->r[{ra}]")
            } else {
                format!("(uint64_t)(uint32_t)ctx->r[{ra}]")
            };
            let right = if instruction.opcode() == Opcode::Cmpl {
                if long {
                    format!("(uint64_t)ctx->r[{rb}]")
                } else {
                    format!("(uint64_t)(uint32_t)ctx->r[{rb}]")
                }
            } else {
                format!("{immediate}ull")
            };
            let less = if instruction.opcode() == Opcode::Cmpli && immediate == 0 {
                "0".to_owned()
            } else {
                format!("({left}) < ({right})")
            };
            compare(
                &mut out,
                u8::try_from(field).unwrap_or(0),
                &left,
                &right,
                &less,
            );
        }

        Opcode::Mtspr => {
            let target = match instruction.spr() {
                1 => "xer",
                8 => "lr",
                9 => "ctr",
                _ => return None,
            };
            let _ = writeln!(out, "    ctx->{target} = ctx->r[{rt}];");
        }
        Opcode::Mfspr => {
            let source = match instruction.spr() {
                1 => "xer",
                8 => "lr",
                9 => "ctr",
                _ => return None,
            };
            let _ = writeln!(out, "    ctx->r[{rt}] = ctx->{source};");
        }

        // The time base advances, and what it advances at is the environment's
        // to decide. The upper form takes the high half of the same counter.
        Opcode::Mftb => {
            let read = match instruction.spr() {
                268 => "xenolith_timebase()".to_owned(),
                269 => "(xenolith_timebase() >> 32)".to_owned(),
                _ => return None,
            };
            let _ = writeln!(out, "    ctx->r[{rt}] = {read};");
        }

        // Storage, and nothing more. What these registers would have done is
        // set out in the runtime interface beside their declaration.
        Opcode::Mfmsr => {
            let _ = writeln!(out, "    ctx->r[{rt}] = ctx->msr;");
        }
        Opcode::Mtmsr | Opcode::Mtmsrd => {
            let _ = writeln!(out, "    ctx->msr = ctx->r[{rt}];");
        }
        Opcode::Mffs => {
            let _ = writeln!(out, "    ctx->f[{rt}].u64 = ctx->fpscr & 0xffffffffull;");
        }
        Opcode::Mtfsf => {
            let mask = nibble_mask(instruction.status_mask());
            let _ = writeln!(
                out,
                "    ctx->fpscr = (ctx->fpscr & ~{mask}ull) | ((uint64_t)(uint32_t)ctx->f[{rb}].u64 & {mask}ull);"
            );
        }
        Opcode::Mtfsfi => {
            let field = rt >> 2;
            let value = u64::from((rb >> 1) & 0xf) << (28 - 4 * u64::from(field));
            let mask = 0xfu64 << (28 - 4 * u64::from(field));
            let _ = writeln!(
                out,
                "    ctx->fpscr = (ctx->fpscr & ~{mask}ull) | {value}ull;"
            );
        }

        // A logical between two condition bits writes the single bit its
        // encoding names and leaves every other one alone.
        Opcode::Crand
        | Opcode::Crandc
        | Opcode::Cror
        | Opcode::Crorc
        | Opcode::Crxor
        | Opcode::Crnand
        | Opcode::Crnor
        | Opcode::Creqv => {
            let left = condition_bit(ra);
            let right = condition_bit(rb);
            let value = match instruction.opcode() {
                Opcode::Crand => format!("{left} & {right}"),
                Opcode::Crandc => format!("{left} & (uint8_t)!{right}"),
                Opcode::Cror => format!("{left} | {right}"),
                Opcode::Crorc => format!("{left} | (uint8_t)!{right}"),
                Opcode::Crxor => format!("{left} ^ {right}"),
                Opcode::Crnand => format!("(uint8_t)!({left} & {right})"),
                Opcode::Crnor => format!("(uint8_t)!({left} | {right})"),
                _ => format!("(uint8_t)!({left} ^ {right})"),
            };
            let _ = writeln!(out, "    {} = (uint8_t)({value}) & 1;", condition_bit(rt));
        }
        Opcode::Mcrf => {
            let (to, from) = (rt >> 2, ra >> 2);
            let _ = writeln!(out, "    ctx->cr[{to}] = ctx->cr[{from}];");
        }

        // The whole condition register moves as one word, in a bit order the
        // runtime interface states once for both directions.
        Opcode::Mfcr => {
            let _ = writeln!(out, "    ctx->r[{rt}] = xenolith_condition_pack(ctx->cr);");
        }
        Opcode::Mtcrf => {
            let mask = instruction.condition_mask();
            let _ = writeln!(
                out,
                "    xenolith_condition_unpack(ctx->cr, (uint32_t)ctx->r[{rt}], {mask}u);"
            );
        }

        opcode if load_width(opcode).is_some() => {
            let (width, signed, indexed, updating, floating) = load_width(opcode)?;
            let base = if ra == 0 && !updating {
                "0".to_owned()
            } else {
                format!("(uint32_t)ctx->r[{ra}]")
            };
            let offset = if indexed {
                format!("(uint32_t)ctx->r[{rb}]")
            } else {
                format!("(uint32_t)({displacement})")
            };
            let _ = writeln!(out, "    address = {base} + {offset};");

            if floating {
                let read = if width == 4 {
                    "    { uint32_t bits = xenolith_load32(base, address); float value; __builtin_memcpy(&value, &bits, 4); ctx->f[RT].f64 = (double)value; }\n"
                } else {
                    "    ctx->f[RT].u64 = xenolith_load64(base, address);\n"
                };
                out.push_str(&read.replace("RT", &rt.to_string()));
            } else if signed {
                let _ = writeln!(
                    out,
                    "    ctx->r[{rt}] = (uint64_t)(int64_t)(int{}_t)xenolith_load{}(base, address);",
                    width * 8,
                    width * 8
                );
            } else {
                let _ = writeln!(
                    out,
                    "    ctx->r[{rt}] = xenolith_load{}(base, address);",
                    width * 8
                );
            }
            if updating {
                let _ = writeln!(out, "    ctx->r[{ra}] = address;");
            }
        }

        opcode if store_width(opcode).is_some() => {
            let (width, indexed, updating, floating) = store_width(opcode)?;
            let base = if ra == 0 && !updating {
                "0".to_owned()
            } else {
                format!("(uint32_t)ctx->r[{ra}]")
            };
            let offset = if indexed {
                format!("(uint32_t)ctx->r[{rb}]")
            } else {
                format!("(uint32_t)({displacement})")
            };
            let _ = writeln!(out, "    address = {base} + {offset};");

            if floating {
                let write_out = if width == 4 {
                    "    { float value = (float)ctx->f[RT].f64; uint32_t bits; __builtin_memcpy(&bits, &value, 4); xenolith_store32(base, address, bits); }\n"
                } else {
                    "    xenolith_store64(base, address, ctx->f[RT].u64);\n"
                };
                out.push_str(&write_out.replace("RT", &rt.to_string()));
            } else {
                let _ = writeln!(
                    out,
                    "    xenolith_store{}(base, address, (uint{}_t)ctx->r[{rt}]);",
                    width * 8,
                    width * 8
                );
            }
            if updating {
                let _ = writeln!(out, "    ctx->r[{ra}] = address;");
            }
        }

        Opcode::Cntlzw => {
            writeln!(
                out,
                "    ctx->r[{ra}] = (uint32_t)ctx->r[{rt}] == 0 ? 32u : (uint32_t)__builtin_clz((uint32_t)ctx->r[{rt}]);"
            )
            .ok()?;
        }
        Opcode::Cntlzd => {
            writeln!(
                out,
                "    ctx->r[{ra}] = ctx->r[{rt}] == 0 ? 64u : (uint64_t)__builtin_clzll(ctx->r[{rt}]);"
            )
            .ok()?;
        }
        // An arithmetic right shift records whether it dropped a one bit out of
        // a negative value, which is what its carry means.
        Opcode::Srawi | Opcode::Sraw => {
            let places = if instruction.opcode() == Opcode::Srawi {
                format!("{}u", instruction.shift_amount())
            } else {
                format!("((uint32_t)ctx->r[{rb}] & 0x3fu)")
            };
            writeln!(out, "    {{ uint32_t value = (uint32_t)ctx->r[{rt}];").ok()?;
            writeln!(out, "    uint32_t places = {places};").ok()?;
            writeln!(out, "    uint32_t taken = places < 32u ? places : 32u;").ok()?;
            writeln!(
                out,
                "    uint32_t lost = taken == 0u ? 0u : (taken >= 32u ? value : (value & ((1u << taken) - 1u)));"
            )
            .ok()?;
            writeln!(
                out,
                "    ctx->r[{ra}] = (uint64_t)(int64_t)((int32_t)value >> (taken >= 32u ? 31u : taken));"
            )
            .ok()?;
            writeln!(
                out,
                "    ctx->xer = (ctx->xer & ~0x20000000ull) | (((int32_t)value < 0 && lost != 0u) ? 0x20000000ull : 0ull); }}"
            )
            .ok()?;
        }
        Opcode::Sld => {
            writeln!(
                out,
                "    ctx->r[{ra}] = (ctx->r[{rb}] & 0x7f) < 64 ? (ctx->r[{rt}] << (ctx->r[{rb}] & 0x7f)) : 0;"
            )
            .ok()?;
        }
        Opcode::Srd => {
            writeln!(
                out,
                "    ctx->r[{ra}] = (ctx->r[{rb}] & 0x7f) < 64 ? (ctx->r[{rt}] >> (ctx->r[{rb}] & 0x7f)) : 0;"
            )
            .ok()?;
        }
        Opcode::Srad | Opcode::Sradi => {
            let places = if instruction.opcode() == Opcode::Sradi {
                format!("{}ull", instruction.long_shift_amount())
            } else {
                format!("(ctx->r[{rb}] & 0x7full)")
            };
            writeln!(out, "    {{ uint64_t value = ctx->r[{rt}];").ok()?;
            writeln!(out, "    uint64_t places = {places};").ok()?;
            writeln!(out, "    uint64_t taken = places < 64ull ? places : 64ull;").ok()?;
            writeln!(
                out,
                "    uint64_t lost = taken == 0ull ? 0ull : (taken >= 64ull ? value : (value & ((1ull << taken) - 1ull)));"
            )
            .ok()?;
            writeln!(
                out,
                "    ctx->r[{ra}] = (uint64_t)((int64_t)value >> (taken >= 64ull ? 63ull : taken));"
            )
            .ok()?;
            writeln!(
                out,
                "    ctx->xer = (ctx->xer & ~0x20000000ull) | (((int64_t)value < 0 && lost != 0ull) ? 0x20000000ull : 0ull); }}"
            )
            .ok()?;
        }
        Opcode::Rldicl | Opcode::Rldicr | Opcode::Rldic | Opcode::Rldimi => {
            let places = u32::from(instruction.long_shift_amount());
            let bound = u32::from(instruction.long_mask_bound());
            // The left shifting form bounds the end of the mask and the right
            // shifting ones bound its beginning.
            let mask: u64 = if instruction.opcode() == Opcode::Rldicr {
                if bound >= 63 {
                    u64::MAX
                } else {
                    u64::MAX << (63 - bound)
                }
            } else if bound >= 64 {
                0
            } else {
                u64::MAX >> bound
            };
            let rotated = if places == 0 {
                format!("ctx->r[{rt}]")
            } else {
                format!(
                    "((ctx->r[{rt}] << {places}) | (ctx->r[{rt}] >> {}))",
                    64 - places
                )
            };
            if instruction.opcode() == Opcode::Rldimi {
                writeln!(
                    out,
                    "    ctx->r[{ra}] = (ctx->r[{ra}] & ~{mask}ull) | ({rotated} & {mask}ull);"
                )
                .ok()?;
            } else {
                writeln!(out, "    ctx->r[{ra}] = {rotated} & {mask}ull;").ok()?;
            }
        }

        // A trap that fires leaves the function, which emitted code cannot
        // express, so where it goes is the environment's to decide.
        Opcode::Tw | Opcode::Twi | Opcode::Td | Opcode::Tdi => {
            writeln!(out, "    xenolith_trap(ctx, base, 0x{address:08x}u);").ok()?;
        }

        // Storing a float as an integer word takes the low half of the register
        // as it stands rather than converting it, which is what makes it the
        // way an integer gets out of the floating bank.
        Opcode::Stfiwx => {
            let base = if ra == 0 {
                "0".to_owned()
            } else {
                format!("(uint32_t)ctx->r[{ra}]")
            };
            writeln!(out, "    address = {base} + (uint32_t)ctx->r[{rb}];").ok()?;
            writeln!(
                out,
                "    xenolith_store32(base, address, (uint32_t)ctx->f[{rt}].u64);"
            )
            .ok()?;
        }

        Opcode::Fmr => {
            writeln!(out, "    ctx->f[{rt}] = ctx->f[{rb}];").ok()?;
        }
        Opcode::Fneg => {
            writeln!(out, "    ctx->f[{rt}].f64 = -ctx->f[{rb}].f64;").ok()?;
        }
        Opcode::Fabs => {
            writeln!(
                out,
                "    ctx->f[{rt}].f64 = __builtin_fabs(ctx->f[{rb}].f64);"
            )
            .ok()?;
        }
        Opcode::Fnabs => {
            writeln!(
                out,
                "    ctx->f[{rt}].f64 = -__builtin_fabs(ctx->f[{rb}].f64);"
            )
            .ok()?;
        }
        Opcode::Frsp => {
            writeln!(
                out,
                "    ctx->f[{rt}].f64 = (double)(float)ctx->f[{rb}].f64;"
            )
            .ok()?;
        }
        Opcode::Fcfid => {
            writeln!(out, "    ctx->f[{rt}].f64 = (double)ctx->f[{rb}].s64;").ok()?;
        }
        Opcode::Fctid | Opcode::Fctidz => {
            writeln!(out, "    ctx->f[{rt}].s64 = (int64_t)ctx->f[{rb}].f64;").ok()?;
        }
        Opcode::Fctiw | Opcode::Fctiwz => {
            writeln!(
                out,
                "    ctx->f[{rt}].s64 = (int64_t)(int32_t)ctx->f[{rb}].f64;"
            )
            .ok()?;
        }
        Opcode::Fadd | Opcode::Fsub | Opcode::Fdiv => {
            let operator = match instruction.opcode() {
                Opcode::Fadd => "+",
                Opcode::Fsub => "-",
                _ => "/",
            };
            writeln!(
                out,
                "    ctx->f[{rt}].f64 = ctx->f[{ra}].f64 {operator} ctx->f[{rb}].f64;"
            )
            .ok()?;
        }
        Opcode::Fadds | Opcode::Fsubs | Opcode::Fdivs => {
            let operator = match instruction.opcode() {
                Opcode::Fadds => "+",
                Opcode::Fsubs => "-",
                _ => "/",
            };
            writeln!(
                out,
                "    ctx->f[{rt}].f64 = (double)((float)ctx->f[{ra}].f64 {operator} (float)ctx->f[{rb}].f64);"
            )
            .ok()?;
        }
        Opcode::Fmul => {
            let frc = (instruction.word() >> 6) & 0x1f;
            writeln!(
                out,
                "    ctx->f[{rt}].f64 = ctx->f[{ra}].f64 * ctx->f[{frc}].f64;"
            )
            .ok()?;
        }
        Opcode::Fmuls => {
            let frc = (instruction.word() >> 6) & 0x1f;
            writeln!(
                out,
                "    ctx->f[{rt}].f64 = (double)((float)ctx->f[{ra}].f64 * (float)ctx->f[{frc}].f64);"
            )
            .ok()?;
        }
        Opcode::Fcmpu | Opcode::Fcmpo => {
            let field = rt >> 2;
            writeln!(
                out,
                "    ctx->cr[{field}].lt = ctx->f[{ra}].f64 < ctx->f[{rb}].f64;\n                     ctx->cr[{field}].gt = ctx->f[{ra}].f64 > ctx->f[{rb}].f64;\n                     ctx->cr[{field}].eq = ctx->f[{ra}].f64 == ctx->f[{rb}].f64;\n                     ctx->cr[{field}].so = !(ctx->f[{ra}].f64 == ctx->f[{ra}].f64) || !(ctx->f[{rb}].f64 == ctx->f[{rb}].f64);"
            )
            .ok()?;
        }

        Opcode::Addc | Opcode::Adde | Opcode::Addze | Opcode::Addme => {
            let addend = match instruction.opcode() {
                Opcode::Addc => format!("ctx->r[{rb}]"),
                Opcode::Adde => format!("ctx->r[{rb}] + ((ctx->xer >> 29) & 1)"),
                Opcode::Addze => "((ctx->xer >> 29) & 1)".to_owned(),
                _ => "((ctx->xer >> 29) & 1) + 0xffffffffffffffffull".to_owned(),
            };
            writeln!(
                out,
                "    {{ uint64_t left = ctx->r[{ra}]; uint64_t sum = left + ({addend});"
            )
            .ok()?;
            writeln!(
                out,
                "    ctx->xer = (ctx->xer & ~0x20000000ull) | (sum < left ? 0x20000000ull : 0);"
            )
            .ok()?;
            writeln!(out, "    ctx->r[{rt}] = sum; }}").ok()?;
        }
        Opcode::Subfc | Opcode::Subfe | Opcode::Subfze | Opcode::Subfme => {
            let minuend = match instruction.opcode() {
                Opcode::Subfc | Opcode::Subfe => format!("ctx->r[{rb}]"),
                Opcode::Subfze => "0".to_owned(),
                _ => "0xffffffffffffffffull".to_owned(),
            };
            let carry = if matches!(instruction.opcode(), Opcode::Subfc) {
                "1".to_owned()
            } else {
                "((ctx->xer >> 29) & 1)".to_owned()
            };
            writeln!(
                out,
                "    {{ uint64_t sum = ~ctx->r[{ra}] + ({minuend}) + ({carry});"
            )
            .ok()?;
            writeln!(
                out,
                "    ctx->xer = (ctx->xer & ~0x20000000ull) | (sum < ({minuend}) ? 0x20000000ull : 0);"
            )
            .ok()?;
            writeln!(out, "    ctx->r[{rt}] = sum; }}").ok()?;
        }
        Opcode::Mulhw => {
            writeln!(
                out,
                "    ctx->r[{rt}] = (uint64_t)(uint32_t)(((int64_t)(int32_t)ctx->r[{ra}] * (int64_t)(int32_t)ctx->r[{rb}]) >> 32);"
            )
            .ok()?;
        }
        Opcode::Mulhwu => {
            writeln!(
                out,
                "    ctx->r[{rt}] = (uint64_t)(uint32_t)(((uint64_t)(uint32_t)ctx->r[{ra}] * (uint64_t)(uint32_t)ctx->r[{rb}]) >> 32);"
            )
            .ok()?;
        }
        // The high half of a doubleword product needs a wider type than the
        // operands, which C does not have, so it is built from the word
        // products the way it would be done by hand.
        Opcode::Mulhdu => {
            writeln!(
                out,
                "    ctx->r[{rt}] = xenolith_multiply_high(ctx->r[{ra}], ctx->r[{rb}]);"
            )
            .ok()?;
        }
        Opcode::Mulhd => {
            writeln!(
                out,
                "    ctx->r[{rt}] = (uint64_t)xenolith_multiply_high_signed((int64_t)ctx->r[{ra}], (int64_t)ctx->r[{rb}]);"
            )
            .ok()?;
        }
        Opcode::Divd => {
            writeln!(
                out,
                "    ctx->r[{rt}] = (uint64_t)((int64_t)ctx->r[{rb}] == 0 ? 0 : (int64_t)ctx->r[{ra}] / (int64_t)ctx->r[{rb}]);"
            )
            .ok()?;
        }
        Opcode::Divdu => {
            writeln!(
                out,
                "    ctx->r[{rt}] = ctx->r[{rb}] == 0 ? 0 : ctx->r[{ra}] / ctx->r[{rb}];"
            )
            .ok()?;
        }
        Opcode::Fmadd | Opcode::Fmsub | Opcode::Fnmadd | Opcode::Fnmsub => {
            let frc = (instruction.word() >> 6) & 0x1f;
            let sign = if matches!(instruction.opcode(), Opcode::Fnmadd | Opcode::Fnmsub) {
                "-"
            } else {
                ""
            };
            let operator = if matches!(instruction.opcode(), Opcode::Fmsub | Opcode::Fnmsub) {
                "-"
            } else {
                "+"
            };
            writeln!(
                out,
                "    ctx->f[{rt}].f64 = {sign}((ctx->f[{ra}].f64 * ctx->f[{frc}].f64) {operator} ctx->f[{rb}].f64);"
            )
            .ok()?;
        }
        Opcode::Fmadds | Opcode::Fmsubs | Opcode::Fnmadds | Opcode::Fnmsubs => {
            let frc = (instruction.word() >> 6) & 0x1f;
            let sign = if matches!(instruction.opcode(), Opcode::Fnmadds | Opcode::Fnmsubs) {
                "-"
            } else {
                ""
            };
            let operator = if matches!(instruction.opcode(), Opcode::Fmsubs | Opcode::Fnmsubs) {
                "-"
            } else {
                "+"
            };
            writeln!(
                out,
                "    ctx->f[{rt}].f64 = (double)({sign}(((float)ctx->f[{ra}].f64 * (float)ctx->f[{frc}].f64) {operator} (float)ctx->f[{rb}].f64));"
            )
            .ok()?;
        }
        Opcode::Fsel => {
            let frc = (instruction.word() >> 6) & 0x1f;
            writeln!(
                out,
                "    ctx->f[{rt}].f64 = ctx->f[{ra}].f64 >= 0.0 ? ctx->f[{frc}].f64 : ctx->f[{rb}].f64;"
            )
            .ok()?;
        }
        Opcode::Fsqrt | Opcode::Fsqrts => {
            writeln!(
                out,
                "    ctx->f[{rt}].f64 = __builtin_sqrt(ctx->f[{rb}].f64);"
            )
            .ok()?;
        }
        Opcode::Fres => {
            writeln!(out, "    ctx->f[{rt}].f64 = 1.0 / ctx->f[{rb}].f64;").ok()?;
        }
        Opcode::Frsqrte => {
            writeln!(
                out,
                "    ctx->f[{rt}].f64 = 1.0 / __builtin_sqrt(ctx->f[{rb}].f64);"
            )
            .ok()?;
        }

        // A whole vector register moves through memory unchanged. The lanes are
        // left in the order the guest holds them, since nothing here reads one.
        Opcode::Lvx | Opcode::Lvxl | Opcode::Lvx128 => {
            let register = vector_register(instruction);
            let base = if ra == 0 {
                "0".to_owned()
            } else {
                format!("(uint32_t)ctx->r[{ra}]")
            };
            writeln!(
                out,
                "    address = ({base} + (uint32_t)ctx->r[{rb}]) & ~0xfu;"
            )
            .ok()?;
            writeln!(
                out,
                "    __builtin_memcpy(ctx->v[{register}].u8, base + address, 16);"
            )
            .ok()?;
        }
        Opcode::Stvx | Opcode::Stvxl | Opcode::Stvx128 => {
            let register = vector_register(instruction);
            let base = if ra == 0 {
                "0".to_owned()
            } else {
                format!("(uint32_t)ctx->r[{ra}]")
            };
            writeln!(
                out,
                "    address = ({base} + (uint32_t)ctx->r[{rb}]) & ~0xfu;"
            )
            .ok()?;
            writeln!(
                out,
                "    __builtin_memcpy(base + address, ctx->v[{register}].u8, 16);"
            )
            .ok()?;
        }

        // These read and write the guest's least significant byte first, which
        // is the opposite of every other access. The bytes are assembled here
        // rather than swapped after loading, so what happens is visible without
        // knowing which end either machine keeps its numbers at.
        Opcode::Lwbrx | Opcode::Lhbrx | Opcode::Ldbrx => {
            let width = byte_reversed_width(instruction.opcode())?;
            let _ = writeln!(out, "    address = {};", indexed_address(ra, rb));
            let value = (0..width)
                .map(|byte| {
                    format!(
                        "((uint{width2}_t)xenolith_load8(base, address + {byte}) << {shift})",
                        width2 = width * 8,
                        shift = byte * 8
                    )
                })
                .collect::<Vec<_>>()
                .join(" | ");
            let _ = writeln!(out, "    ctx->r[{rt}] = {value};");
        }
        Opcode::Stwbrx | Opcode::Sthbrx | Opcode::Stdbrx => {
            let width = byte_reversed_width(instruction.opcode())?;
            let _ = writeln!(out, "    address = {};", indexed_address(ra, rb));
            for byte in 0..width {
                let _ = writeln!(
                    out,
                    "    xenolith_store8(base, address + {byte}, (uint8_t)(ctx->r[{rt}] >> {}));",
                    byte * 8
                );
            }
        }

        // A reservation is taken and redeemed through the runtime rather than
        // becoming a load and a store, so that an environment with threads has
        // somewhere to put real atomicity.
        Opcode::Lwarx | Opcode::Ldarx => {
            let width = if instruction.opcode() == Opcode::Lwarx {
                32
            } else {
                64
            };
            let _ = writeln!(out, "    address = {};", indexed_address(ra, rb));
            let _ = writeln!(
                out,
                "    ctx->r[{rt}] = xenolith_reserve{width}(base, address);"
            );
        }
        Opcode::Stwcx | Opcode::Stdcx => {
            let (width, cast) = if instruction.opcode() == Opcode::Stwcx {
                (32, "(uint32_t)")
            } else {
                (64, "")
            };
            let _ = writeln!(out, "    address = {};", indexed_address(ra, rb));
            // The architecture sets the field to two zero bits, whether the
            // store happened, and the summary overflow bit, in that order.
            let _ = writeln!(out, "    ctx->cr[0].lt = 0;\n    ctx->cr[0].gt = 0;");
            let _ = writeln!(
                out,
                "    ctx->cr[0].eq = xenolith_conditional{width}(base, address, {cast}ctx->r[{rt}]);"
            );
            let _ = writeln!(out, "    ctx->cr[0].so = (uint8_t)(ctx->xer >> 31) & 1;");
        }

        // Zeroing a block is the one cache instruction with an effect a
        // program can see, so it is the one that is not nothing.
        //
        // The long form shares this opcode and is told apart by the field the
        // other forms spend on a register. It clears four times as much, and
        // reading it as the short form would leave most of a block untouched.
        Opcode::Dcbz => {
            let size = if rt == 1 { 128 } else { 32 };
            let _ = writeln!(out, "    address = {};", indexed_address(ra, rb));
            let _ = writeln!(out, "    xenolith_zero_block(base, address, {size});");
        }

        // Cache hints change no register this model describes, and control
        // transfer is written by the caller, which knows the graph. Neither
        // contributes a statement here.
        Opcode::Dcbt
        | Opcode::Dcbtst
        | Opcode::Dcbf
        | Opcode::Dcbst
        | Opcode::Icbi
        | Opcode::B
        | Opcode::Bc
        | Opcode::Bclr
        | Opcode::Bcctr => {}

        _ => out.push_str(&vector_code(instruction)?),
    }

    if records {
        let _ = writeln!(
            out,
            "    ctx->cr[0].lt = (int64_t)ctx->r[{recorded}] < 0;\n    \
             ctx->cr[0].gt = (int64_t)ctx->r[{recorded}] > 0;\n    \
             ctx->cr[0].eq = ctx->r[{recorded}] == 0;\n    \
             ctx->cr[0].so = (uint8_t)(ctx->xer >> 31) & 1;"
        );
    }
    let _ = address;

    Some(out)
}

/// Returns the width, signedness, and addressing of a load.
fn load_width(opcode: Opcode) -> Option<(u32, bool, bool, bool, bool)> {
    Some(match opcode {
        Opcode::Lbz => (1, false, false, false, false),
        Opcode::Lbzu => (1, false, false, true, false),
        Opcode::Lbzx => (1, false, true, false, false),
        Opcode::Lbzux => (1, false, true, true, false),
        Opcode::Lhz => (2, false, false, false, false),
        Opcode::Lhzu => (2, false, false, true, false),
        Opcode::Lhzx => (2, false, true, false, false),
        Opcode::Lhzux => (2, false, true, true, false),
        Opcode::Lha => (2, true, false, false, false),
        Opcode::Lhau => (2, true, false, true, false),
        Opcode::Lhax => (2, true, true, false, false),
        Opcode::Lhaux => (2, true, true, true, false),
        Opcode::Lwz => (4, false, false, false, false),
        Opcode::Lwzu => (4, false, false, true, false),
        Opcode::Lwzx => (4, false, true, false, false),
        Opcode::Lwzux => (4, false, true, true, false),
        Opcode::Lwa => (4, true, false, false, false),
        Opcode::Lwax => (4, true, true, false, false),
        Opcode::Lwaux => (4, true, true, true, false),
        Opcode::Ld => (8, false, false, false, false),
        Opcode::Ldu => (8, false, false, true, false),
        Opcode::Ldx => (8, false, true, false, false),
        Opcode::Ldux => (8, false, true, true, false),
        Opcode::Lfs => (4, false, false, false, true),
        Opcode::Lfsu => (4, false, false, true, true),
        Opcode::Lfsx => (4, false, true, false, true),
        Opcode::Lfsux => (4, false, true, true, true),
        Opcode::Lfd => (8, false, false, false, true),
        Opcode::Lfdu => (8, false, false, true, true),
        Opcode::Lfdx => (8, false, true, false, true),
        Opcode::Lfdux => (8, false, true, true, true),
        _ => return None,
    })
}

/// Returns the width and addressing of a store.
fn store_width(opcode: Opcode) -> Option<(u32, bool, bool, bool)> {
    Some(match opcode {
        Opcode::Stb => (1, false, false, false),
        Opcode::Stbu => (1, false, true, false),
        Opcode::Stbx => (1, true, false, false),
        Opcode::Stbux => (1, true, true, false),
        Opcode::Sth => (2, false, false, false),
        Opcode::Sthu => (2, false, true, false),
        Opcode::Sthx => (2, true, false, false),
        Opcode::Sthux => (2, true, true, false),
        Opcode::Stw => (4, false, false, false),
        Opcode::Stwu => (4, false, true, false),
        Opcode::Stwx => (4, true, false, false),
        Opcode::Stwux => (4, true, true, false),
        Opcode::Std => (8, false, false, false),
        Opcode::Stdu => (8, false, true, false),
        Opcode::Stdx => (8, true, false, false),
        Opcode::Stdux => (8, true, true, false),
        Opcode::Stfs => (4, false, false, true),
        Opcode::Stfsu => (4, false, true, true),
        Opcode::Stfsx => (4, true, false, true),
        Opcode::Stfsux => (4, true, true, true),
        Opcode::Stfd => (8, false, false, true),
        Opcode::Stfdu => (8, false, true, true),
        Opcode::Stfdx => (8, true, false, true),
        Opcode::Stfdux => (8, true, true, true),
        _ => return None,
    })
}

/// Returns the C for one instruction, if it can be written.
///
/// Exposed so that a test can run one instruction through exactly the code this
/// crate emits, rather than through a second implementation written for the
/// test which would only ever agree with itself.
///
/// # Errors
///
/// Returns `None` for an instruction this crate cannot express.
#[must_use]
pub fn code_for(instruction: Instruction, address: u32) -> Option<String> {
    code_of(instruction, address)
}

/// Returns whether an instruction can be both described and written out.
#[must_use]
pub fn is_liftable(instruction: Instruction, address: u32) -> bool {
    effect_of(instruction).is_some() && code_of(instruction, address).is_some()
}

/// Writes the branch ending a block.
fn terminator_code(
    out: &mut String,
    function: &Function,
    instruction: Instruction,
    address: u32,
    starts: &std::collections::BTreeSet<u32>,
    calls: &mut std::collections::BTreeSet<u32>,
) {
    let flow = instruction.flow(address);
    let next = address.saturating_add(INSTRUCTION_SIZE);

    match flow.kind {
        FlowKind::Return => out.push_str("    return;\n"),
        FlowKind::Call => {
            if instruction.link_bit() {
                let _ = writeln!(out, "    ctx->lr = 0x{next:08x}u;");
            }
            match flow.target {
                Some(target) => {
                    calls.insert(target);
                    let _ = writeln!(out, "    {}(ctx, base);", name_of(target));
                }
                None => {
                    let _ = writeln!(out, "    xenolith_dispatch(ctx, base, (uint32_t)ctx->ctr);");
                }
            }
        }
        FlowKind::Branch => {
            let taken = match flow.target {
                Some(target) if starts.contains(&target) => format!("goto {};", label_of(target)),
                // A branch out of the function is a tail call, which returns
                // once the callee has.
                Some(target) => {
                    calls.insert(target);
                    format!("{}(ctx, base); return;", name_of(target))
                }
                None => "xenolith_dispatch(ctx, base, (uint32_t)ctx->ctr); return;".to_owned(),
            };

            if flow.falls_through {
                let _ = writeln!(out, "    if ({}) {{ {taken} }}", condition_of(instruction));
            } else {
                let _ = writeln!(out, "    {taken}");
            }
        }
        FlowKind::Indirect => {
            let targets = function.resolved.get(&address);
            match targets {
                Some(targets) if !targets.is_empty() => {
                    out.push_str("    switch ((uint32_t)ctx->ctr) {\n");
                    let mut seen = std::collections::BTreeSet::new();
                    for target in targets {
                        if !seen.insert(*target) {
                            continue;
                        }
                        if starts.contains(target) {
                            let _ = writeln!(
                                out,
                                "    case 0x{target:08x}u: goto {};",
                                label_of(*target)
                            );
                        } else {
                            calls.insert(*target);
                            let _ = writeln!(
                                out,
                                "    case 0x{target:08x}u: {}(ctx, base); return;",
                                name_of(*target)
                            );
                        }
                    }
                    out.push_str(
                        "    default: xenolith_dispatch(ctx, base, (uint32_t)ctx->ctr); return;\n    }\n",
                    );
                }
                _ => {
                    out.push_str(
                        "    xenolith_dispatch(ctx, base, (uint32_t)ctx->ctr);\n    return;\n",
                    );
                }
            }
        }
        FlowKind::Continue => {}
    }
}

/// Writes where control goes when it continues past a block.
///
/// Inside the function that is a branch to a label. Outside it, control has
/// entered another function, so that one is called and returned from. Writing
/// nothing would let the emitted function run off its end, which loses the rest
/// of the program quietly.
fn continue_at(
    out: &mut String,
    address: u32,
    starts: &std::collections::BTreeSet<u32>,
    calls: &mut std::collections::BTreeSet<u32>,
) {
    if starts.contains(&address) {
        writeln!(out, "    goto {};", label_of(address)).ok();
        return;
    }
    calls.insert(address);
    writeln!(out, "    {}(ctx, base);\n    return;", name_of(address)).ok();
}

/// Returns the C condition a conditional branch tests.
fn condition_of(instruction: Instruction) -> String {
    let bo = instruction.branch_condition();
    let bi = instruction.branch_condition_bit();
    let field = bi >> 2;
    let bit = match bi & 3 {
        0 => "lt",
        1 => "gt",
        2 => "eq",
        _ => "so",
    };

    // Bit four of the condition operand says the branch is taken whatever the
    // condition register holds.
    if bo & 0b1_0000 != 0 {
        return "1".to_owned();
    }
    // Bit one says the branch is taken when the bit is set rather than clear.
    if bo & 0b0_1000 != 0 {
        format!("ctx->cr[{field}].{bit}")
    } else {
        format!("!ctx->cr[{field}].{bit}")
    }
}

/// Returns a C string literal holding the given text.
///
/// The text comes from a container, which is input rather than something this
/// crate chose, so anything that could end the literal early or leave the source
/// malformed is escaped. Octal is used because it takes a fixed three digits and
/// so cannot absorb the character after it.
fn string_literal(text: &str) -> String {
    let mut out = String::from("\"");
    for byte in text.bytes() {
        match byte {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7e => out.push(char::from(byte)),
            _ => {
                let _ = write!(out, "\\{byte:03o}");
            }
        }
    }
    out.push('"');
    out
}

/// Returns the C for a thunk that calls an imported function.
fn import_thunk(address: u32, imported: &Imported) -> Lifted {
    let mut code = String::new();
    let _ = writeln!(
        code,
        "/* {address:#010x}  import: {} ordinal {} */",
        imported.library, imported.ordinal
    );
    let _ = writeln!(
        code,
        "void {}(xenolith_context *ctx, uint8_t *base) {{",
        name_of(address)
    );
    let _ = writeln!(
        code,
        "    xenolith_import(ctx, base, {}, {});",
        string_literal(&imported.library),
        imported.ordinal
    );
    let _ = writeln!(code, "}}");

    Lifted {
        code,
        calls: std::collections::BTreeSet::new(),
    }
}

/// Turns a discovered function into C.
///
/// # Errors
///
/// Returns the instruction that stopped it when any instruction in the function
/// cannot be both described and written out. Nothing is emitted in that case,
/// because a function that is right except in one place compiles and runs and is
/// wrong, and nothing downstream can tell.
pub fn lift(image: &Image, function: &Function, imports: &Imports) -> Result<Lifted, Unlifted> {
    // A thunk's first two words are placeholders the console's loader
    // overwrites with the address it resolved, so there is nothing to decode
    // and the container is the only thing that says what it stands for.
    if let Some(imported) = imports.get(&function.start) {
        return Ok(import_thunk(function.start, imported));
    }

    // A function whose walk decoded nothing has no blocks, and so no label to
    // enter at. There is nothing to emit and saying so is the answer.
    if function.blocks.is_empty() {
        return Err(Unlifted {
            function: function.start,
            address: function.start,
            mnemonic: "<no blocks>",
        });
    }

    let starts: std::collections::BTreeSet<u32> =
        function.blocks.iter().map(|block| block.start).collect();

    let mut calls = std::collections::BTreeSet::new();
    let mut body = String::new();
    let mut blocks: Vec<_> = function.blocks.iter().collect();
    blocks.sort_by_key(|block| block.start);

    for block in blocks {
        let _ = writeln!(body, "{}:;", label_of(block.start));

        let mut address = block.start;
        while address < block.end {
            let Ok(word) = image.u32(address) else {
                return Err(Unlifted {
                    function: function.start,
                    address,
                    mnemonic: "<unreadable>",
                });
            };
            let instruction = Instruction::decode(word);

            let Some(code) = code_of(instruction, address) else {
                return Err(Unlifted {
                    function: function.start,
                    address,
                    mnemonic: instruction.opcode().mnemonic(),
                });
            };
            if effect_of(instruction).is_none() {
                return Err(Unlifted {
                    function: function.start,
                    address,
                    mnemonic: instruction.opcode().mnemonic(),
                });
            }

            let _ = writeln!(
                body,
                "    /* {:#010x}  {} */",
                address,
                instruction.render(address)
            );
            body.push_str(&code);

            let last = address.saturating_add(INSTRUCTION_SIZE) >= block.end;
            if last {
                if let Terminator::Transfer { .. } = block.terminator {
                    terminator_code(
                        &mut body,
                        function,
                        instruction,
                        address,
                        &starts,
                        &mut calls,
                    );

                    // Where control continues afterwards is stated rather than
                    // left to the order the blocks happen to be written in,
                    // which nothing downstream should have to rely on.
                    if instruction.flow(address).falls_through {
                        continue_at(&mut body, block.end, &starts, &mut calls);
                    }
                }
            }

            address = address.saturating_add(INSTRUCTION_SIZE);
        }

        // A block that runs into the next one says so for the same reason.
        if let Terminator::FallsInto { next } = block.terminator {
            continue_at(&mut body, next, &starts, &mut calls);
        }
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "/* {:#010x} */\nvoid {}(xenolith_context *ctx, uint8_t *base) {{\n    uint32_t address;\n    (void)address;\n    (void)ctx;\n    (void)base;\n    goto {};\n\n{body}}}",
        function.start,
        name_of(function.start),
        label_of(function.start)
    );

    Ok(Lifted { code: out, calls })
}
