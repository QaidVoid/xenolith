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

use crate::layout::{Asking, Step, Stretch, WhereNext, stretches, stretches_that_jump};
use crate::opcodes;
use crate::preamble::{
    ALL_THE_CONSTANTS, CONSTANTS, Named, POSITION, THE_COUNTS, THE_TRUTHS, preamble,
};

use crate::{
    ALU_WORDS, Alu, Fetch, FetchKind, Format, Kind, ScalarOperation, VectorOperation, spirv::Module,
};

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

/// The operations that read two sources and produce one value, or nothing where
/// this is one of the others.
///
/// Splitting these out keeps the ones needing a third source, a comparison, or
/// blocks of their own apart from the ones that do not.
fn plainly(
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
fn dot_two_add(
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
fn dot_three(
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
fn throw_the_pixel_away(
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
fn arithmetic(
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

/// Whether one lane of a predicate push holds: the left is nothing there and
/// the right compares against nothing the way `comparing` says.
fn push_holds(
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

/// Reads one word out of the window on a title's memory.
fn one_word(module: &mut Module, named: &Named, at: u32) -> u32 {
    let where_from = module.name();
    module.add(
        opcodes::ACCESS_CHAIN,
        &[
            named.to_memory.number(),
            where_from.number(),
            named.memory.number(),
            named.zero_index.number(),
            at,
        ],
    );

    let value = module.name();
    module.add(
        opcodes::LOAD,
        &[named.whole.number(), value.number(), where_from.number()],
    );
    value.number()
}

/// Puts the four numbers of a register together the way a fetch says to.
///
/// The fetch names where each of them comes from in three bits for each in
/// turn. This is not a detail: a fetch of two numbers that says the third is one
/// and the fourth is to be left alone is how a vertex with two numbers in memory
/// arrives as a position, and putting nothing in those two instead gives every
/// vertex a fourth number of nothing, which is a position no hardware draws.
fn the_way_a_fetch_says(
    module: &mut Module,
    named: &Named,
    fetch: Fetch,
    fetched: &[Option<u32>],
) -> Vec<u32> {
    /* Where each of the four numbers in the register comes from, which the
     * fetch says in three bits for each of them in turn.
     *
     * This is not a detail. A fetch of two numbers that says the third is one
     * and the fourth is to be left alone is how a vertex with two numbers in
     * memory arrives as a position, and putting nothing in those two instead
     * gives every vertex a fourth number of nothing, which is a position no
     * hardware draws. */
    let already = read_whole(module, named, fetch.destination(), true);
    let swizzle = fetch.destination_swizzle();
    let mut parts = Vec::new();
    for step in 0..4u32 {
        let says = u32::from((swizzle >> (step * 3)) & 7);
        let part = module.name();
        match says {
            0..=3 => match fetched.get(says as usize).copied().flatten() {
                // Already a number of its own, so it is used as it is.
                Some(one) => {
                    parts.push(one);
                    continue;
                }
                /* A number the format does not reach. A fetch of two says
                 * nothing about the other two, and reading them out of memory
                 * it never named would be reading whatever sat next to it. */
                None => module.add(
                    opcodes::COMPOSITE_EXTRACT,
                    &[
                        named.float.number(),
                        part.number(),
                        named.nothing.number(),
                        step,
                    ],
                ),
            },
            5 => module.add(
                opcodes::COMPOSITE_EXTRACT,
                &[
                    named.float.number(),
                    part.number(),
                    named.all_ones.number(),
                    step,
                ],
            ),
            7 => module.add(
                opcodes::COMPOSITE_EXTRACT,
                &[named.float.number(), part.number(), already, step],
            ),
            _ => module.add(
                opcodes::COMPOSITE_EXTRACT,
                &[
                    named.float.number(),
                    part.number(),
                    named.nothing.number(),
                    step,
                ],
            ),
        }
        parts.push(part.number());
    }

    parts
}

/// Returns the numbers a fetch brings back, out of the words it read.
///
/// A format may give each number a word to itself, in which case the word is
/// the number and only its bits have to be read as one, or it may pack several
/// into a word, in which case each is taken apart.
fn the_numbers_in_the_words(
    module: &mut Module,
    named: &Named,
    fetch: Fetch,
    format: Format,
    words: &[u32],
) -> Result<Vec<Option<u32>>, Refused> {
    let how_many = format.count().ok_or(Refused::Fetch)?;
    let mut fetched = Vec::new();
    /* Four sixteen bit floats come two to a word, and a host turns a word back
     * into two of them in one step, so each word is unpacked to a pair and the
     * four numbers are the two pairs' halves. */
    if matches!(format, Format::FourHalfFloats) {
        for word in 0..2u32 {
            let from = *words.get(word as usize).ok_or(Refused::Fetch)?;
            let pair = module.name();
            module.add(
                opcodes::EXT_INST,
                &[
                    named.two.number(),
                    pair.number(),
                    named.extended.number(),
                    opcodes::GLSL_UNPACK_HALF2X16,
                    from,
                ],
            );
            for lane in 0..2u32 {
                let one = module.name();
                module.add(
                    opcodes::COMPOSITE_EXTRACT,
                    &[named.float.number(), one.number(), pair.number(), lane],
                );
                fetched.push(Some(one.number()));
            }
        }
        return Ok(fetched);
    }
    if let Some(packed) = format.packed() {
        for step in 0..4usize {
            match packed.get(step) {
                Some(&(width, offset, word)) => {
                    let from = *words.get(word as usize).ok_or(Refused::Fetch)?;
                    fetched.push(Some(taken_apart(
                        module, named, fetch, from, width, offset,
                    )?));
                }
                None => fetched.push(None),
            }
        }
    } else {
        for step in 0..4u32 {
            if step >= how_many {
                fetched.push(None);
                continue;
            }
            let word = *words.get(step as usize).ok_or(Refused::Fetch)?;
            let as_a_number = module.name();
            module.add(
                opcodes::BITCAST,
                &[named.float.number(), as_a_number.number(), word],
            );
            fetched.push(Some(as_a_number.number()));
        }
    }

    Ok(fetched)
}

/// Returns one number taken out of a word that holds several.
///
/// How wide it is and how far up the word it sits come from the format. Whether
/// it counts either way from nothing, and whether it is brought into the range a
/// fraction runs over, come from the fetch. What is divided by to bring it there
/// is the largest number that many bits hold, which for one counting either way
/// is one bit fewer.
fn taken_apart(
    module: &mut Module,
    named: &Named,
    fetch: Fetch,
    from: u32,
    width: u32,
    offset: u32,
) -> Result<u32, Refused> {
    let number = |which: u32| -> Result<u32, Refused> {
        named
            .numbers
            .get(&which)
            .map(|one| one.number())
            .ok_or(Refused::Fetch)
    };

    let bits = module.name();
    module.add(
        if fetch.is_signed() {
            opcodes::BIT_FIELD_SIGNED_EXTRACT
        } else {
            opcodes::BIT_FIELD_UNSIGNED_EXTRACT
        },
        &[
            named.whole.number(),
            bits.number(),
            from,
            number(offset)?,
            number(width)?,
        ],
    );

    let value = module.name();
    module.add(
        if fetch.is_signed() {
            opcodes::CONVERT_SIGNED_TO_FLOAT
        } else {
            opcodes::CONVERT_UNSIGNED_TO_FLOAT
        },
        &[named.float.number(), value.number(), bits.number()],
    );

    if !fetch.is_a_fraction() {
        return Ok(value.number());
    }

    /* What it is divided by, made from a whole number rather than named as a
     * fraction, because a number cannot be made inside the function that uses
     * it and a fraction of every size a format takes is not already there. */
    let whole = if fetch.is_signed() && !fetch.has_no_zero() {
        number((1u32 << (width - 1)) - 1)?
    } else {
        number((1u32 << width) - 1)?
    };
    let over = module.name();
    module.add(
        opcodes::CONVERT_UNSIGNED_TO_FLOAT,
        &[named.float.number(), over.number(), whole],
    );

    if !(fetch.is_signed() && fetch.has_no_zero()) {
        let made = module.name();
        module.add(
            opcodes::FLOAT_DIVIDE,
            &[
                named.float.number(),
                made.number(),
                value.number(),
                over.number(),
            ],
        );
        return Ok(made.number());
    }

    /* One that reaches both ends evenly is moved half a step first, so that
     * nothing is not one of the values it takes, and divided by half as much. */
    let two = module.name();
    module.add(
        opcodes::CONVERT_UNSIGNED_TO_FLOAT,
        &[named.float.number(), two.number(), number(2)?],
    );
    let one = extract(module, named, named.all_ones.number(), 0);
    let half = module.name();
    module.add(
        opcodes::FLOAT_DIVIDE,
        &[named.float.number(), half.number(), one, two.number()],
    );
    let moved = module.name();
    module.add(
        opcodes::FLOAT_ADD,
        &[
            named.float.number(),
            moved.number(),
            value.number(),
            half.number(),
        ],
    );
    let halved = module.name();
    module.add(
        opcodes::FLOAT_DIVIDE,
        &[
            named.float.number(),
            halved.number(),
            over.number(),
            two.number(),
        ],
    );

    let made = module.name();
    module.add(
        opcodes::FLOAT_DIVIDE,
        &[
            named.float.number(),
            made.number(),
            moved.number(),
            halved.number(),
        ],
    );
    Ok(made.number())
}

/// Goes and gets a vertex, putting it in the register the fetch names.
///
/// Where it sits is worked out the way the hardware works it out: the number the
/// draw handed the shader, times how far apart two vertices are, plus how far
/// into one this part sits, from wherever the title said that fetch reads from.
/// That last is the only piece this cannot get from the instruction, so it is
/// read out of the first words of the window, where the runtime put it.
///
/// # Errors
///
/// Reports [`Refused`] where the shader names a number this has no reading of.
fn go_and_get(
    module: &mut Module,
    named: &Named,
    fetch: Fetch,
    slot: u32,
    stride: u32,
    format: Format,
) -> Result<(), Refused> {
    let number = |which: u32| -> Result<u32, Refused> {
        named
            .numbers
            .get(&which)
            .map(|one| one.number())
            .ok_or(Refused::Fetch)
    };

    // Which vertex this is, which a draw hands over in a register.
    let source = read_whole(module, named, fetch.source(), true);
    let picked = module.name();
    module.add(
        opcodes::COMPOSITE_EXTRACT,
        &[
            named.float.number(),
            picked.number(),
            source,
            u32::from(fetch.source_component()),
        ],
    );
    let which_vertex = module.name();
    module.add(
        opcodes::CONVERT_FLOAT_TO_UNSIGNED,
        &[named.whole.number(), which_vertex.number(), picked.number()],
    );

    let stepped = module.name();
    module.add(
        opcodes::INTEGER_MULTIPLY,
        &[
            named.whole.number(),
            stepped.number(),
            which_vertex.number(),
            number(stride)?,
        ],
    );
    let from = one_word(module, named, number(slot)?);
    let began = module.name();
    module.add(
        opcodes::INTEGER_ADD,
        &[named.whole.number(), began.number(), from, stepped.number()],
    );
    let at = module.name();
    module.add(
        opcodes::INTEGER_ADD,
        &[
            named.whole.number(),
            at.number(),
            began.number(),
            number(fetch.offset())?,
        ],
    );

    // The words this reads, which a packed format shares between its numbers.
    let mut words = Vec::new();
    for step in 0..format.words().ok_or(Refused::Fetch)? {
        let here = module.name();
        module.add(
            opcodes::INTEGER_ADD,
            &[
                named.whole.number(),
                here.number(),
                at.number(),
                number(step)?,
            ],
        );
        words.push(one_word(module, named, here.number()));
    }

    let fetched = the_numbers_in_the_words(module, named, fetch, format, &words)?;

    let parts = the_way_a_fetch_says(module, named, fetch, &fetched);

    let together = module.name();
    let mut making = vec![named.four.number(), together.number()];
    making.extend(parts);
    module.add(opcodes::COMPOSITE_CONSTRUCT, &making);

    let index = named
        .indices
        .get(usize::from(fetch.destination() % 64))
        .ok_or(Refused::Fetch)?;
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
    module.add(opcodes::STORE, &[where_to.number(), together.number()]);

    Ok(())
}

/// Every number the fetches in a shader work their addresses out of.
///
/// Reads a picture at the place a register names.
///
/// Where in the picture to read is a pair of numbers running from nothing to one
/// across it, which is how the hardware counts and how a host counts too, so
/// they are handed over as they were found. How far down the sizes of the
/// picture to read is not said by anything: it follows from how fast the place
/// being read moves from one part of the screen to the next, and the hardware a
/// host has works that out itself.
///
/// # Errors
///
/// Reports [`Refused`] where the shader names a picture or a register this has
/// no reading of.
#[allow(clippy::too_many_arguments)]
fn look_at_a_picture(module: &mut Module, named: &Named, fetch: Fetch) -> Result<(), Refused> {
    let (looked_at, to_one, read_one) = named.looked_at.ok_or(Refused::Fetch)?;

    let where_in_it = read_whole(module, named, fetch.source(), true);
    let across = module.name();
    module.add(
        opcodes::VECTOR_SHUFFLE,
        &[
            named.two.number(),
            across.number(),
            where_in_it,
            where_in_it,
            0,
            1,
        ],
    );

    let which = named
        .numbers
        .get(&u32::from(fetch.constant()))
        .ok_or(Refused::Fetch)?;
    let at = module.name();
    module.add(
        opcodes::ACCESS_CHAIN,
        &[
            to_one.number(),
            at.number(),
            looked_at.number(),
            which.number(),
        ],
    );
    let reading = module.name();
    module.add(
        opcodes::LOAD,
        &[read_one.number(), reading.number(), at.number()],
    );

    let got = module.name();
    if matches!(named.kind, Kind::Vertex) {
        /* Reading the whole picture rather than a smaller copy of it, said
         * outright because nothing here can be worked out from the screen. */
        module.add(
            opcodes::IMAGE_SAMPLE_EXPLICIT_LOD,
            &[
                named.four.number(),
                got.number(),
                reading.number(),
                across.number(),
                2,
                named.no_number.number(),
            ],
        );
    } else {
        module.add(
            opcodes::IMAGE_SAMPLE_IMPLICIT_LOD,
            &[
                named.four.number(),
                got.number(),
                reading.number(),
                across.number(),
            ],
        );
    }

    /* Which of the four numbers read the fetch puts where, and which it leaves
     * alone. A shader that reads three pictures into three lanes of one
     * register says so here and nowhere else, so writing the whole of what was
     * read leaves only the last of the three: the other two are read and thrown
     * away, and what the shader goes on to work out is two thirds wrong. */
    let already = read_whole(module, named, fetch.destination(), true);
    let swizzle = fetch.destination_swizzle();
    let mut parts = Vec::new();
    for step in 0..4u32 {
        let says = u32::from((swizzle >> (step * 3)) & 7);
        let part = module.name();
        let (from, lane) = match says {
            0..=3 => (got.number(), says),
            5 => (named.all_ones.number(), step),
            7 => (already, step),
            _ => (named.nothing.number(), step),
        };
        module.add(
            opcodes::COMPOSITE_EXTRACT,
            &[named.float.number(), part.number(), from, lane],
        );
        parts.push(part.number());
    }

    let put = module.name();
    let mut making = vec![named.four.number(), put.number()];
    making.extend(parts);
    module.add(opcodes::COMPOSITE_CONSTRUCT, &making);

    let into = named
        .indices
        .get(usize::from(fetch.destination() % 64))
        .ok_or(Refused::Fetch)?;
    let where_to = module.name();
    module.add(
        opcodes::ACCESS_CHAIN,
        &[
            named.to_one.number(),
            where_to.number(),
            named.registers.number(),
            into.number(),
        ],
    );
    module.add(opcodes::STORE, &[where_to.number(), put.number()]);

    Ok(())
}

/// Writes whichever of the two kinds of going and getting an instruction is.
///
/// A fetch either reads a picture through the hardware that knows how one is
/// laid out, or works out an address and reads a vertex from it. Which of the
/// two it is the instruction says, and nothing else about it is shared.
///
/// # Errors
///
/// Reports [`Refused`] where the instruction is one this has no reading of.
fn go_and_fetch(
    module: &mut Module,
    named: &Named,
    words: &[u32],
    which: usize,
    carried_over: &mut Option<(u32, u32)>,
) -> Result<(), Refused> {
    let Some(fetch) = Fetch::at(
        words,
        which.checked_mul(ALU_WORDS).ok_or(Refused::NotAShader)?,
    ) else {
        return Err(Refused::NotAShader);
    };

    if fetch.kind() == FetchKind::Texture {
        return look_at_a_picture(module, named, fetch);
    }

    let (slot, stride) = what_it_reads_from(fetch, carried_over)?;
    go_and_get(module, named, fetch, slot, stride, fetch.format())
}

/// Returns which fetch constant says where a stream is and how far apart two
/// vertices sit in it, following what came before where a fetch says to.
///
/// A fetch that leaves those to the one before it takes them from **the last
/// full fetch**, whichever constant that named, rather than from the last one
/// on the constant this names: the console works its address out once on a full
/// fetch and every fetch after it reads from there. Its own constant is not
/// used at all. What it does say for itself is how far into a vertex it reads
/// and how what it finds there is stored.
fn what_it_reads_from(
    fetch: Fetch,
    carried_over: &mut Option<(u32, u32)>,
) -> Result<(u32, u32), Refused> {
    if fetch.follows_another() {
        return carried_over.ok_or(Refused::Fetch);
    }

    let both = (fetch.where_in_the_constants(), fetch.stride());
    *carried_over = Some(both);
    Ok(both)
}

/// Everything the fetches in a shader need said before its first instruction.
struct Fetches {
    /// Every number one of them works an address or an index out of.
    wanted: Vec<u32>,
    /// Where the shader goes for each of its vertex streams.
    streams: Vec<Stream>,
    /// Whether any of them reads a texture, which needs saying because a shader
    /// that reads none must not name a texture it never looks at.
    samples: bool,
    /// Which places it reads one from, so that a host need only make ready the
    /// pictures a shader will actually look at.
    pictures: Vec<u32>,
}

/// Reports [`Refused`] where a fetch is one this has no reading of.
fn what_the_fetches_need(words: &[u32], running: &[Step]) -> Result<Fetches, Refused> {
    /* What every fetch works its address out of, gathered before anything is
     * written, because a number cannot be named inside the function that uses
     * it and these are not small enough to be among the ones already there. */
    let mut wanted = Vec::new();
    let mut streams = Vec::new();
    let mut samples = false;
    let mut pictures = Vec::new();
    let mut carried_over: Option<(u32, u32)> = None;
    for step in running {
        if !step.fetches {
            continue;
        }
        let one = step.at;
        let Some(fetch) = Fetch::at(
            words,
            one.checked_mul(ALU_WORDS).ok_or(Refused::NotAShader)?,
        ) else {
            return Err(Refused::NotAShader);
        };
        /* A texture is read rather than worked out an address for. Which one is
         * named by the fetch itself, so all that is needed ahead of time is the
         * number to reach it by.
         *
         * A vertex shader may read one as readily as a pixel shader, and does
         * where a title works out where a vertex goes from a picture. What it
         * cannot do is let the hardware choose how far down the picture to
         * read, since that is worked out from how fast the place being read
         * moves across the screen and nothing moves across a screen before
         * there is one; so it says which to read instead, further down. */
        if fetch.kind() == FetchKind::Texture {
            samples = true;
            let which = u32::from(fetch.constant());
            if !pictures.contains(&which) {
                pictures.push(which);
            }
            wanted.push(which);
            continue;
        }

        if fetch.kind() != FetchKind::Vertex {
            return Err(Refused::Fetch);
        }

        let (slot, stride) = what_it_reads_from(fetch, &mut carried_over)?;
        let format = fetch.format();
        let Some(count) = format.count() else {
            return Err(Refused::Fetch);
        };

        wanted.push(slot);
        wanted.push(stride);
        wanted.push(fetch.offset());
        for step in 0..4 {
            wanted.push(step);
        }

        /* What taking a packed number apart names: how wide each is, how far up
         * the word it sits, and what it is divided by to bring it into the
         * range a fraction runs over. A number cannot be made inside the
         * function that uses it, so every one is asked for here. */
        if let Some(packed) = format.packed() {
            wanted.push(2);
            for (width, offset, _) in packed {
                wanted.push(width);
                wanted.push(offset);
                wanted.push((1u32 << width) - 1);
                wanted.push((1u32 << (width - 1)) - 1);
            }
        }
        streams.push(Stream {
            slot,
            stride,
            offset: fetch.offset(),
            words: count,
        });
    }

    Ok(Fetches {
        wanted,
        streams,
        samples,
        pictures,
    })
}

/// Returns a value whose lanes come from one of two vectors, lane by lane.
///
/// Which is what the console's write masks come to, and a host says it in one
/// instruction rather than four. `taking` holds a bit for each lane that comes
/// from the first, and the rest come from the second.
fn lane_by_lane(module: &mut Module, named: &Named, first: u32, second: u32, taking: u8) -> u32 {
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
fn what_is_exported(
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
fn extract(module: &mut Module, named: &Named, from: u32, lane: u32) -> u32 {
    let one = module.name();
    module.add(
        opcodes::COMPOSITE_EXTRACT,
        &[named.float.number(), one.number(), from, lane],
    );
    one.number()
}

/// One number worked out from two, as the operation a host names for it.
fn worked_out(module: &mut Module, named: &Named, operation: u16, left: u32, right: u32) -> u32 {
    let made = module.name();
    module.add(
        operation,
        &[named.float.number(), made.number(), left, right],
    );
    made.number()
}

/// Whether something holds of two numbers.
fn holds_of(module: &mut Module, named: &Named, operation: u16, left: u32, right: u32) -> u32 {
    let made = module.name();
    module.add(
        operation,
        &[named.boolean.number(), made.number(), left, right],
    );
    made.number()
}

/// One number or the other, according to whether something holds.
fn one_or_the_other(module: &mut Module, named: &Named, holds: u32, yes: u32, no: u32) -> u32 {
    let made = module.name();
    module.add(
        opcodes::SELECT,
        &[named.float.number(), made.number(), holds, yes, no],
    );
    made.number()
}

/// The same number the other way about.
fn the_other_way(module: &mut Module, named: &Named, value: u32) -> u32 {
    let made = module.name();
    module.add(
        opcodes::FLOAT_NEGATE,
        &[named.float.number(), made.number(), value],
    );
    made.number()
}

/// How far a number is from nothing, whichever way it lies.
fn how_far_from_nothing(module: &mut Module, named: &Named, value: u32) -> u32 {
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
fn a_cube_is_read(module: &mut Module, named: &Named, from: u32, produced: u32) {
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

/// Returns one of two numbers, according to whether something holds.
fn chosen(module: &mut Module, named: &Named, holds: u32, when: f32, otherwise: f32) -> u32 {
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

/// Reads one of the constants a shader was handed, giving the name of what was
/// read.
///
/// The number naming it is written out here rather than taken from the ones
/// prepared for registers, because a constant can be named past where those
/// stop.
fn read_a_constant(module: &mut Module, named: &Named, which: u32) -> u32 {
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
fn moved_about(module: &mut Module, named: &Named, value: u32, along: u8, negated: bool) -> u32 {
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
fn read(
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
fn read_whole(module: &mut Module, named: &Named, which: u8, from_register: bool) -> u32 {
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
