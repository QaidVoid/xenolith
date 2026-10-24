//! The four lane half of an instruction, written out as host arithmetic.
//!
//! One console instruction does two things at once: something across four
//! lanes and something in one. This is the first half. It is the arithmetic a
//! shader does on the numbers it was handed, said in the operations a host
//! graphics driver has, which is mostly a matter of naming: add is add, and the
//! rest is choosing which of the extended instructions matches.

use crate::Kind;
use crate::opcodes;
use crate::operands::read;
use crate::preamble::Named;
use crate::spirv::Module;
use crate::values::{
    extract, holds_of, how_far_from_nothing, one_or_the_other, push_holds, the_other_way,
    worked_out,
};
use crate::{Alu, Refused, VectorOperation};

/// The operations that read two sources and produce one value, or nothing where
/// this is one of the others.
///
/// Splitting these out keeps the ones needing a third source, a comparison, or
/// blocks of their own apart from the ones that do not.
pub(crate) fn plainly(
    module: &mut Module,
    named: &Named,
    what: VectorOperation,
    left: u32,
    right: u32,
) -> Option<u32> {
    let produced = module.name();
    match what {
        VectorOperation::Add => {
            module.add(
                opcodes::FLOAT_ADD,
                &[named.four.number(), produced.number(), left, right],
            );
        }
        VectorOperation::Multiply => {
            module.add(
                opcodes::FLOAT_MULTIPLY,
                &[named.four.number(), produced.number(), left, right],
            );
        }
        VectorOperation::Max => module.add(
            opcodes::EXT_INST,
            &[
                named.four.number(),
                produced.number(),
                named.extended.number(),
                opcodes::GLSL_MAXIMUM,
                left,
                right,
            ],
        ),
        VectorOperation::Min => module.add(
            opcodes::EXT_INST,
            &[
                named.four.number(),
                produced.number(),
                named.extended.number(),
                opcodes::GLSL_MINIMUM,
                left,
                right,
            ],
        ),
        VectorOperation::Floor => module.add(
            opcodes::EXT_INST,
            &[
                named.four.number(),
                produced.number(),
                named.extended.number(),
                opcodes::GLSL_FLOOR,
                left,
            ],
        ),
        VectorOperation::Fraction => module.add(
            opcodes::EXT_INST,
            &[
                named.four.number(),
                produced.number(),
                named.extended.number(),
                opcodes::GLSL_FRACT,
                left,
            ],
        ),
        VectorOperation::Truncate => module.add(
            opcodes::EXT_INST,
            &[
                named.four.number(),
                produced.number(),
                named.extended.number(),
                opcodes::GLSL_TRUNC,
                left,
            ],
        ),
        _ => return None,
    }
    Some(produced.number())
}

/// Two lanes multiplied and summed, with a third source added, and the one
/// number that comes out put in every lane.
pub(crate) fn dot_two_add(
    module: &mut Module,
    named: &Named,
    produced: crate::spirv::Name,
    left: u32,
    right: u32,
    last: u32,
) {
    let left_pair = module.name();
    module.add(
        opcodes::VECTOR_SHUFFLE,
        &[named.two.number(), left_pair.number(), left, left, 0, 1],
    );
    let right_pair = module.name();
    module.add(
        opcodes::VECTOR_SHUFFLE,
        &[named.two.number(), right_pair.number(), right, right, 0, 1],
    );

    let dotted = module.name();
    module.add(
        opcodes::DOT,
        &[
            named.float.number(),
            dotted.number(),
            left_pair.number(),
            right_pair.number(),
        ],
    );

    let added_to = module.name();
    module.add(
        opcodes::COMPOSITE_EXTRACT,
        &[named.float.number(), added_to.number(), last, 0],
    );
    let summed = module.name();
    module.add(
        opcodes::FLOAT_ADD,
        &[
            named.float.number(),
            summed.number(),
            dotted.number(),
            added_to.number(),
        ],
    );

    module.add(
        opcodes::COMPOSITE_CONSTRUCT,
        &[
            named.four.number(),
            produced.number(),
            summed.number(),
            summed.number(),
            summed.number(),
            summed.number(),
        ],
    );
}

/// Three lanes multiplied and summed, and the one number that comes out put in
/// every lane. The console's three lane dot, which reads only the first three
/// numbers of each source and carries the result to all four.
pub(crate) fn dot_three(
    module: &mut Module,
    named: &Named,
    produced: crate::spirv::Name,
    left: u32,
    right: u32,
) {
    let left_three = module.name();
    module.add(
        opcodes::VECTOR_SHUFFLE,
        &[
            named.three.number(),
            left_three.number(),
            left,
            left,
            0,
            1,
            2,
        ],
    );
    let right_three = module.name();
    module.add(
        opcodes::VECTOR_SHUFFLE,
        &[
            named.three.number(),
            right_three.number(),
            right,
            right,
            0,
            1,
            2,
        ],
    );

    let dotted = module.name();
    module.add(
        opcodes::DOT,
        &[
            named.float.number(),
            dotted.number(),
            left_three.number(),
            right_three.number(),
        ],
    );

    module.add(
        opcodes::COMPOSITE_CONSTRUCT,
        &[
            named.four.number(),
            produced.number(),
            dotted.number(),
            dotted.number(),
            dotted.number(),
            dotted.number(),
        ],
    );
}

/// Throws the pixel away where any lane of two values is the same.
///
/// The one operation here that produces nothing and changes where the shader
/// goes, so it is written on its own rather than as something to store.
pub(crate) fn throw_the_pixel_away(
    module: &mut Module,
    named: &Named,
    left: u32,
    right: u32,
) -> Result<(), Refused> {
    /* Only something running for a pixel can throw one away. A shader doing
     * this is a pixel shader, so being asked to say it as anything else is
     * refused rather than answered with something a driver would turn down. */
    if matches!(named.kind, Kind::Vertex) {
        return Err(Refused::Operation(VectorOperation::KillOnEqual));
    }

    let asked = module.name();
    module.add(
        opcodes::FLOAT_ORDERED_EQUAL,
        &[named.four_bools.number(), asked.number(), left, right],
    );
    let any = module.name();
    module.add(
        opcodes::ANY,
        &[named.boolean.number(), any.number(), asked.number()],
    );

    let throwing = module.name();
    let carrying_on = module.name();
    module.add(opcodes::SELECTION_MERGE, &[carrying_on.number(), 0]);
    module.add(
        opcodes::BRANCH_CONDITIONAL,
        &[any.number(), throwing.number(), carrying_on.number()],
    );
    module.add(opcodes::LABEL, &[throwing.number()]);
    module.add(opcodes::KILL, &[]);
    module.add(opcodes::LABEL, &[carrying_on.number()]);
    Ok(())
}

/// Writes out the four lane half of an instruction: the operations that read
/// two sources and produce one value, and the fused forms that read three.
#[allow(clippy::too_many_lines)]
pub(crate) fn arithmetic(
    module: &mut Module,
    named: &Named,
    alu: Alu,
    left: u32,
    right: u32,
    predicate: &mut u32,
) -> Result<Option<u32>, Refused> {
    /* The third source is taken from the instruction here rather than handed
     * in, because only the few operations that read one ever ask for it. */
    let third = alu.sources()[2];
    let third_is_register = alu.sources_are_registers()[2];
    let third_along = alu.source_lanes()[2];
    let third_negated = alu.sources_are_negated()[2];
    let third_absolute = alu.sources_are_absolute()[2];

    if matches!(alu.vector_operation(), VectorOperation::KillOnEqual) {
        throw_the_pixel_away(module, named, left, right)?;
        return Ok(None);
    }

    if let Some(produced) = plainly(module, named, alu.vector_operation(), left, right) {
        return Ok(Some(produced));
    }

    let produced = module.name();
    match alu.vector_operation() {
        /* Multiplying two and adding a third, which is the one operation
         * here that reads all three of its sources. */
        VectorOperation::MultiplyAdd => {
            let last = read(
                module,
                named,
                third,
                third_is_register,
                third_along,
                third_absolute,
                third_negated,
            );
            module.add(
                opcodes::EXT_INST,
                &[
                    named.four.number(),
                    produced.number(),
                    named.extended.number(),
                    opcodes::GLSL_FUSED_MULTIPLY_ADD,
                    left,
                    right,
                    last,
                ],
            );
        }
        /* Choosing between two of the sources on what the first one says.
         * The console compares against nothing and takes one side or the
         * other, which a host says as a comparison and a choice. */
        VectorOperation::ChooseOnEqual
        | VectorOperation::ChooseOnGreaterOrEqual
        | VectorOperation::ChooseOnGreater => {
            let last = read(
                module,
                named,
                third,
                third_is_register,
                third_along,
                third_absolute,
                third_negated,
            );
            let asked = module.name();
            let comparison = match alu.vector_operation() {
                VectorOperation::ChooseOnEqual => opcodes::FLOAT_ORDERED_EQUAL,
                VectorOperation::ChooseOnGreater => opcodes::FLOAT_ORDERED_GREATER_THAN,
                _ => opcodes::FLOAT_ORDERED_GREATER_THAN_EQUAL,
            };
            module.add(
                comparison,
                &[
                    named.four_bools.number(),
                    asked.number(),
                    left,
                    named.nothing.number(),
                ],
            );
            module.add(
                opcodes::SELECT,
                &[
                    named.four.number(),
                    produced.number(),
                    asked.number(),
                    right,
                    last,
                ],
            );
        }
        /* One where the comparison holds and nothing where it does not,
         * which is a comparison and a choice between two constants. */
        VectorOperation::SetEqual | VectorOperation::SetNotEqual | VectorOperation::SetGreater => {
            let comparing = match alu.vector_operation() {
                VectorOperation::SetEqual => opcodes::FLOAT_ORDERED_EQUAL,
                VectorOperation::SetNotEqual => opcodes::FLOAT_UNORDERED_NOT_EQUAL,
                _ => opcodes::FLOAT_ORDERED_GREATER_THAN,
            };
            let asked = module.name();
            module.add(
                comparing,
                &[named.four_bools.number(), asked.number(), left, right],
            );
            module.add(
                opcodes::SELECT,
                &[
                    named.four.number(),
                    produced.number(),
                    asked.number(),
                    named.all_ones.number(),
                    named.nothing.number(),
                ],
            );
        }
        VectorOperation::SetGreaterOrEqual => {
            let asked = module.name();
            module.add(
                opcodes::FLOAT_ORDERED_GREATER_THAN_EQUAL,
                &[named.four_bools.number(), asked.number(), left, right],
            );
            module.add(
                opcodes::SELECT,
                &[
                    named.four.number(),
                    produced.number(),
                    asked.number(),
                    named.all_ones.number(),
                    named.nothing.number(),
                ],
            );
        }
        /* Two lanes multiplied and summed, with a third source added, and
         * the one number that comes out put in every lane. */
        VectorOperation::DotTwoAdd => {
            let last = read(
                module,
                named,
                third,
                third_is_register,
                third_along,
                third_absolute,
                third_negated,
            );
            dot_two_add(module, named, produced, left, right, last);
        }
        VectorOperation::DotThree => {
            dot_three(module, named, produced, left, right);
        }
        /* All four lanes multiplied and summed, and the one number that comes
         * out put in every lane. */
        VectorOperation::DotFour => {
            let dot = module.name();
            module.add(
                opcodes::DOT,
                &[named.float.number(), dot.number(), left, right],
            );
            module.add(
                opcodes::COMPOSITE_CONSTRUCT,
                &[
                    named.four.number(),
                    produced.number(),
                    dot.number(),
                    dot.number(),
                    dot.number(),
                    dot.number(),
                ],
            );
        }
        /* Setting the predicate and pushing.
         *
         * Two things come out of one of these. The predicate is set from the
         * fourth lane: it holds where the left is nothing and the right
         * compares against nothing the way the operation says. What is
         * written reads the first lane the same way and is nothing where
         * that holds, and one more than the left otherwise, put in every
         * lane. This is the reading the reference emulator gives them. */
        VectorOperation::PredicateEqualPush
        | VectorOperation::PredicateNotEqualPush
        | VectorOperation::PredicateGreaterPush
        | VectorOperation::PredicateGreaterOrEqualPush => {
            let comparing = match alu.vector_operation() {
                VectorOperation::PredicateEqualPush => opcodes::FLOAT_ORDERED_EQUAL,
                VectorOperation::PredicateNotEqualPush => opcodes::FLOAT_UNORDERED_NOT_EQUAL,
                VectorOperation::PredicateGreaterPush => opcodes::FLOAT_ORDERED_GREATER_THAN,
                _ => opcodes::FLOAT_ORDERED_GREATER_THAN_EQUAL,
            };
            *predicate = push_holds(module, named, left, right, comparing, 3);
            let holds = push_holds(module, named, left, right, comparing, 0);
            let first = extract(module, named, left, 0);
            let one = extract(module, named, named.all_ones.number(), 0);
            let nothing = extract(module, named, named.nothing.number(), 0);
            let one_more = module.name();
            module.add(
                opcodes::FLOAT_ADD,
                &[named.float.number(), one_more.number(), first, one],
            );
            let picked = module.name();
            module.add(
                opcodes::SELECT,
                &[
                    named.float.number(),
                    picked.number(),
                    holds,
                    nothing,
                    one_more.number(),
                ],
            );
            module.add(
                opcodes::COMPOSITE_CONSTRUCT,
                &[
                    named.four.number(),
                    produced.number(),
                    picked.number(),
                    picked.number(),
                    picked.number(),
                    picked.number(),
                ],
            );
        }
        VectorOperation::Cube => a_cube_is_read(module, named, left, produced.number()),
        other => return Err(Refused::Operation(other)),
    }

    Ok(Some(produced.number()))
}
/// Which face of a cube a direction points at, and where on that face it lands.
///
/// The console does this in one instruction: it hands back where on the face
/// the reading falls, twice the length along whichever way the direction points
/// most, and which of the six faces that is. The three parts of the direction
/// come out of the one source read three ways round, which is how the hardware
/// is handed them, so they are taken from the lanes rather than the order they
/// are written in.
///
/// It is written without branches. Every one of the three ways is worked out
/// and then chosen between, which says the same thing as running one of three
/// stretches and leaves a driver free to do whichever is cheaper.
pub(crate) fn a_cube_is_read(module: &mut Module, named: &Named, from: u32, produced: u32) {
    /* The five operations the choice is written from, under the names this
     * function reads them by. */
    const AT_LEAST: u16 = opcodes::FLOAT_ORDERED_GREATER_THAN_EQUAL;
    const BELOW: u16 = opcodes::FLOAT_ORDERED_LESS_THAN;
    const BOTH: u16 = opcodes::LOGICAL_AND;
    const ADDED: u16 = opcodes::FLOAT_ADD;
    const MULTIPLIED: u16 = opcodes::FLOAT_MULTIPLY;

    let along_x = extract(module, named, from, 2);
    let along_y = extract(module, named, from, 3);
    let along_z = extract(module, named, from, 0);

    let none = named.no_number.number();
    let one = extract(module, named, named.all_ones.number(), 0);
    let two = worked_out(module, named, ADDED, one, one);
    let three = worked_out(module, named, ADDED, two, one);
    let four = worked_out(module, named, ADDED, two, two);
    let five = worked_out(module, named, ADDED, four, one);

    let far_x = how_far_from_nothing(module, named, along_x);
    let far_y = how_far_from_nothing(module, named, along_y);
    let far_z = how_far_from_nothing(module, named, along_z);

    let over_x = holds_of(module, named, AT_LEAST, far_z, far_x);
    let over_y = holds_of(module, named, AT_LEAST, far_z, far_y);
    let mostly_z = module.name();
    module.add(
        BOTH,
        &[named.boolean.number(), mostly_z.number(), over_x, over_y],
    );
    let mostly_y = holds_of(module, named, AT_LEAST, far_y, far_x);

    let x_behind = holds_of(module, named, BELOW, along_x, none);
    let y_behind = holds_of(module, named, BELOW, along_y, none);
    let z_behind = holds_of(module, named, BELOW, along_z, none);

    let back_x = the_other_way(module, named, along_x);
    let back_y = the_other_way(module, named, along_y);
    let back_z = the_other_way(module, named, along_z);

    /* Across the face. Where the way it points most is upwards it turns with
     * the sign, and otherwise it is the same either way. */
    let across_when_y = one_or_the_other(module, named, y_behind, back_z, along_z);
    let across = one_or_the_other(module, named, mostly_y, across_when_y, back_y);
    let across = one_or_the_other(module, named, mostly_z.number(), back_y, across);

    /* Along the face. */
    let down_when_z = one_or_the_other(module, named, z_behind, back_x, along_x);
    let down_when_x = one_or_the_other(module, named, x_behind, along_z, back_z);
    let down = one_or_the_other(module, named, mostly_y, along_x, down_when_x);
    let down = one_or_the_other(module, named, mostly_z.number(), down_when_z, down);

    /* Twice the length along the way it points most. */
    let twice_z = worked_out(module, named, MULTIPLIED, two, along_z);
    let twice_y = worked_out(module, named, MULTIPLIED, two, along_y);
    let twice_x = worked_out(module, named, MULTIPLIED, two, along_x);
    let twice = one_or_the_other(module, named, mostly_y, twice_y, twice_x);
    let twice = one_or_the_other(module, named, mostly_z.number(), twice_z, twice);

    /* Which of the six faces, numbered the console's way. */
    let face_z = one_or_the_other(module, named, z_behind, five, four);
    let face_y = one_or_the_other(module, named, y_behind, three, two);
    let face_x = one_or_the_other(module, named, x_behind, one, none);
    let face = one_or_the_other(module, named, mostly_y, face_y, face_x);
    let face = one_or_the_other(module, named, mostly_z.number(), face_z, face);

    module.add(
        opcodes::COMPOSITE_CONSTRUCT,
        &[named.four.number(), produced, across, down, twice, face],
    );
}
