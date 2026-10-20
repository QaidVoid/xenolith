//! Saying in a host's terms what a shader says in the console's.
//!
//! A shader is registers of four floats and instructions that read some and
//! write one. A host takes the same idea, so most of the work is naming things:
//! the registers become one array a shader keeps to itself, an instruction
//! becomes a read of two of them and a write of a third, and what the console
//! calls exporting becomes a store to something the host can see.
//!
//! What is not known is not written. An operation this has no reading of stops
//! the translation and is reported, rather than being turned into something
//! that looks similar, because a shader that draws the wrong thing is harder to
//! find than one that was never translated.

use crate::arithmetic::{
    arithmetic, chosen, extract, lane_by_lane, read, read_a_constant, what_is_exported,
};
use crate::fetching::{Fetches, go_and_fetch, what_the_fetches_need};
use crate::layout::{Asking, Step, Stretch, WhereNext, stretches, stretches_that_jump};
use crate::opcodes;
use crate::preamble::{ALL_THE_CONSTANTS, Named, POSITION, THE_COUNTS, THE_TRUTHS, preamble};

use crate::{ALU_WORDS, Alu, Kind, ScalarOperation, VectorOperation, spirv::Module};

/// What came of translating a shader.
#[derive(Debug)]
pub struct Translated {
    /// The module, as the words a driver reads.
    pub words: Vec<u32>,
    /// Where in a title's memory the shader goes for each of its streams.
    pub streams: Vec<Stream>,
    /// Which of the places a picture is kept the shader reads one from.
    pub pictures: Vec<u32>,
}

/// One stream a shader fetches from, in the terms its address is worked out in.
///
/// A host needs these where it has to lay a vertex out itself rather than hand
/// over what a title wrote, which is what a shape the hardware completes asks
/// for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stream {
    /// Which of the fetch constants says where the stream is.
    pub slot: u32,
    /// How far apart two vertices sit in it, in words.
    pub stride: u32,
    /// How far into a vertex this fetch reads, in words.
    pub offset: u32,
    /// How many words it reads.
    pub words: u32,
}

/// Why a shader was not translated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The words do not read as a shader at all.
    NotAShader,
    /// An operation this has no reading of, which is reported rather than
    /// guessed at.
    Operation(VectorOperation),
    /// The shader fetches its own data, which is where geometry comes from and
    /// is not something this can say yet. Reading such an instruction as
    /// arithmetic would put out a module that runs and means nothing, so it is
    /// refused instead.
    Fetch,
    /// An operation of the single lane half this has no reading of.
    SingleLaneOperation(ScalarOperation),
    /// A lane is wanted from the single lane half of an instruction, which is
    /// not modelled.
    ///
    /// The two halves of an instruction write different lanes of the same
    /// place, and only the four lane half is read here. Where the single lane
    /// half is the one naming a lane, what belongs there cannot be worked out,
    /// and putting the other half's answer in would be putting a number the
    /// console never wrote.
    SingleLaneHalf,
    /// The shader exports somewhere this carries nothing to.
    ///
    /// The console numbers a memory export apart from the things a shader
    /// carries on, and a shader writing to one of those is doing work whose
    /// result goes somewhere this has nowhere to put. Passing it over would
    /// leave a module that runs, draws, and quietly leaves out whatever the
    /// export was for.
    Export(u8),
}

/// Returns what one of the operations that set the predicate works out, and
/// sets it.
///
/// What is written is the other way round from what the predicate becomes:
/// nothing where it holds, one where it does not. Clearing the counter is the
/// odd one out and writes the largest number a float holds.
fn setting_the_predicate(
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
fn single_lane(
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
fn with_a_constant(
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
fn on_its_own(
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
fn only_where_it_runs(
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
fn the_single_lane_write(
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
fn put_it_where_it_goes(
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
fn whether_it_runs(
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

/// What carries from one instruction to the next while a shader is written out.
struct Carrying {
    previous: Option<u32>,
    predicate: u32,
    exported: Option<u32>,
    carried_over: Option<(u32, u32)>,
}

/// Writes out one instruction of a shader.
///
/// Kept apart from the two ways a shader's instructions are reached, because a
/// shader that runs straight through and one that jumps do the same thing to
/// each instruction and differ only in the order they come in.
fn one_instruction(
    module: &mut Module,
    named: &Named,
    words: &[u32],
    step: Step,
    carrying: &mut Carrying,
) -> Result<(), Refused> {
    let one = step.at;
    if step.fetches {
        go_and_fetch(module, named, words, one, &mut carrying.carried_over)?;
        return Ok(());
    }

    let Some(alu) = Alu::at(
        words,
        one.checked_mul(ALU_WORDS).ok_or(Refused::NotAShader)?,
    ) else {
        return Err(Refused::NotAShader);
    };

    let [first, second, _] = alu.sources();
    let [first_is_register, second_is_register, _] = alu.sources_are_registers();
    let [first_along, second_along, _] = alu.source_lanes();
    let [first_negated, second_negated, _] = alu.sources_are_negated();
    let left = read(
        module,
        named,
        first,
        first_is_register,
        first_along,
        first_negated,
    );
    let right = read(
        module,
        named,
        second,
        second_is_register,
        second_along,
        second_negated,
    );

    let Some(produced) = arithmetic(module, named, alu, left, right, &mut carrying.predicate)?
    else {
        return Ok(());
    };

    /* What the single lane half worked out, which the next instruction may ask
     * for by name as well as this one writing it somewhere. */
    let alone = match single_lane(
        module,
        named,
        alu,
        carrying.previous,
        &mut carrying.predicate,
    ) {
        Ok(alone) => {
            carrying.previous = Some(alone);
            Some(alone)
        }
        Err(_) if alu.scalar_lanes() == 0 => None,
        Err(why) => return Err(why),
    };

    let runs = whether_it_runs(module, named, alu, step.under, carrying.predicate);

    if let Some(leaving) =
        put_it_where_it_goes(module, named, alu, produced, alone, runs, carrying.exported)?
    {
        carrying.exported = Some(leaving);
    }
    Ok(())
}

/// Reads one of the truths a title handed over, giving the name of whether it
/// holds.
///
/// They arrive packed a bit apiece among the constants, so the one asked for is
/// found by which word holds it and which bit of that word it is.
#[allow(clippy::indexing_slicing)]
fn one_of_the_truths(module: &mut Module, named: &Named, which: u8) -> u32 {
    let which = u32::from(which);
    let word = which / 32;
    let bit = which % 32;

    let held = read_a_constant(module, named, THE_TRUTHS + word / 4);
    let one = module.name();
    module.add(
        opcodes::COMPOSITE_EXTRACT,
        &[named.float.number(), one.number(), held, word % 4],
    );
    let bits = module.name();
    module.add(
        opcodes::BITCAST,
        &[named.whole.number(), bits.number(), one.number()],
    );
    let moved = module.name();
    module.add(
        opcodes::SHIFT_RIGHT_LOGICAL,
        &[
            named.whole.number(),
            moved.number(),
            bits.number(),
            named.whole_numbers[bit as usize].number(),
        ],
    );
    let kept = module.name();
    module.add(
        opcodes::BITWISE_AND,
        &[
            named.whole.number(),
            kept.number(),
            moved.number(),
            named.whole_numbers[1].number(),
        ],
    );
    let holds = module.name();
    module.add(
        opcodes::INTEGER_NOT_EQUAL,
        &[
            named.boolean.number(),
            holds.number(),
            kept.number(),
            named.whole_numbers[0].number(),
        ],
    );
    holds.number()
}

/// Reads how many times round a loop goes, out of the counts a title handed
/// over, giving the name of the number.
fn how_many_times_round(module: &mut Module, named: &Named, which: u8) -> u32 {
    let which = u32::from(which);
    let held = read_a_constant(module, named, THE_COUNTS + which / 4);
    let one = module.name();
    module.add(
        opcodes::COMPOSITE_EXTRACT,
        &[named.float.number(), one.number(), held, which % 4],
    );
    let bits = module.name();
    module.add(
        opcodes::BITCAST,
        &[named.whole.number(), bits.number(), one.number()],
    );
    /* How many times round is the lowest eight bits of it. */
    let how_many = module.name();
    module.add(
        opcodes::BITWISE_AND,
        &[
            named.whole.number(),
            how_many.number(),
            bits.number(),
            named.a_byte.number(),
        ],
    );
    how_many.number()
}

/// Writes out what a shader puts out, and the end of it.
fn what_it_puts_out(module: &mut Module, named: &Named, exported: Option<u32>) {
    if let Some(value) = exported {
        /* What the hardware would have done to this, done here instead: a
         * console applies a scale and an offset to what a vertex shader puts
         * out, and a host has no such switch. */
        let value = if matches!(named.kind, Kind::Vertex) {
            let scale = read_a_constant(module, named, ALL_THE_CONSTANTS);
            let offset = read_a_constant(module, named, ALL_THE_CONSTANTS + 1);
            let scaled = module.name();
            module.add(
                opcodes::FLOAT_MULTIPLY,
                &[named.four.number(), scaled.number(), value, scale],
            );
            let moved = module.name();
            module.add(
                opcodes::FLOAT_ADD,
                &[named.four.number(), moved.number(), scaled.number(), offset],
            );
            moved.number()
        } else {
            value
        };

        if let Some((which, against)) = named.cutoff {
            throw_away_what_is_not_solid_enough(module, named, value, which, against);
        }

        module.add(opcodes::STORE, &[named.out.number(), value]);
    }

    module.add(opcodes::RETURN, &[]);
    module.add(opcodes::FUNCTION_END, &[]);
}

/// Writes the test a title sets on how solid a place has to be to be drawn.
///
/// The console makes this test itself, before what a pixel shader worked out
/// reaches the surface, and a host has nothing that does it, so the shader ends
/// by throwing the place away where the test does not hold.
///
/// The three bits of the test say whether a place less solid than the test,
/// exactly as solid, and more solid are each drawn, so the whole test is those
/// three comparisons and nothing else. Written this way it needs no reading of
/// which test it is: the bits and the comparisons stand side by side, and a
/// driver folds both away when the test draws everything.
fn throw_away_what_is_not_solid_enough(
    module: &mut Module,
    named: &Named,
    value: u32,
    which: crate::spirv::Name,
    against: crate::spirv::Name,
) {
    let Some(none) = named.whole_numbers.first().copied() else {
        return;
    };

    let solid = module.name();
    module.add(
        opcodes::COMPOSITE_EXTRACT,
        &[named.float.number(), solid.number(), value, 3],
    );

    let mut drawn = None;
    for (comparison, bit) in [
        (opcodes::FLOAT_ORDERED_LESS_THAN, 1usize),
        (opcodes::FLOAT_ORDERED_EQUAL, 2),
        (opcodes::FLOAT_ORDERED_GREATER_THAN, 4),
    ] {
        let Some(one) = named.whole_numbers.get(bit).copied() else {
            return;
        };

        let holds = module.name();
        module.add(
            comparison,
            &[
                named.boolean.number(),
                holds.number(),
                solid.number(),
                against.number(),
            ],
        );
        let kept = module.name();
        module.add(
            opcodes::BITWISE_AND,
            &[
                named.whole.number(),
                kept.number(),
                which.number(),
                one.number(),
            ],
        );
        let asked = module.name();
        module.add(
            opcodes::INTEGER_NOT_EQUAL,
            &[
                named.boolean.number(),
                asked.number(),
                kept.number(),
                none.number(),
            ],
        );
        let both = module.name();
        module.add(
            opcodes::LOGICAL_AND,
            &[
                named.boolean.number(),
                both.number(),
                holds.number(),
                asked.number(),
            ],
        );

        drawn = Some(match drawn {
            None => both.number(),
            Some(before) => {
                let either = module.name();
                module.add(
                    opcodes::LOGICAL_OR,
                    &[
                        named.boolean.number(),
                        either.number(),
                        before,
                        both.number(),
                    ],
                );
                either.number()
            }
        });
    }

    let Some(drawn) = drawn else {
        return;
    };

    let thrown = module.name();
    let after = module.name();
    module.add(opcodes::SELECTION_MERGE, &[after.number(), 0]);
    module.add(
        opcodes::BRANCH_CONDITIONAL,
        &[drawn, after.number(), thrown.number()],
    );
    module.add(opcodes::LABEL, &[thrown.number()]);
    module.add(opcodes::KILL, &[]);
    module.add(opcodes::LABEL, &[after.number()]);
}

/// Writes out a shader whose stretches are not run one after another.
///
/// Which stretch runs next is only settled while the shader runs, so the
/// stretches are written as one loop over a number saying where the shader is
/// up to: the loop reads that number, runs the stretch it names, writes down
/// where to go next, and comes round again. What carries between stretches is
/// kept in memory rather than in a name, because a name written in one stretch
/// is not there to be read in another.
#[allow(clippy::too_many_lines)]
fn a_shader_that_jumps(
    module: &mut Module,
    named: &Named,
    words: &[u32],
    all: &[Stretch],
) -> Result<Option<u32>, Refused> {
    let header = module.name();
    let check = module.name();
    let after = module.name();
    let onwards = module.name();
    let done = module.name();
    let nowhere = module.name();
    let labels: Vec<_> = all.iter().map(|_| module.name()).collect();
    /* The number one past the last stretch, which means the shader is done. */
    let finished = named
        .which_stretch
        .get(all.len())
        .ok_or(Refused::NotAShader)?
        .number();

    module.add(opcodes::BRANCH, &[header.number()]);

    module.add(opcodes::LABEL, &[header.number()]);
    module.add(opcodes::LOOP_MERGE, &[done.number(), onwards.number(), 0]);
    module.add(opcodes::BRANCH, &[check.number()]);

    module.add(opcodes::LABEL, &[check.number()]);
    let up_to = module.name();
    module.add(
        opcodes::LOAD,
        &[
            named.whole.number(),
            up_to.number(),
            named.where_it_is_up_to.number(),
        ],
    );
    module.add(opcodes::SELECTION_MERGE, &[after.number(), 0]);
    let mut choosing = vec![up_to.number(), nowhere.number()];
    for (at, label) in labels.iter().enumerate() {
        let Ok(at) = u32::try_from(at) else {
            return Err(Refused::NotAShader);
        };
        choosing.push(at);
        choosing.push(label.number());
    }
    module.add(opcodes::SWITCH, &choosing);

    for (at, stretch) in all.iter().enumerate() {
        let label = labels.get(at).ok_or(Refused::NotAShader)?;
        module.add(opcodes::LABEL, &[label.number()]);

        let predicate = module.name();
        module.add(
            opcodes::LOAD,
            &[
                named.boolean.number(),
                predicate.number(),
                named.predicate_kept.number(),
            ],
        );
        let previous = module.name();
        module.add(
            opcodes::LOAD,
            &[
                named.float.number(),
                previous.number(),
                named.previous_kept.number(),
            ],
        );
        let exported = module.name();
        module.add(
            opcodes::LOAD,
            &[
                named.four.number(),
                exported.number(),
                named.exported_kept.number(),
            ],
        );

        let mut carrying = Carrying {
            previous: Some(previous.number()),
            predicate: predicate.number(),
            exported: Some(exported.number()),
            carried_over: None,
        };
        for step in &stretch.steps {
            one_instruction(module, named, words, *step, &mut carrying)?;
        }

        module.add(
            opcodes::STORE,
            &[named.predicate_kept.number(), carrying.predicate],
        );
        if let Some(previous) = carrying.previous {
            module.add(opcodes::STORE, &[named.previous_kept.number(), previous]);
        }
        if let Some(exported) = carrying.exported {
            module.add(opcodes::STORE, &[named.exported_kept.number(), exported]);
        }

        let onward = named
            .which_stretch
            .get(at + 1)
            .ok_or(Refused::NotAShader)?
            .number();
        match stretch.next {
            WhereNext::Ends => {
                module.add(
                    opcodes::STORE,
                    &[named.where_it_is_up_to.number(), finished],
                );
                module.add(opcodes::BRANCH, &[after.number()]);
            }
            WhereNext::Along => {
                module.add(opcodes::STORE, &[named.where_it_is_up_to.number(), onward]);
                module.add(opcodes::BRANCH, &[after.number()]);
            }
            WhereNext::Rounds { count, past } => {
                /* How many times round, read where a title put it. Nought
                 * means the loop is passed over altogether. */
                let how_many = how_many_times_round(module, named, count);
                module.add(opcodes::STORE, &[named.times_round.number(), how_many]);
                let any = module.name();
                module.add(
                    opcodes::INTEGER_NOT_EQUAL,
                    &[
                        named.boolean.number(),
                        any.number(),
                        how_many,
                        named
                            .whole_numbers
                            .first()
                            .ok_or(Refused::NotAShader)?
                            .number(),
                    ],
                );
                let beyond = named
                    .which_stretch
                    .get(past)
                    .ok_or(Refused::NotAShader)?
                    .number();
                let chosen = module.name();
                module.add(
                    opcodes::SELECT,
                    &[
                        named.whole.number(),
                        chosen.number(),
                        any.number(),
                        onward,
                        beyond,
                    ],
                );
                module.add(
                    opcodes::STORE,
                    &[named.where_it_is_up_to.number(), chosen.number()],
                );
                module.add(opcodes::BRANCH, &[after.number()]);
            }
            WhereNext::Again { top } => {
                let left = module.name();
                module.add(
                    opcodes::LOAD,
                    &[
                        named.whole.number(),
                        left.number(),
                        named.times_round.number(),
                    ],
                );
                let fewer = module.name();
                module.add(
                    opcodes::INTEGER_SUBTRACT,
                    &[
                        named.whole.number(),
                        fewer.number(),
                        left.number(),
                        named
                            .whole_numbers
                            .get(1)
                            .ok_or(Refused::NotAShader)?
                            .number(),
                    ],
                );
                module.add(
                    opcodes::STORE,
                    &[named.times_round.number(), fewer.number()],
                );
                let any = module.name();
                module.add(
                    opcodes::INTEGER_NOT_EQUAL,
                    &[
                        named.boolean.number(),
                        any.number(),
                        fewer.number(),
                        named
                            .whole_numbers
                            .first()
                            .ok_or(Refused::NotAShader)?
                            .number(),
                    ],
                );
                let back = named
                    .which_stretch
                    .get(top)
                    .ok_or(Refused::NotAShader)?
                    .number();
                let chosen = module.name();
                module.add(
                    opcodes::SELECT,
                    &[
                        named.whole.number(),
                        chosen.number(),
                        any.number(),
                        back,
                        onward,
                    ],
                );
                module.add(
                    opcodes::STORE,
                    &[named.where_it_is_up_to.number(), chosen.number()],
                );
                module.add(opcodes::BRANCH, &[after.number()]);
            }
            WhereNext::Goes { to } => {
                let there = named
                    .which_stretch
                    .get(to)
                    .ok_or(Refused::NotAShader)?
                    .number();
                module.add(opcodes::STORE, &[named.where_it_is_up_to.number(), there]);
                module.add(opcodes::BRANCH, &[after.number()]);
            }
            WhereNext::Jumps { to, when, asking } => {
                let asked = match asking {
                    Asking::Predicate => carrying.predicate,
                    Asking::Truth(which) => one_of_the_truths(module, named, which),
                };
                let holds = if when {
                    asked
                } else {
                    let turned = module.name();
                    module.add(
                        opcodes::LOGICAL_NOT,
                        &[named.boolean.number(), turned.number(), asked],
                    );
                    turned.number()
                };
                let there = named
                    .which_stretch
                    .get(to)
                    .ok_or(Refused::NotAShader)?
                    .number();
                let chosen = module.name();
                module.add(
                    opcodes::SELECT,
                    &[named.whole.number(), chosen.number(), holds, there, onward],
                );
                module.add(
                    opcodes::STORE,
                    &[named.where_it_is_up_to.number(), chosen.number()],
                );
                module.add(opcodes::BRANCH, &[after.number()]);
            }
        }
    }

    /* A number naming no stretch is a shader that has run off its own end,
     * which is where it stops. */
    module.add(opcodes::LABEL, &[nowhere.number()]);
    module.add(
        opcodes::STORE,
        &[named.where_it_is_up_to.number(), finished],
    );
    module.add(opcodes::BRANCH, &[after.number()]);

    /* Where every stretch comes back together: on again where there is another
     * to run, and out of the loop where the shader is done. */
    module.add(opcodes::LABEL, &[after.number()]);
    let where_now = module.name();
    module.add(
        opcodes::LOAD,
        &[
            named.whole.number(),
            where_now.number(),
            named.where_it_is_up_to.number(),
        ],
    );
    let over = module.name();
    module.add(
        opcodes::INTEGER_EQUAL,
        &[
            named.boolean.number(),
            over.number(),
            where_now.number(),
            finished,
        ],
    );
    module.add(
        opcodes::BRANCH_CONDITIONAL,
        &[over.number(), done.number(), onwards.number()],
    );

    module.add(opcodes::LABEL, &[onwards.number()]);
    module.add(opcodes::BRANCH, &[header.number()]);

    module.add(opcodes::LABEL, &[done.number()]);
    let exported = module.name();
    module.add(
        opcodes::LOAD,
        &[
            named.four.number(),
            exported.number(),
            named.exported_kept.number(),
        ],
    );
    Ok(Some(exported.number()))
}

/// Turns a shader a title handed over into one a host graphics driver takes.
///
/// Reports [`Refused`] where the words hold something this has no reading of,
/// rather than putting out a module that runs and means something else.
///
/// # Errors
///
/// [`Refused::NotAShader`] when the words are not a shader at all, and the
/// variant naming the instruction or the feature this has no reading of when
/// they are one.
pub fn translate(kind: Kind, words: &[u32]) -> Result<Translated, Refused> {
    /* A shader that runs straight through is written out as it always was. One
     * that jumps is read a second way, because its stretches are not one run. */
    if let Some(running) = stretches(words) {
        let Fetches {
            wanted,
            streams,
            samples,
            pictures,
        } = what_the_fetches_need(words, &running)?;

        let mut module = Module::new();
        let named = preamble(&mut module, kind, &wanted, samples, 0);

        let mut carrying = Carrying {
            previous: Some(extract(&mut module, &named, named.nothing.number(), 0)),
            predicate: named.never.number(),
            exported: None,
            carried_over: None,
        };

        for step in running {
            one_instruction(&mut module, &named, words, step, &mut carrying)?;
        }

        what_it_puts_out(&mut module, &named, carrying.exported);
        return Ok(Translated {
            words: module.finish(),
            streams,
            pictures,
        });
    }

    let all = stretches_that_jump(words).ok_or(Refused::NotAShader)?;
    let flattened: Vec<Step> = all
        .iter()
        .flat_map(|one| one.steps.iter().copied())
        .collect();
    let Fetches {
        wanted,
        streams,
        samples,
        pictures,
    } = what_the_fetches_need(words, &flattened)?;

    let mut module = Module::new();
    let named = preamble(&mut module, kind, &wanted, samples, all.len());
    let exported = a_shader_that_jumps(&mut module, &named, words, &all)?;
    what_it_puts_out(&mut module, &named, exported);

    Ok(Translated {
        words: module.finish(),
        streams,
        pictures,
    })
}
