//! Putting one number where another will do.
//!
//! The operations here each produce one value out of ones already worked out:
//! a lane of a vector, a choice between two, a comparison. They are what the
//! operations themselves are written from, and what the single lane half uses to
//! put its answer in a register.

use crate::Refused;
use crate::opcodes;
use crate::preamble::Named;
use crate::spirv::Module;

/// Whether one lane of a predicate push holds: the left is nothing there and
/// the right compares against nothing the way `comparing` says.
pub(crate) fn push_holds(
    module: &mut Module,
    named: &Named,
    left: u32,
    right: u32,
    comparing: u16,
    lane: u32,
) -> u32 {
    let nothing = extract(module, named, named.nothing.number(), 0);
    let a = extract(module, named, left, lane);
    let b = extract(module, named, right, lane);
    let a_is_nothing = module.name();
    module.add(
        opcodes::FLOAT_ORDERED_EQUAL,
        &[named.boolean.number(), a_is_nothing.number(), a, nothing],
    );
    let b_compares = module.name();
    module.add(
        comparing,
        &[named.boolean.number(), b_compares.number(), b, nothing],
    );
    let both = module.name();
    module.add(
        opcodes::LOGICAL_AND,
        &[
            named.boolean.number(),
            both.number(),
            a_is_nothing.number(),
            b_compares.number(),
        ],
    );
    both.number()
}

/// Returns a value whose lanes come from one of two vectors, lane by lane.
///
/// Which is what the console's write masks come to, and a host says it in one
/// instruction rather than four. `taking` holds a bit for each lane that comes
/// from the first, and the rest come from the second.
pub(crate) fn lane_by_lane(
    module: &mut Module,
    named: &Named,
    first: u32,
    second: u32,
    taking: u8,
) -> u32 {
    let mixed = module.name();
    let mut operands = vec![named.four.number(), mixed.number(), first, second];
    for lane in 0..4u32 {
        operands.push(if taking & (1 << lane) != 0 {
            lane
        } else {
            lane + 4
        });
    }
    module.add(opcodes::VECTOR_SHUFFLE, &operands);
    mixed.number()
}

/// Returns what a shader exports once the lanes it did not name are filled in.
///
/// A lane neither half of an instruction names is nothing, and one both halves
/// name is one. Only the lanes the four lane half names on its own carry what
/// it worked out. Putting that across all four, which is what writing the
/// result whole comes to, puts a number in lanes the console filled with
/// nothing.
pub(crate) fn what_is_exported(
    module: &mut Module,
    named: &Named,
    produced: u32,
    alone: Option<u32>,
    by_four: u8,
    by_one: u8,
    standing: Option<u32>,
) -> Result<u32, Refused> {
    if by_four == 0xf && by_one == 0 {
        return Ok(produced);
    }

    /* Lanes only the four lane half names carry what it worked out. The rest
     * keep what the place being exported to already stands at, and are nothing
     * where nothing has been put there yet.
     *
     * A shader may put a position together a lane at a time, one instruction
     * for each. Filling the lanes an instruction does not name with nothing
     * every time throws away the lanes the instructions before it wrote, so only
     * the last of them survives: every corner of a shape arrives at the same
     * place and a whole screen of it is drawn as a single dot. */
    let mut value = lane_by_lane(
        module,
        named,
        produced,
        standing.unwrap_or_else(|| named.nothing.number()),
        by_four & !by_one,
    );

    /* Lanes only the single lane half names carry what it worked out, put into
     * every lane first because that is what the console does with it. */
    let only_one = by_one & !by_four;
    if only_one != 0 {
        let alone = alone.ok_or(Refused::SingleLaneHalf)?;
        let across = module.name();
        module.add(
            opcodes::COMPOSITE_CONSTRUCT,
            &[
                named.four.number(),
                across.number(),
                alone,
                alone,
                alone,
                alone,
            ],
        );
        value = lane_by_lane(module, named, across.number(), value, only_one);
    }

    /* A lane both halves name is one. */
    let both = by_four & by_one;
    if both != 0 {
        value = lane_by_lane(module, named, named.all_ones.number(), value, both);
    }

    Ok(value)
}

/// Returns one lane of a value.
pub(crate) fn extract(module: &mut Module, named: &Named, from: u32, lane: u32) -> u32 {
    let one = module.name();
    module.add(
        opcodes::COMPOSITE_EXTRACT,
        &[named.float.number(), one.number(), from, lane],
    );
    one.number()
}

/// One number worked out from two, as the operation a host names for it.
pub(crate) fn worked_out(
    module: &mut Module,
    named: &Named,
    operation: u16,
    left: u32,
    right: u32,
) -> u32 {
    let made = module.name();
    module.add(
        operation,
        &[named.float.number(), made.number(), left, right],
    );
    made.number()
}

/// Whether something holds of two numbers.
pub(crate) fn holds_of(
    module: &mut Module,
    named: &Named,
    operation: u16,
    left: u32,
    right: u32,
) -> u32 {
    let made = module.name();
    module.add(
        operation,
        &[named.boolean.number(), made.number(), left, right],
    );
    made.number()
}

/// One number or the other, according to whether something holds.
pub(crate) fn one_or_the_other(
    module: &mut Module,
    named: &Named,
    holds: u32,
    yes: u32,
    no: u32,
) -> u32 {
    let made = module.name();
    module.add(
        opcodes::SELECT,
        &[named.float.number(), made.number(), holds, yes, no],
    );
    made.number()
}

/// The same number the other way about.
pub(crate) fn the_other_way(module: &mut Module, named: &Named, value: u32) -> u32 {
    let made = module.name();
    module.add(
        opcodes::FLOAT_NEGATE,
        &[named.float.number(), made.number(), value],
    );
    made.number()
}

/// How far a number is from nothing, whichever way it lies.
pub(crate) fn how_far_from_nothing(module: &mut Module, named: &Named, value: u32) -> u32 {
    let made = module.name();
    module.add(
        opcodes::EXT_INST,
        &[
            named.float.number(),
            made.number(),
            named.extended.number(),
            opcodes::GLSL_ABSOLUTE,
            value,
        ],
    );
    made.number()
}

/// Returns one of two numbers, according to whether something holds.
pub(crate) fn chosen(
    module: &mut Module,
    named: &Named,
    holds: u32,
    when: f32,
    otherwise: f32,
) -> u32 {
    let pick = |value: f32| {
        if value == 0.0 {
            named.nothing.number()
        } else {
            named.all_ones.number()
        }
    };
    let first = extract(module, named, pick(when), 0);
    let second = extract(module, named, pick(otherwise), 0);
    let made = module.name();
    module.add(
        opcodes::SELECT,
        &[named.float.number(), made.number(), holds, first, second],
    );
    made.number()
}
