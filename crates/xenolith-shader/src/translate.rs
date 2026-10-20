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

use crate::arithmetic::{arithmetic, extract, read, read_a_constant};
use crate::export::what_it_puts_out;
use crate::fetching::{Fetches, go_and_fetch, what_the_fetches_need};
use crate::layout::{Asking, Step, Stretch, WhereNext, stretches, stretches_that_jump};
use crate::opcodes;
use crate::preamble::{Named, THE_COUNTS, THE_TRUTHS, preamble};
use crate::single_lane::{put_it_where_it_goes, single_lane, whether_it_runs};

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
