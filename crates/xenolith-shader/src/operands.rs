//! Reading what an instruction reads.
//!
//! An instruction names its sources as registers or as constants, moves their
//! lanes about, and turns their signs. All of that happens before any arithmetic
//! does, and none of it depends on which operation follows, so it is kept apart
//! from the operations themselves.

use crate::Kind;
use crate::constants::CONSTANTS;
use crate::opcodes;
use crate::preamble::Named;
use crate::spirv::Module;

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

    let where_from = module.name();
    module.add(
        opcodes::ACCESS_CHAIN,
        &[
            named.to_constant.number(),
            where_from.number(),
            named.constants.number(),
            named.zero_index.number(),
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

/// Moves the lanes of what was read about and turns its sign, as the
/// instruction that reads it says to.
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
    negated: bool,
) -> u32 {
    let read = read_whole(module, named, which, from_register);
    moved_about(module, named, read, along, negated)
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
