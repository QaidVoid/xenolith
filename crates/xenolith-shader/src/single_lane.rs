//! The single lane half of an instruction.
//!
//! One console instruction does two things at once: something across four
//! lanes and something in one. This is the second half, which writes one place
//! in the register rather than all four.
//!
//! It also carries the predicate: an instruction may run only where the last
//! comparison said so, and a module written as one straight run says that by
//! choosing between the answer and what was already there.

use crate::arithmetic::{chosen, extract, lane_by_lane, read, what_is_exported};
use crate::opcodes;
use crate::preamble::Named;
use crate::preamble::POSITION;
use crate::spirv::Module;
use crate::{Alu, Kind, Refused, ScalarOperation};

/// Returns what one of the operations that set the predicate works out, and
/// sets it.
///
/// What is written is the other way round from what the predicate becomes:
/// nothing where it holds, one where it does not. Clearing the counter is the
/// odd one out and writes the largest number a float holds.
pub(crate) fn setting_the_predicate(
    module: &mut Module,
    named: &Named,
    operation: ScalarOperation,
    left: u32,
    predicate: &mut u32,
) -> u32 {
    if matches!(operation, ScalarOperation::PredicateClear) {
        *predicate = named.never.number();
        return named.biggest.number();
    }

    let against = extract(module, named, named.nothing.number(), 0);
    let comparing = match operation {
        ScalarOperation::PredicateEqual => opcodes::FLOAT_ORDERED_EQUAL,
        ScalarOperation::PredicateNotEqual => opcodes::FLOAT_ORDERED_NOT_EQUAL,
        ScalarOperation::PredicateGreater => opcodes::FLOAT_ORDERED_GREATER_THAN,
        _ => opcodes::FLOAT_ORDERED_GREATER_THAN_EQUAL,
    };
    let holds = module.name();
    module.add(
        comparing,
        &[named.boolean.number(), holds.number(), left, against],
    );
    *predicate = holds.number();
    chosen(module, named, holds.number(), 0.0, 1.0)
}

/// Returns what the single lane half of an instruction works out.
///
/// The two halves of an instruction run together and write different lanes of a
/// place. This half reads one or two lanes of the third source and produces one
/// number, which is then put into every lane it names.
///
/// Which lanes it reads is settled: the left hand one sits where the fourth
/// lane would be after the source is moved about, and the right hand one where
/// the first would. So the moving is done first and the two are simply the
/// fourth and the first of what comes out.
///
/// An operation with no reading here is refused rather than passed over,
/// because a half that quietly does nothing leaves a lane holding whatever the
/// other half put there.
#[allow(clippy::too_many_lines)]
pub(crate) fn single_lane(
    module: &mut Module,
    named: &Named,
    alu: Alu,
    previous: Option<u32>,
    predicate: &mut u32,
) -> Result<u32, Refused> {
    let operation = alu.scalar_operation();
    let third = read(
        module,
        named,
        alu.sources()[2],
        alu.sources_are_registers()[2],
        alu.source_lanes()[2],
        alu.sources_are_negated()[2],
    );

    let lane = |module: &mut Module, which: u32| {
        let one = module.name();
        module.add(
            opcodes::COMPOSITE_EXTRACT,
            &[named.float.number(), one.number(), third, which],
        );
        one.number()
    };
    let left = lane(module, 3);

    let two = |module: &mut Module, operation: u16, right: u32| {
        let made = module.name();
        module.add(
            operation,
            &[named.float.number(), made.number(), left, right],
        );
        made.number()
    };
    let extended = |module: &mut Module, which: u32, operands: &[u32]| {
        let made = module.name();
        let mut all = vec![
            named.float.number(),
            made.number(),
            named.extended.number(),
            which,
        ];
        all.extend_from_slice(operands);
        module.add(opcodes::EXT_INST, &all);
        made.number()
    };

    let made = match operation {
        ScalarOperation::Add => {
            let right = lane(module, 0);
            two(module, opcodes::FLOAT_ADD, right)
        }
        ScalarOperation::Subtract => {
            let right = lane(module, 0);
            two(module, opcodes::FLOAT_SUBTRACT, right)
        }
        ScalarOperation::Multiply => {
            let right = lane(module, 0);
            two(module, opcodes::FLOAT_MULTIPLY, right)
        }
        ScalarOperation::Max => {
            let right = lane(module, 0);
            extended(module, 40, &[left, right])
        }
        ScalarOperation::Min => {
            let right = lane(module, 0);
            extended(module, 37, &[left, right])
        }
        ScalarOperation::AddToPrevious => {
            let right = previous.ok_or(Refused::SingleLaneOperation(operation))?;
            two(module, opcodes::FLOAT_ADD, right)
        }
        ScalarOperation::MultiplyPrevious | ScalarOperation::MultiplyPreviousAgain => {
            let right = previous.ok_or(Refused::SingleLaneOperation(operation))?;
            two(module, opcodes::FLOAT_MULTIPLY, right)
        }
        ScalarOperation::SubtractFromPrevious => {
            let right = previous.ok_or(Refused::SingleLaneOperation(operation))?;
            let made = module.name();
            module.add(
                opcodes::FLOAT_SUBTRACT,
                &[named.float.number(), made.number(), right, left],
            );
            made.number()
        }
        ScalarOperation::KeepPrevious => previous.ok_or(Refused::SingleLaneOperation(operation))?,
        ScalarOperation::PredicateEqual
        | ScalarOperation::PredicateNotEqual
        | ScalarOperation::PredicateGreater
        | ScalarOperation::PredicateGreaterOrEqual
        | ScalarOperation::PredicateClear => {
            setting_the_predicate(module, named, operation, left, predicate)
        }
        /* One where the source compares against nothing the way the operation
         * says, and nothing otherwise. The same comparison as setting the
         * predicate, written out rather than kept. */
        ScalarOperation::SetEqual
        | ScalarOperation::SetNotEqual
        | ScalarOperation::SetGreater
        | ScalarOperation::SetGreaterOrEqual => {
            let against = extract(module, named, named.nothing.number(), 0);
            let comparing = match operation {
                ScalarOperation::SetEqual => opcodes::FLOAT_ORDERED_EQUAL,
                ScalarOperation::SetNotEqual => opcodes::FLOAT_UNORDERED_NOT_EQUAL,
                ScalarOperation::SetGreater => opcodes::FLOAT_ORDERED_GREATER_THAN,
                _ => opcodes::FLOAT_ORDERED_GREATER_THAN_EQUAL,
            };
            let holds = module.name();
            module.add(
                comparing,
                &[named.boolean.number(), holds.number(), left, against],
            );
            chosen(module, named, holds.number(), 1.0, 0.0)
        }
        /* Turning the predicate around. It holds where the source is one, and
         * what is written is nothing where it holds, one where the source was
         * nothing, and the source itself otherwise. The reading is the
         * reference emulator's. */
        ScalarOperation::PredicateInvert => {
            let one = extract(module, named, named.all_ones.number(), 0);
            let nothing = extract(module, named, named.nothing.number(), 0);
            let holds = module.name();
            module.add(
                opcodes::FLOAT_ORDERED_EQUAL,
                &[named.boolean.number(), holds.number(), left, one],
            );
            *predicate = holds.number();
            let is_nothing = module.name();
            module.add(
                opcodes::FLOAT_ORDERED_EQUAL,
                &[named.boolean.number(), is_nothing.number(), left, nothing],
            );
            let inner = module.name();
            module.add(
                opcodes::SELECT,
                &[
                    named.float.number(),
                    inner.number(),
                    is_nothing.number(),
                    one,
                    left,
                ],
            );
            let made = module.name();
            module.add(
                opcodes::SELECT,
                &[
                    named.float.number(),
                    made.number(),
                    holds.number(),
                    nothing,
                    inner.number(),
                ],
            );
            made.number()
        }
        /* Taking the predicate back off the stack. One is taken off the
         * source; the predicate holds where what is left is nothing or less,
         * and what is written is nothing where it holds and what is left
         * otherwise. The reading is the reference emulator's. */
        ScalarOperation::PredicatePop => {
            let one = extract(module, named, named.all_ones.number(), 0);
            let nothing = extract(module, named, named.nothing.number(), 0);
            let left_over = module.name();
            module.add(
                opcodes::FLOAT_SUBTRACT,
                &[named.float.number(), left_over.number(), left, one],
            );
            let holds = module.name();
            module.add(
                opcodes::FLOAT_ORDERED_LESS_THAN_EQUAL,
                &[
                    named.boolean.number(),
                    holds.number(),
                    left_over.number(),
                    nothing,
                ],
            );
            *predicate = holds.number();
            let made = module.name();
            module.add(
                opcodes::SELECT,
                &[
                    named.float.number(),
                    made.number(),
                    holds.number(),
                    nothing,
                    left_over.number(),
                ],
            );
            made.number()
        }
        ScalarOperation::MultiplyByConstant0
        | ScalarOperation::MultiplyByConstant1
        | ScalarOperation::AddConstant0
        | ScalarOperation::AddConstant1
        | ScalarOperation::SubtractConstant0
        | ScalarOperation::SubtractConstant1 => with_a_constant(module, named, alu, operation),
        _ => on_its_own(module, named, operation, left)
            .ok_or(Refused::SingleLaneOperation(operation))?,
    };

    Ok(made)
}

/// Returns what one of the single lane operations reading a constant and a
/// register together works out.
///
/// This is the one shape whose register is built rather than named, out of bits
/// scattered across the instruction. Both operands are moved about and signed
/// the same way, so the constant's is the fourth lane of what comes out and the
/// register's is the first.
pub(crate) fn with_a_constant(
    module: &mut Module,
    named: &Named,
    alu: Alu,
    operation: ScalarOperation,
) -> u32 {
    let held = read(
        module,
        named,
        alu.sources()[2],
        false,
        alu.source_lanes()[2],
        alu.sources_are_negated()[2],
    );
    let kept = read(
        module,
        named,
        alu.single_lane_register(),
        true,
        alu.source_lanes()[2],
        alu.sources_are_negated()[2],
    );

    let from_constant = module.name();
    module.add(
        opcodes::COMPOSITE_EXTRACT,
        &[named.float.number(), from_constant.number(), held, 3],
    );
    let from_register = module.name();
    module.add(
        opcodes::COMPOSITE_EXTRACT,
        &[named.float.number(), from_register.number(), kept, 0],
    );

    /* All three work on numbers with a point in them, so all three are the
     * operation for those rather than the one for whole numbers. Taking one
     * away used to name the whole number operation, which a driver rejects
     * outright, so every shader reaching it was thrown out entire. */
    let doing = match operation {
        ScalarOperation::MultiplyByConstant0 | ScalarOperation::MultiplyByConstant1 => {
            opcodes::FLOAT_MULTIPLY
        }
        ScalarOperation::AddConstant0 | ScalarOperation::AddConstant1 => opcodes::FLOAT_ADD,
        _ => opcodes::FLOAT_SUBTRACT,
    };
    let made = module.name();
    module.add(
        doing,
        &[
            named.float.number(),
            made.number(),
            from_constant.number(),
            from_register.number(),
        ],
    );
    made.number()
}

/// Returns what one of the single lane operations reading one number works out,
/// or nothing where the operation reads more than one or is not known here.
pub(crate) fn on_its_own(
    module: &mut Module,
    named: &Named,
    operation: ScalarOperation,
    left: u32,
) -> Option<u32> {
    let extended = |module: &mut Module, which: u32, operands: &[u32]| {
        let made = module.name();
        let mut all = vec![
            named.float.number(),
            made.number(),
            named.extended.number(),
            which,
        ];
        all.extend_from_slice(operands);
        module.add(opcodes::EXT_INST, &all);
        made.number()
    };

    Some(match operation {
        ScalarOperation::Fraction => extended(module, 10, &[left]),
        ScalarOperation::Truncate => extended(module, 3, &[left]),
        ScalarOperation::Floor => extended(module, 8, &[left]),
        ScalarOperation::Exponent => extended(module, 29, &[left]),
        ScalarOperation::Logarithm | ScalarOperation::LogarithmClamped => {
            extended(module, 30, &[left])
        }
        ScalarOperation::Root => extended(module, 31, &[left]),
        ScalarOperation::Sine => extended(module, 13, &[left]),
        ScalarOperation::Cosine => extended(module, 14, &[left]),
        ScalarOperation::ReciprocalRoot
        | ScalarOperation::ReciprocalRootClamped
        | ScalarOperation::ReciprocalRootRough => extended(module, 32, &[left]),
        ScalarOperation::Reciprocal
        | ScalarOperation::ReciprocalClamped
        | ScalarOperation::ReciprocalRough => {
            let one = module.name();
            module.add(
                opcodes::COMPOSITE_EXTRACT,
                &[
                    named.float.number(),
                    one.number(),
                    named.all_ones.number(),
                    0,
                ],
            );
            let made = module.name();
            module.add(
                opcodes::FLOAT_DIVIDE,
                &[named.float.number(), made.number(), one.number(), left],
            );
            made.number()
        }
        _ => return None,
    })
}

/// Returns what to write where an instruction runs on the predicate: its answer
/// where it runs, and what was already there where it does not.
///
/// A stretch that runs on the predicate is not turned into a branch. The module
/// this writes is one straight run, so every instruction is worked out and only
/// what it writes is decided, which comes to the same thing and needs no blocks.
pub(crate) fn only_where_it_runs(
    module: &mut Module,
    named: &Named,
    running: Option<u32>,
    answer: u32,
    already: u32,
) -> u32 {
    let Some(running) = running else {
        return answer;
    };

    /* Choosing between two of four lanes wants something holding or not for
     * each lane rather than one answer for all four, so the one answer is put
     * into all four first. */
    let across = module.name();
    module.add(
        opcodes::COMPOSITE_CONSTRUCT,
        &[
            named.four_bools.number(),
            across.number(),
            running,
            running,
            running,
            running,
        ],
    );

    let chosen = module.name();
    module.add(
        opcodes::SELECT,
        &[
            named.four.number(),
            chosen.number(),
            across.number(),
            answer,
            already,
        ],
    );
    chosen.number()
}

/// Writes what the single lane half worked out into the place it names.
///
/// It writes a place of its own, and what it worked out goes into every lane it
/// names.
pub(crate) fn the_single_lane_write(
    module: &mut Module,
    named: &Named,
    alu: Alu,
    alone: Option<u32>,
    running: Option<u32>,
) -> Result<(), Refused> {
    /* The single lane half writes a place of its own, and what it worked out
     * goes into every lane it names. */
    if let Some(alone) = alone
        && alu.scalar_lanes() != 0
        && !alu.exports()
    {
        let Some(index) = named
            .indices
            .get(usize::from(alu.scalar_destination() % 64))
        else {
            return Err(Refused::NotAShader);
        };

        let where_to = module.name();
        module.add(
            opcodes::ACCESS_CHAIN,
            &[
                named.to_one.number(),
                where_to.number(),
                named.registers.number(),
                index.number(),
            ],
        );

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

        let kept = if alu.scalar_lanes() == 0xf && running.is_none() {
            across.number()
        } else {
            let held = module.name();
            module.add(
                opcodes::LOAD,
                &[named.four.number(), held.number(), where_to.number()],
            );
            let kept = lane_by_lane(
                module,
                named,
                across.number(),
                held.number(),
                alu.scalar_lanes(),
            );
            only_where_it_runs(module, named, running, kept, held.number())
        };
        module.add(opcodes::STORE, &[where_to.number(), kept]);
    }

    Ok(())
}

/// Puts what one instruction worked out where the instruction says it goes,
/// and returns it where that is out of the shader rather than into a register.
pub(crate) fn put_it_where_it_goes(
    module: &mut Module,
    named: &Named,
    alu: Alu,
    produced: u32,
    alone: Option<u32>,
    running: Option<u32>,
    standing: Option<u32>,
) -> Result<Option<u32>, Refused> {
    if alu.exports() {
        /* Where an export goes is the number it names.
         *
         * A vertex shader names one number for where the vertex went and
         * others for what it carries on beside it. Treating them all as the
         * same thing puts whichever came last where the position belongs,
         * which is how a shader that draws nothing comes about. */
        let where_to = alu.vector_destination();
        if matches!(named.kind, Kind::Vertex) && where_to != POSITION {
            let Some(one) = named.carried.get(usize::from(where_to)) else {
                return Err(Refused::Export(where_to));
            };
            /* Read before writing, so that the lanes this instruction does not
             * name keep what an earlier one carried on. */
            let held = module.name();
            module.add(
                opcodes::LOAD,
                &[named.four.number(), held.number(), one.number()],
            );
            let leaving = what_is_exported(
                module,
                named,
                produced,
                alone,
                alu.vector_lanes(),
                alu.scalar_lanes(),
                Some(held.number()),
            )?;
            let leaving = only_where_it_runs(module, named, running, leaving, held.number());
            module.add(opcodes::STORE, &[one.number(), leaving]);
            return Ok(None);
        }
        let leaving = what_is_exported(
            module,
            named,
            produced,
            alone,
            alu.vector_lanes(),
            alu.scalar_lanes(),
            standing,
        )?;
        if running.is_some() {
            let held = module.name();
            module.add(
                opcodes::LOAD,
                &[named.four.number(), held.number(), named.out.number()],
            );
            return Ok(Some(only_where_it_runs(
                module,
                named,
                running,
                leaving,
                held.number(),
            )));
        }
        return Ok(Some(leaving));
    }

    {
        /* A lane the four lane half does not name keeps what it held.
         * Writing the result whole puts a number in every lane, so an
         * instruction working on two of them quietly overwrote the other
         * two, and everything reading those afterwards read that instead of
         * what was there. */
        let writing = alu.vector_lanes();
        if writing == 0 {
            return Ok(None);
        }

        let where_to = module.name();
        let Some(index) = named
            .indices
            .get(usize::from(alu.vector_destination() % 64))
        else {
            return Err(Refused::NotAShader);
        };
        module.add(
            opcodes::ACCESS_CHAIN,
            &[
                named.to_one.number(),
                where_to.number(),
                named.registers.number(),
                index.number(),
            ],
        );

        let kept = if writing == 0xf && running.is_none() {
            produced
        } else {
            let held = module.name();
            module.add(
                opcodes::LOAD,
                &[named.four.number(), held.number(), where_to.number()],
            );
            let kept = lane_by_lane(module, named, produced, held.number(), writing);
            only_where_it_runs(module, named, running, kept, held.number())
        };
        module.add(opcodes::STORE, &[where_to.number(), kept]);
    }

    the_single_lane_write(module, named, alu, alone, running)?;

    Ok(None)
}

/// Returns what says whether an instruction runs, or nothing where it always
/// does.
///
/// A stretch may run only where the predicate holds, and an instruction may say
/// the same for itself, either way round. What such an instruction writes is
/// chosen against what was already there rather than written outright.
pub(crate) fn whether_it_runs(
    module: &mut Module,
    named: &Named,
    alu: Alu,
    under: Option<bool>,
    predicate: u32,
) -> Option<u32> {
    if under.is_none() && !alu.is_predicated() {
        return None;
    }

    if under.unwrap_or_else(|| alu.predicate_holds()) {
        return Some(predicate);
    }

    let other_way = module.name();
    module.add(
        opcodes::LOGICAL_NOT,
        &[named.boolean.number(), other_way.number(), predicate],
    );
    Some(other_way.number())
}
