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

/// Writes the statements that set a condition field from a signed comparison.
fn compare_signed(into: &mut String, field: u8, left: &str, right: &str) {
    let _ = writeln!(
        into,
        "    ctx->cr[{field}].lt = ({left}) < ({right});\n    \
         ctx->cr[{field}].gt = ({left}) > ({right});\n    \
         ctx->cr[{field}].eq = ({left}) == ({right});\n    \
         ctx->cr[{field}].so = (uint8_t)(ctx->xer >> 31) & 1;"
    );
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
    let records = instruction
        .form()
        .is_some_and(xenolith_ppc::Form::has_record_bit)
        && instruction.record_bit()
        || matches!(instruction.opcode(), Opcode::Andi | Opcode::Andis);

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
        Opcode::Addi | Opcode::Addic => {
            let _ = writeln!(
                out,
                "    ctx->r[{rt}] = ctx->r[{ra}] + (uint64_t)(int64_t)({displacement});"
            );
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
                "    ctx->r[{rt}] = (uint64_t)(int64_t)({displacement}) - ctx->r[{ra}];"
            );
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
                "    ctx->r[{rt}] = (uint64_t)(int64_t)(int32_t)((int32_t)ctx->r[{rb}] == 0 ? 0 : (int32_t)ctx->r[{ra}] / (int32_t)ctx->r[{rb}]);"
            );
        }
        Opcode::Divwu => {
            let _ = writeln!(
                out,
                "    ctx->r[{rt}] = (uint32_t)ctx->r[{rb}] == 0 ? 0 : (uint32_t)ctx->r[{ra}] / (uint32_t)ctx->r[{rb}];"
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
            compare_signed(&mut out, u8::try_from(field).unwrap_or(0), &left, &right);
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
            compare_signed(&mut out, u8::try_from(field).unwrap_or(0), &left, &right);
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

        Opcode::AddicRc => {
            writeln!(
                out,
                "    ctx->r[{rt}] = ctx->r[{ra}] + (uint64_t)(int64_t)({displacement});"
            )
            .ok()?;
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
        Opcode::Srawi => {
            let places = u32::from(instruction.shift_amount());
            writeln!(
                out,
                "    ctx->r[{ra}] = (uint64_t)(int64_t)((int32_t)ctx->r[{rt}] >> {places});"
            )
            .ok()?;
        }
        Opcode::Sraw => {
            writeln!(
                out,
                "    ctx->r[{ra}] = (uint64_t)(int64_t)((int32_t)ctx->r[{rt}] >> ((ctx->r[{rb}] & 0x3f) < 32 ? (ctx->r[{rb}] & 0x3f) : 31));"
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
                format!("{}", instruction.long_shift_amount())
            } else {
                format!("((ctx->r[{rb}] & 0x7f) < 64 ? (ctx->r[{rb}] & 0x7f) : 63)")
            };
            writeln!(
                out,
                "    ctx->r[{ra}] = (uint64_t)((int64_t)ctx->r[{rt}] >> {places});"
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
                "    ctx->r[{rt}] = (uint64_t)(int64_t)(int32_t)(((int64_t)(int32_t)ctx->r[{ra}] * (int64_t)(int32_t)ctx->r[{rb}]) >> 32);"
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

        _ => return None,
    }

    if records {
        let _ = writeln!(
            out,
            "    ctx->cr[0].lt = (int64_t)(int32_t)ctx->r[{recorded}] < 0;\n    \
             ctx->cr[0].gt = (int64_t)(int32_t)ctx->r[{recorded}] > 0;\n    \
             ctx->cr[0].eq = (int32_t)ctx->r[{recorded}] == 0;\n    \
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

/// Turns a discovered function into C.
///
/// # Errors
///
/// Returns the instruction that stopped it when any instruction in the function
/// cannot be both described and written out. Nothing is emitted in that case,
/// because a function that is right except in one place compiles and runs and is
/// wrong, and nothing downstream can tell.
pub fn lift(image: &Image, function: &Function) -> Result<Lifted, Unlifted> {
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
