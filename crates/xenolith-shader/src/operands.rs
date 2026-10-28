//! Reading what an instruction reads.
//!
//! An instruction names its sources as registers or as constants, moves their
//! lanes about, and turns their signs. All of that happens before any arithmetic
//! does, and none of it depends on which operation follows, so it is kept apart
//! from the operations themselves.

use crate::constants::CONSTANTS;
use crate::opcodes;
use crate::preamble::Named;
use crate::spirv::Module;
use crate::{Alu, Kind};

/// Reads one of the constants a shader was handed, giving the name of what was
/// read.
///
/// The number naming it is written out here rather than taken from the ones
/// prepared for registers, because a constant can be named past where those
/// stop.
pub(crate) fn read_a_constant(module: &mut Module, named: &Named, which: u32) -> u32 {
    let Some(index) = usize::try_from(which)
        .ok()
        .and_then(|which| named.indices.get(which))
    else {
        return 0;
    };
    read_the_constant_at(module, named, index.number())
}

/// Reads the constant a whole number worked out while the shader runs names.
fn read_the_constant_at(module: &mut Module, named: &Named, index: u32) -> u32 {
    let where_from = module.name();
    module.add(
        opcodes::ACCESS_CHAIN,
        &[
            named.to_constant.number(),
            where_from.number(),
            named.constants.number(),
            named.zero_index.number(),
            index,
        ],
    );

    let value = module.name();
    module.add(
        opcodes::LOAD,
        &[named.four.number(), value.number(), where_from.number()],
    );
    value.number()
}

/// Moves the lanes of what was read about, takes how far it is from nought, and
/// turns its sign, in that order, as the instruction that reads it says to.
///
/// The two bits for a lane say how far along to reach rather than which lane to
/// take, so nought is a source read as it stands and nothing is done. Over half
/// of one title's instructions say something else, and reading those as they
/// stand takes the right register and the wrong numbers out of it.
pub(crate) fn moved_about(
    module: &mut Module,
    named: &Named,
    value: u32,
    along: u8,
    absolute: bool,
    negated: bool,
) -> u32 {
    let mut value = value;

    if along != 0 {
        let moved = module.name();
        let mut operands = vec![named.four.number(), moved.number(), value, value];
        for lane in 0..4u32 {
            let far = u32::from(along >> (lane * 2)) & 3;
            operands.push((lane + far) & 3);
        }
        module.add(opcodes::VECTOR_SHUFFLE, &operands);
        value = moved.number();
    }

    if absolute {
        let unsigned = module.name();
        module.add(
            opcodes::EXT_INST,
            &[
                named.four.number(),
                unsigned.number(),
                named.extended.number(),
                opcodes::GLSL_ABSOLUTE,
                value,
            ],
        );
        value = unsigned.number();
    }

    if negated {
        let turned = module.name();
        module.add(
            opcodes::FLOAT_NEGATE,
            &[named.four.number(), turned.number(), value],
        );
        value = turned.number();
    }

    value
}

/// Reads one register, giving the name of what was read.
pub(crate) fn read(
    module: &mut Module,
    named: &Named,
    which: u8,
    from_register: bool,
    along: u8,
    absolute: bool,
    negated: bool,
) -> u32 {
    let read = read_whole(module, named, which, from_register);
    moved_about(module, named, read, along, absolute, negated)
}

/// Reads one of an instruction's three sources, moved about and signed the way
/// the instruction says.
pub(crate) fn read_source(module: &mut Module, named: &Named, alu: Alu, which: usize) -> u32 {
    let register = at(alu.sources_are_registers(), which);
    let whole = read_source_whole(module, named, alu, which, register);
    moved_about(
        module,
        named,
        whole,
        at(alu.source_lanes(), which),
        at(alu.sources_are_absolute(), which),
        at(alu.sources_are_negated(), which),
    )
}

/// Reads one of an instruction's sources as a register or a constant, before
/// its lanes are moved about.
///
/// A constant that follows the address is counted on from the address the
/// shader last kept. Characters are drawn this way: each vertex names the bone
/// it hangs from, and the bone's place is read from constants counted on from
/// that, so read from the start every vertex hung from the same bone.
pub(crate) fn read_source_whole(
    module: &mut Module,
    named: &Named,
    alu: Alu,
    which: usize,
    register: bool,
) -> u32 {
    let number = at(alu.sources(), which);
    if !register && alu.constant_follows_the_address(which) {
        read_after_the_address(module, named, number)
    } else {
        read_whole(module, named, number, register)
    }
}

/// One of the three things an instruction says about its sources.
fn at<T: Copy + Default>(all: [T; 3], which: usize) -> T {
    all.get(which).copied().unwrap_or_default()
}

/// Reads the constant `which` places on from the address a shader last kept.
///
/// The sum is kept among the shader's own constants, because one counted past
/// either end would read the other kind's or nothing a host has to answer for.
fn read_after_the_address(module: &mut Module, named: &Named, which: u8) -> u32 {
    let (Some(on), Some(first), Some(last)) = (
        named.indices.get(usize::from(which)),
        named.indices.first(),
        named.indices.get(CONSTANTS as usize - 1),
    ) else {
        return 0;
    };
    let along = match named.kind {
        Kind::Vertex => *first,
        Kind::Pixel => match named.indices.get(CONSTANTS as usize) {
            Some(along) => *along,
            None => return 0,
        },
    };

    let kept = module.name();
    module.add(
        opcodes::LOAD,
        &[named.whole.number(), kept.number(), named.address.number()],
    );
    let counted = module.name();
    module.add(
        opcodes::INTEGER_ADD,
        &[
            named.whole.number(),
            counted.number(),
            kept.number(),
            on.number(),
        ],
    );
    let inside = module.name();
    module.add(
        opcodes::EXT_INST,
        &[
            named.whole.number(),
            inside.number(),
            named.extended.number(),
            opcodes::GLSL_SIGNED_CLAMP,
            counted.number(),
            first.number(),
            last.number(),
        ],
    );
    let index = module.name();
    module.add(
        opcodes::INTEGER_ADD,
        &[
            named.whole.number(),
            index.number(),
            inside.number(),
            along.number(),
        ],
    );
    read_the_constant_at(module, named, index.number())
}

/// Keeps the address later constants are read on from, out of a number with a
/// point in it, where the instruction keeping it runs.
///
/// The console rounds it to the nearest whole number unless told to drop what
/// is past the point, and keeps it between minus two hundred and fifty six and
/// two hundred and fifty five.
pub(crate) fn keep_the_address(
    module: &mut Module,
    named: &Named,
    from: u32,
    rounded: bool,
    runs: Option<u32>,
) {
    let mut from = from;
    if rounded {
        let more = module.name();
        module.add(
            opcodes::FLOAT_ADD,
            &[
                named.float.number(),
                more.number(),
                from,
                named.half.number(),
            ],
        );
        from = more.number();
    }
    let floor = module.name();
    module.add(
        opcodes::EXT_INST,
        &[
            named.float.number(),
            floor.number(),
            named.extended.number(),
            opcodes::GLSL_FLOOR,
            from,
        ],
    );
    let (least, most) = named.address_bounds;
    let inside = module.name();
    module.add(
        opcodes::EXT_INST,
        &[
            named.float.number(),
            inside.number(),
            named.extended.number(),
            opcodes::GLSL_CLAMP,
            floor.number(),
            least.number(),
            most.number(),
        ],
    );
    let whole = module.name();
    module.add(
        opcodes::CONVERT_FLOAT_TO_SIGNED,
        &[named.whole.number(), whole.number(), inside.number()],
    );
    let mut kept = whole.number();
    if let Some(runs) = runs {
        let already = module.name();
        module.add(
            opcodes::LOAD,
            &[
                named.whole.number(),
                already.number(),
                named.address.number(),
            ],
        );
        let chosen = module.name();
        module.add(
            opcodes::SELECT,
            &[
                named.whole.number(),
                chosen.number(),
                runs,
                kept,
                already.number(),
            ],
        );
        kept = chosen.number();
    }
    module.add(opcodes::STORE, &[named.address.number(), kept]);
}

/// Reads one register whole, before its lanes are moved about.
pub(crate) fn read_whole(
    module: &mut Module,
    named: &Named,
    which: u8,
    from_register: bool,
) -> u32 {
    if !from_register {
        /* Where a shader's own numbering sits in the file the hardware keeps.
         * The two kinds do not share it, so the second kind's first constant is
         * the file's two hundred and fifty seventh. */
        let along = match named.kind {
            Kind::Vertex => 0,
            Kind::Pixel => CONSTANTS,
        };
        return read_a_constant(module, named, u32::from(which) + along);
    }

    let Some(index) = named.indices.get(usize::from(which % 64)) else {
        return 0;
    };

    let where_from = module.name();
    module.add(
        opcodes::ACCESS_CHAIN,
        &[
            named.to_one.number(),
            where_from.number(),
            named.registers.number(),
            index.number(),
        ],
    );

    let value = module.name();
    module.add(
        opcodes::LOAD,
        &[named.four.number(), value.number(), where_from.number()],
    );
    value.number()
}
