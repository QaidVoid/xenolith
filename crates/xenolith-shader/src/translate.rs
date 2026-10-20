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

use std::collections::BTreeMap;

use crate::opcodes;

use crate::{
    ALU_WORDS, Alu, Fetch, FetchKind, Flow, FlowOperation, Format, Kind, ScalarOperation,
    VectorOperation, spirv::Module,
};

/// Which export a vertex shader puts the position of the vertex in.
///
/// The console's own number for it, and what this title's shaders use: they
/// export to nought and one, which are carried on to whatever runs next, and to
/// this, which is where the vertex went.
const POSITION: u8 = 62;

/// How many things a shader carries on to whatever runs after it.
///
/// Sixteen, which is how many the console has. Fewer was tried, on the grounds
/// that the shaders looked at named only the first four, and a title turned out
/// to name up to the seventh: a shader naming one past the end is refused
/// whole, so whatever it drew is not drawn at all. The number the hardware has
/// is the only one that cannot be wrong.
const CARRIED: u32 = 16;

/// How many more than the constants a shader names are handed to it.
///
/// Two: what the hardware would have done to a position after the shader put it
/// out, as something to multiply by and something to add. On the console that is
/// a set of registers rather than a constant, and whether it is applied at all
/// is a register too, so a host cannot be told once and for all. Handing it over
/// beside the constants is how the same shader serves both a title that leaves
/// the hardware to do it and one that has already done it itself.
const BESIDE_THE_CONSTANTS: u32 = 12;

/// Where among the constants the truths a shader asks after when it jumps are
/// kept: after the scale and the offset, two lots of four words holding the two
/// hundred and fifty six of them a bit apiece.
const THE_TRUTHS: u32 = ALL_THE_CONSTANTS + 2;

/// Where among the constants the counts sit: eight lots of four words holding
/// the thirty two a shader reads to know how many times to go round a loop.
const THE_COUNTS: u32 = ALL_THE_CONSTANTS + 4;

/// How many constants the hardware holds altogether.
///
/// Twice what one shader names, because the two kinds of shader do not share
/// them: what a vertex shader calls its first constant is the file's first, and
/// what a pixel shader calls its first is the file's two hundred and fifty
/// seventh. The file runs to where the constants a fetch reads begin, which is
/// what says there are this many and not some other number.
const ALL_THE_CONSTANTS: u32 = 512;

/// How many constants a shader is handed.
///
/// The console's own number for the float constants a shader can name, and more
/// than the highest one this title's shaders reach.
const CONSTANTS: u32 = 256;

/// How many registers a shader is given.
///
/// The console's own number, and what the six bits naming a destination can
/// reach.
const REGISTERS: u32 = 64;

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

/// One instruction the control flow says to run.
#[derive(Debug, Clone, Copy)]
struct Step {
    /// Which instruction it is.
    at: usize,
    /// Whether the control flow marked it a fetch rather than arithmetic.
    fetches: bool,
    /// Whether the stretch it belongs to runs only where the predicate holds,
    /// and which way round.
    under: Option<bool>,
}

/// The instructions a shader's control flow says to run, in order.
///
/// A stretch may be one that runs only where the predicate holds, which is
/// carried on each instruction rather than turned into a branch: the module
/// this writes is one straight run, so what a predicated instruction writes is
/// chosen between its answer and what was already there.
fn stretches(words: &[u32]) -> Option<Vec<Step>> {
    let mut running = Vec::new();
    let mut at = 0;
    let mut ended = false;

    while let Some((low, high)) = Flow::pair(words, at) {
        for flow in [low, high] {
            if ended {
                continue;
            }
            let under = match flow.operation() {
                FlowOperation::Run | FlowOperation::RunAndEnd => None,
                FlowOperation::RunIfPredicated
                | FlowOperation::RunIfPredicatedAndEnd
                | FlowOperation::RunIfPredicatedClean
                | FlowOperation::RunIfPredicatedCleanAndEnd => Some(flow.predicate_holds()),
                FlowOperation::Nothing | FlowOperation::Reserve => continue,
                _ => return None,
            };

            let first = usize::from(flow.address());
            for step in 0..usize::from(flow.count()) {
                running.push(Step {
                    at: first.checked_add(step)?,
                    fetches: flow.fetches_at(step),
                    under,
                });
            }
            if flow.ends_the_shader() {
                ended = true;
            }
        }
        at += ALU_WORDS;
        if ended {
            return Some(running);
        }
    }

    None
}

/// Where a stretch goes once it has run.
#[derive(Debug, Clone, Copy)]
enum WhereNext {
    /// On to the stretch after it.
    Along,
    /// Nowhere: the shader is done.
    Ends,
    /// Straight to another stretch, asking after nothing.
    Goes { to: usize },
    /// Begins a loop: how many times round is read, and where to go when it is
    /// no times at all.
    Rounds { count: u8, past: usize },
    /// Ends a loop: one off how many are left, and back to the top while any
    /// remain.
    Again { top: usize },
    /// To another stretch, where what it asks after reads the way it says.
    Jumps {
        to: usize,
        when: bool,
        asking: Asking,
    },
}

/// What a jump asks after to decide whether it goes.
#[derive(Debug, Clone, Copy)]
enum Asking {
    /// The predicate, as the stretches that run on it do.
    Predicate,
    /// One of the truths a title handed over, named by where it sits.
    Truth(u8),
}

/// One stretch of a shader that jumps: what it runs and where it goes after.
struct Stretch {
    steps: Vec<Step>,
    next: WhereNext,
}

/// Reads the control flow as stretches that each say where they go, for a
/// shader whose stretches are not simply run one after another.
///
/// A shader that jumps cannot be laid out as one run of instructions, because
/// which of them run and in what order is only settled while it runs. So each
/// control flow instruction becomes a stretch of its own, kept at the place the
/// jumps name it by, and what it does when it has run is written down beside
/// it.
///
/// Nothing is guessed at: an operation with no reading here stops the whole
/// reading, so a shader that loops or calls is refused rather than translated
/// into something that runs and means something else.
fn stretches_that_jump(words: &[u32]) -> Option<Vec<Stretch>> {
    let mut all: Vec<Stretch> = Vec::new();
    let mut at = 0;
    let mut ended = false;

    while let Some((low, high)) = Flow::pair(words, at) {
        for flow in [low, high] {
            if ended {
                break;
            }
            let operation = flow.operation();
            match operation {
                /* Nothing to run and nowhere of its own to go. Kept all the
                 * same, because the jumps name a stretch by where it sits. */
                FlowOperation::Nothing | FlowOperation::Reserve | FlowOperation::FetchDone => {
                    all.push(Stretch {
                        steps: Vec::new(),
                        next: WhereNext::Along,
                    });
                }
                FlowOperation::LoopBegins => {
                    all.push(Stretch {
                        steps: Vec::new(),
                        next: WhereNext::Rounds {
                            count: flow.loop_named(),
                            past: usize::from(flow.jump_target()),
                        },
                    });
                }
                FlowOperation::LoopEnds => {
                    all.push(Stretch {
                        steps: Vec::new(),
                        next: WhereNext::Again {
                            top: usize::from(flow.jump_target()),
                        },
                    });
                }
                FlowOperation::JumpIf => {
                    if flow.goes_regardless() {
                        all.push(Stretch {
                            steps: Vec::new(),
                            next: WhereNext::Goes {
                                to: usize::from(flow.jump_target()),
                            },
                        });
                        continue;
                    }
                    all.push(Stretch {
                        steps: Vec::new(),
                        next: WhereNext::Jumps {
                            to: usize::from(flow.jump_target()),
                            when: flow.predicate_holds(),
                            asking: if flow.is_predicated() {
                                Asking::Predicate
                            } else {
                                Asking::Truth(flow.bool_index())
                            },
                        },
                    });
                }
                FlowOperation::Run
                | FlowOperation::RunAndEnd
                | FlowOperation::RunIfPredicated
                | FlowOperation::RunIfPredicatedAndEnd
                | FlowOperation::RunIfPredicatedClean
                | FlowOperation::RunIfPredicatedCleanAndEnd => {
                    let under = match operation {
                        FlowOperation::RunIfPredicated
                        | FlowOperation::RunIfPredicatedAndEnd
                        | FlowOperation::RunIfPredicatedClean
                        | FlowOperation::RunIfPredicatedCleanAndEnd => Some(flow.predicate_holds()),
                        _ => None,
                    };
                    let first = usize::from(flow.address());
                    let mut steps = Vec::new();
                    for step in 0..usize::from(flow.count()) {
                        steps.push(Step {
                            at: first.checked_add(step)?,
                            fetches: flow.fetches_at(step),
                            under,
                        });
                    }
                    let next = if flow.ends_the_shader() {
                        ended = true;
                        WhereNext::Ends
                    } else {
                        WhereNext::Along
                    };
                    all.push(Stretch { steps, next });
                }
                _ => return None,
            }
        }
        at += ALU_WORDS;
        if ended {
            break;
        }
    }

    /* A run of stretches that never stops is not a shader: something would run
     * off the end of what was handed over. */
    if !ended {
        return None;
    }
    Some(all)
}

/// What a module needs named before anything is done in it.
struct Named {
    kind: Kind,
    extended: crate::spirv::Name,
    four: crate::spirv::Name,
    to_one: crate::spirv::Name,
    out: crate::spirv::Name,
    carried: Vec<crate::spirv::Name>,
    registers: crate::spirv::Name,
    boolean: crate::spirv::Name,
    float: crate::spirv::Name,
    two: crate::spirv::Name,
    three: crate::spirv::Name,
    constants: crate::spirv::Name,
    to_constant: crate::spirv::Name,
    zero_index: crate::spirv::Name,
    four_bools: crate::spirv::Name,
    nothing: crate::spirv::Name,
    all_ones: crate::spirv::Name,
    biggest: crate::spirv::Name,
    never: crate::spirv::Name,
    indices: Vec<crate::spirv::Name>,
    whole: crate::spirv::Name,
    memory: crate::spirv::Name,
    to_memory: crate::spirv::Name,
    numbers: std::collections::BTreeMap<u32, crate::spirv::Name>,
    /* Somewhere to keep what carries from one stretch of a shader to the next
     * where the stretches are not run one after another. A shader that jumps
     * runs its stretches in an order only known while it runs, so what one
     * leaves for another cannot be held in a name the way a straight run's is;
     * it is kept in memory a shader has to itself. Made for every shader
     * because a place to keep something has to be made where a function
     * begins, and left alone by the shaders that run straight through. */
    a_byte: crate::spirv::Name,
    times_round: crate::spirv::Name,
    no_number: crate::spirv::Name,
    whole_numbers: Vec<crate::spirv::Name>,
    which_stretch: Vec<crate::spirv::Name>,
    where_it_is_up_to: crate::spirv::Name,
    predicate_kept: crate::spirv::Name,
    previous_kept: crate::spirv::Name,
    exported_kept: crate::spirv::Name,
    /// The pictures a shader was handed, named only where it reads one.
    looked_at: Option<(crate::spirv::Name, crate::spirv::Name, crate::spirv::Name)>,
    /// The test a title sets on how solid a place has to be to be drawn at all,
    /// named for a pixel shader only: which test it is, and what it is against.
    cutoff: Option<(crate::spirv::Name, crate::spirv::Name)>,
}

/// The values a translated shader needs standing ready before its first
/// instruction: what a comparison produces, the two vectors every operation
/// reaches for, and the number each register and constant is reached by.
///
/// These are kept apart from the rest of the preamble because a constant
/// cannot be written inside the function that uses it, so all of them have to
/// exist before the body starts.
fn the_values(
    module: &mut Module,
    float: crate::spirv::Name,
    four: crate::spirv::Name,
    count: crate::spirv::Name,
) -> Standing {
    /* What a comparison produces and what it compares against. Choosing
     * between two things needs both, and a constant cannot be written inside
     * the function that uses it. */
    let boolean = module.name();
    module.add(opcodes::TYPE_BOOL, &[boolean.number()]);
    let four_bools = module.name();
    module.add(
        opcodes::TYPE_VECTOR,
        &[four_bools.number(), boolean.number(), 4],
    );
    let zero = module.name();
    module.add(opcodes::CONSTANT, &[float.number(), zero.number(), 0]);
    let nothing = module.name();
    module.add(
        opcodes::CONSTANT_COMPOSITE,
        &[
            four.number(),
            nothing.number(),
            zero.number(),
            zero.number(),
            zero.number(),
            zero.number(),
        ],
    );

    let one = module.name();
    module.add(
        opcodes::CONSTANT,
        &[float.number(), one.number(), 0x3f80_0000],
    );
    let all_ones = module.name();
    module.add(
        opcodes::CONSTANT_COMPOSITE,
        &[
            four.number(),
            all_ones.number(),
            one.number(),
            one.number(),
            one.number(),
            one.number(),
        ],
    );

    /* The largest number a float holds, which is what clearing the predicate
     * counter writes, and a predicate that does not hold, which is what one
     * starts as. */
    let biggest = module.name();
    module.add(
        opcodes::CONSTANT,
        &[float.number(), biggest.number(), 0x7f7f_ffff],
    );
    let never = module.name();
    module.add(opcodes::CONSTANT_FALSE, &[boolean.number(), never.number()]);

    /* Every number a register or a constant is reached by, named before the
     * function begins. There are more constants than registers, so the count is
     * the larger of the two.
     * A constant cannot sit inside one, which is the sort of thing a validator
     * says and nothing else does. */
    let mut indices = Vec::with_capacity((ALL_THE_CONSTANTS + BESIDE_THE_CONSTANTS) as usize);
    for which in 0..ALL_THE_CONSTANTS + BESIDE_THE_CONSTANTS {
        let index = module.name();
        module.add(opcodes::CONSTANT, &[count.number(), index.number(), which]);
        indices.push(index);
    }

    Standing {
        boolean,
        four_bools,
        nothing,
        all_ones,
        biggest,
        never,
        indices,
    }
}

/// What [`the_values`] put out.
struct Standing {
    boolean: crate::spirv::Name,
    four_bools: crate::spirv::Name,
    nothing: crate::spirv::Name,
    all_ones: crate::spirv::Name,
    biggest: crate::spirv::Name,
    never: crate::spirv::Name,
    indices: Vec<crate::spirv::Name>,
}

/// The places a shader puts what it produces, and the words naming where it
/// starts.
///
/// Where a shader starts is said after its outputs exist, because saying it has
/// to name every one of them, and a module keeps that with the first things it
/// says however late it was worked out.
fn the_outputs(
    module: &mut Module,
    kind: Kind,
    to_out: crate::spirv::Name,
    out: crate::spirv::Name,
    entry: crate::spirv::Name,
    model: u32,
    also: &[crate::spirv::Name],
) -> Vec<crate::spirv::Name> {
    /* What a shader carries on to whatever runs after it.
     *
     * A vertex shader puts out where the vertex went and some number of other
     * things beside it, which what runs next reads by the place they were put
     * in. A pixel shader has no such thing: its one output is the colour.
     *
     * Where a shader starts is said after these, because saying it has to name
     * every one of them, and a module keeps that with the first things it says
     * however late it was worked out. */
    /* What is carried from one to the other.
     *
     * A vertex shader puts these out and a pixel shader is handed them, one for
     * one and in the same places. On the console a pixel shader finds them in
     * its registers, already worked out for the pixel being drawn, which is why
     * a pixel shader that is handed nothing computes a colour out of nothing and
     * puts out black however right the rest of it is. */
    let mut carried = Vec::new();
    for which in 0..CARRIED {
        let one = module.name();
        let goes = if matches!(kind, Kind::Vertex) { 3 } else { 1 };
        module.add(opcodes::VARIABLE, &[to_out.number(), one.number(), goes]);
        module.annotate(71, &[one.number(), 30, which]);
        carried.push(one);
    }

    let mut talks_to = vec![out.number()];
    for one in &carried {
        talks_to.push(one.number());
    }
    for one in also {
        talks_to.push(one.number());
    }
    module.add_early_with_text(15, &[model, entry.number()], "main", &talks_to);
    /* How a shader is run, which comes after saying where it starts. */
    if matches!(kind, Kind::Pixel) {
        module.add_early(16, &[entry.number(), 7]);
    }

    carried
}

/// Puts in the registers what a shader finds there before its first
/// instruction: how big a point is, what was carried here, and which vertex
/// this is.
fn what_a_shader_starts_with(
    module: &mut Module,
    at_the_edges: &Edges,
    indices: &[crate::spirv::Name],
    carried: &[crate::spirv::Name],
) {
    let Edges {
        kind,
        float,
        four,
        to_one,
        registers,
        all_ones,
        how_big,
        which_vertex,
    } = *at_the_edges;

    /* One pixel across, said before anything else because a shader that never
     * says it is one a host will not draw points with at all. */
    if let Some(one) = how_big {
        let across = module.name();
        module.add(
            opcodes::COMPOSITE_EXTRACT,
            &[float.number(), across.number(), all_ones.number(), 0],
        );
        module.add(opcodes::STORE, &[one.number(), across.number()]);
    }

    /* What was carried here, put in the registers a pixel shader finds it in. */
    if matches!(kind, Kind::Pixel) {
        for (which, one) in carried.iter().enumerate() {
            let Some(index) = indices.get(which) else {
                break;
            };
            let read = module.name();
            module.add(opcodes::LOAD, &[four.number(), read.number(), one.number()]);
            let where_to = module.name();
            module.add(
                opcodes::ACCESS_CHAIN,
                &[
                    to_one.number(),
                    where_to.number(),
                    registers.number(),
                    index.number(),
                ],
            );
            module.add(opcodes::STORE, &[where_to.number(), read.number()]);
        }
    }

    /* The number of the vertex, put where the hardware puts it. */
    if let Some((one, counting)) = which_vertex {
        let read = module.name();
        module.add(
            opcodes::LOAD,
            &[counting.number(), read.number(), one.number()],
        );
        let as_a_number = module.name();
        module.add(
            opcodes::CONVERT_SIGNED_TO_FLOAT,
            &[float.number(), as_a_number.number(), read.number()],
        );
        let together = module.name();
        module.add(
            opcodes::COMPOSITE_CONSTRUCT,
            &[
                four.number(),
                together.number(),
                as_a_number.number(),
                as_a_number.number(),
                as_a_number.number(),
                as_a_number.number(),
            ],
        );
        let Some(first) = indices.first() else {
            return;
        };
        let where_to = module.name();
        module.add(
            opcodes::ACCESS_CHAIN,
            &[
                to_one.number(),
                where_to.number(),
                registers.number(),
                first.number(),
            ],
        );
        module.add(opcodes::STORE, &[where_to.number(), together.number()]);
    }
}

/// What a shader finds in its registers before its first instruction, and what
/// is needed to put it there.
#[derive(Clone, Copy)]
struct Edges {
    kind: Kind,
    float: crate::spirv::Name,
    four: crate::spirv::Name,
    to_one: crate::spirv::Name,
    registers: crate::spirv::Name,
    all_ones: crate::spirv::Name,
    how_big: Option<crate::spirv::Name>,
    which_vertex: Option<(crate::spirv::Name, crate::spirv::Name)>,
}

/// Makes the window a shader reads the vertices it goes and gets through, and
/// names every other number a fetch works an address out of.
///
/// Kept apart from the rest of the preamble because none of it is about what a
/// shader is, only about what it is handed.
fn what_a_fetch_reads(
    module: &mut Module,
    count: crate::spirv::Name,
    wanted: &[u32],
) -> (
    crate::spirv::Name,
    crate::spirv::Name,
    BTreeMap<u32, crate::spirv::Name>,
) {
    /* Where the vertices a shader goes and gets for itself are read from.
     *
     * A draw hands a shader a number and nothing else, and the shader works out
     * from it where its vertex sits and reads it. What it reads is memory the
     * title filled in, so this is a window on that memory rather than anything
     * this decides: the runtime puts what a fetch names into it and says where,
     * and the first words of it are those wheres. */
    let run_of_words = module.name();
    module.add(
        opcodes::TYPE_RUNTIME_ARRAY,
        &[run_of_words.number(), count.number()],
    );
    module.annotate(71, &[run_of_words.number(), 6, 4]);
    let holding = module.name();
    module.add(
        opcodes::TYPE_STRUCT,
        &[holding.number(), run_of_words.number()],
    );
    module.annotate(71, &[holding.number(), 2]);
    module.annotate(72, &[holding.number(), 0, 35, 0]);
    let to_holding = module.name();
    module.add(
        opcodes::TYPE_POINTER,
        &[to_holding.number(), 12, holding.number()],
    );
    let memory = module.name();
    module.add(
        opcodes::VARIABLE,
        &[to_holding.number(), memory.number(), 12],
    );
    module.annotate(71, &[memory.number(), 34, 0]);
    module.annotate(71, &[memory.number(), 33, 1]);
    /* Read and never written, which has to be said rather than left to be seen:
     * a device is allowed to refuse a shader that could write to one of these
     * from where a vertex is worked out, and saying so is what makes it not
     * one of those. */
    module.annotate(71, &[memory.number(), 24]);
    module.annotate(72, &[holding.number(), 0, 24]);
    let to_memory = module.name();
    module.add(
        opcodes::TYPE_POINTER,
        &[to_memory.number(), 12, count.number()],
    );

    /* Every other number a fetch works its address out of, named here because a
     * number cannot be written inside the function that uses it. */
    let mut numbers = std::collections::BTreeMap::new();
    for &which in wanted {
        if let std::collections::btree_map::Entry::Vacant(slot) = numbers.entry(which) {
            let named = module.name();
            module.add(opcodes::CONSTANT, &[count.number(), named.number(), which]);
            slot.insert(named);
        }
    }

    (memory, to_memory, numbers)
}

/// Says what a module can do, which kind of shader it is, where what it
/// produces goes, and the types every one of them works in.
fn what_the_module_is(
    module: &mut Module,
    kind: Kind,
    extended: crate::spirv::Name,
    out: crate::spirv::Name,
    types: &[crate::spirv::Name; 4],
) -> u32 {
    let [void, signature, float, four] = *types;

    module.add_early(17, &[1]);
    module.add_early_with_text(
        opcodes::EXT_INST_IMPORT,
        &[extended.number()],
        opcodes::GLSL_STANDARD,
        &[],
    );
    module.add_early(14, &[0, 1]);

    let model = match kind {
        Kind::Vertex => 0,
        Kind::Pixel => 4,
    };
    /* Where what a shader produces goes.
     *
     * A pixel shader puts out a colour, which is one of however many a target
     * has, so it is named by its place. A vertex shader puts out where the
     * vertex is, which is not one of anything: it is the one thing the hardware
     * after it looks for, so it is named as that rather than by a place. */
    match kind {
        Kind::Vertex => module.annotate(71, &[out.number(), 11, 0]),
        Kind::Pixel => module.annotate(71, &[out.number(), 30, 0]),
    }

    module.add(opcodes::TYPE_VOID, &[void.number()]);
    module.add(opcodes::TYPE_FUNCTION, &[signature.number(), void.number()]);
    module.add(opcodes::TYPE_FLOAT, &[float.number(), 32]);
    module.add(opcodes::TYPE_VECTOR, &[four.number(), float.number(), 4]);

    model
}

/// Makes where a shader keeps its registers, and the pointers what it puts out
/// and what it is handed are reached through.
fn where_a_shader_works(
    module: &mut Module,
    names: &[crate::spirv::Name; 7],
) -> (crate::spirv::Name, crate::spirv::Name) {
    let [count, four, float, bank, to_bank, to_out, out] = *names;
    let length = module.name();
    module.add(
        opcodes::CONSTANT,
        &[count.number(), length.number(), REGISTERS],
    );
    module.add(
        opcodes::TYPE_ARRAY,
        &[bank.number(), four.number(), length.number()],
    );
    module.add(opcodes::TYPE_POINTER, &[to_bank.number(), 7, bank.number()]);
    module.add(opcodes::TYPE_POINTER, &[to_out.number(), 3, four.number()]);
    let to_carried = module.name();
    module.add(
        opcodes::TYPE_POINTER,
        &[to_carried.number(), 1, four.number()],
    );
    let to_size = module.name();
    module.add(
        opcodes::TYPE_POINTER,
        &[to_size.number(), 3, float.number()],
    );
    module.add(opcodes::VARIABLE, &[to_out.number(), out.number(), 3]);

    /* Which vertex this is.
     *
     * A draw names no vertices: it says how many, and the hardware counts them
     * off and hands each shader its number in the first of its registers. That
     * number is what a shader works out from where its vertex sits, so without
     * it every vertex of every draw is the same one and nothing has any size.
     *
     * It is the last thing about a vertex shader that comes from outside it,
     * and it comes from the host's own counting rather than from anything a
     * title wrote. */

    (to_carried, to_size)
}

/// Makes what a shader says at its edges: how big a point is, which vertex this
/// is, and what it carries on to whatever runs after it.
fn what_a_shader_says(
    module: &mut Module,
    kind: Kind,
    model: u32,
    names: &[crate::spirv::Name; 5],
) -> (
    Vec<crate::spirv::Name>,
    Option<crate::spirv::Name>,
    Option<(crate::spirv::Name, crate::spirv::Name)>,
) {
    let [to_size, to_out, to_carried, out, entry] = *names;
    /* How big a point is, which a shader has to say whenever what is being
     * drawn is points.
     *
     * The console keeps that in a register rather than in the shader, and a host
     * will not build a pipeline for points at all unless the shader says it. One
     * pixel is what a title drawing points at named places wants, and it is the
     * size this says until a title is seen to ask for another. */
    let how_big = matches!(kind, Kind::Vertex).then(|| {
        let one = module.name();
        module.add(opcodes::VARIABLE, &[to_size.number(), one.number(), 3]);
        module.annotate(71, &[one.number(), 11, 1]);
        one
    });

    let which_vertex = matches!(kind, Kind::Vertex).then(|| {
        let counting = module.name();
        module.add(opcodes::TYPE_INT, &[counting.number(), 32, 1]);
        let to_counting = module.name();
        module.add(
            opcodes::TYPE_POINTER,
            &[to_counting.number(), 1, counting.number()],
        );
        let one = module.name();
        module.add(opcodes::VARIABLE, &[to_counting.number(), one.number(), 1]);
        module.annotate(71, &[one.number(), 11, 42]);
        (one, counting)
    });

    let carried = the_outputs(
        module,
        kind,
        if matches!(kind, Kind::Vertex) {
            to_out
        } else {
            to_carried
        },
        out,
        entry,
        model,
        &[which_vertex.map(|(one, _)| one), how_big]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
    );

    (carried, how_big, which_vertex)
}

/// Makes the block of constants a shader is handed from outside.
///
/// One block of four float lanes each, laid out the way a host lays out a
/// uniform block, so that what the runtime copies in arrives where the shader
/// looks for it.
fn what_a_shader_is_handed(
    module: &mut Module,
    count: crate::spirv::Name,
    four: crate::spirv::Name,
) -> (crate::spirv::Name, crate::spirv::Name) {
    /* What a shader is handed from outside, which is most of what it reads.
     *
     * One block of four float lanes each, laid out the way a host lays out a
     * uniform block, so that what the runtime copies in arrives where the
     * shader looks for it. */
    let bank_of_constants = module.name();
    module.add(
        opcodes::CONSTANT,
        &[
            count.number(),
            bank_of_constants.number(),
            ALL_THE_CONSTANTS + BESIDE_THE_CONSTANTS,
        ],
    );
    let constant_array = module.name();
    module.add(
        opcodes::TYPE_ARRAY,
        &[
            constant_array.number(),
            four.number(),
            bank_of_constants.number(),
        ],
    );
    module.annotate(71, &[constant_array.number(), 6, 16]);
    let block = module.name();
    module.add(
        opcodes::TYPE_STRUCT,
        &[block.number(), constant_array.number()],
    );
    module.annotate(71, &[block.number(), 2]);
    module.annotate(72, &[block.number(), 0, 35, 0]);
    let to_block = module.name();
    module.add(
        opcodes::TYPE_POINTER,
        &[to_block.number(), 2, block.number()],
    );
    let constants = module.name();
    module.add(
        opcodes::VARIABLE,
        &[to_block.number(), constants.number(), 2],
    );
    module.annotate(71, &[constants.number(), 34, 0]);
    module.annotate(71, &[constants.number(), 33, 0]);
    let to_constant = module.name();
    module.add(
        opcodes::TYPE_POINTER,
        &[to_constant.number(), 2, four.number()],
    );

    (constants, to_constant)
}

/// How many pictures a shader can be handed, which is how many the hardware
/// keeps a place for.
const TEXTURES: u32 = 32;

/// What a shader reads a picture through.
///
/// A texture is not memory a shader works an address into. It is read through
/// hardware that knows how the picture is laid out, which of its sizes to read
/// at, and how to blend between the parts of it. A host has that hardware too,
/// so what is named here is a picture together with the way of reading it, one
/// for each place the console keeps, and a fetch says which of them it means.
fn what_a_shader_looks_at(
    module: &mut Module,
    float: crate::spirv::Name,
    count: crate::spirv::Name,
) -> (crate::spirv::Name, crate::spirv::Name, crate::spirv::Name) {
    let picture = module.name();
    module.add(
        opcodes::TYPE_IMAGE,
        &[picture.number(), float.number(), 1, 0, 0, 0, 1, 0],
    );
    let read_one = module.name();
    module.add(
        opcodes::TYPE_SAMPLED_IMAGE,
        &[read_one.number(), picture.number()],
    );

    let how_many = module.name();
    module.add(
        opcodes::CONSTANT,
        &[count.number(), how_many.number(), TEXTURES],
    );
    let all_of_them = module.name();
    module.add(
        opcodes::TYPE_ARRAY,
        &[all_of_them.number(), read_one.number(), how_many.number()],
    );
    let to_all = module.name();
    module.add(
        opcodes::TYPE_POINTER,
        &[to_all.number(), 0, all_of_them.number()],
    );
    let looked_at = module.name();
    module.add(opcodes::VARIABLE, &[to_all.number(), looked_at.number(), 0]);
    module.annotate(71, &[looked_at.number(), 34, 0]);
    module.annotate(71, &[looked_at.number(), 33, 2]);

    let to_one = module.name();
    module.add(
        opcodes::TYPE_POINTER,
        &[to_one.number(), 0, read_one.number()],
    );

    (looked_at, to_one, read_one)
}

/// The decoration that says a constant is settled when a pair of shaders is
/// built rather than when the module is written.
const SAID_WHEN_BUILT: u32 = 1;

/// The numbers the two settled constants are asked for by.
const WHICH_TEST: u32 = 0;
const WHAT_IT_IS_AGAINST: u32 = 1;

/// The test that draws a place however solid it is, which is what a title that
/// asks for no test at all comes to. Its three bits stand for drawing a place
/// less solid than the test, exactly as solid, and more solid, so all three
/// together draw everything and a driver folds the whole test away.
const DRAWN_WHATEVER_IT_IS: u32 = 7;

/// Writes everything a shader needs before its first instruction: what the
/// module can do, the types it works in, where its registers live, and every
/// number one of them is reached by.
#[allow(clippy::too_many_lines)]
fn preamble(
    module: &mut Module,
    kind: Kind,
    wanted: &[u32],
    samples: bool,
    stretches: usize,
) -> Named {
    let extended = module.name();
    let void = module.name();
    let signature = module.name();
    let float = module.name();
    let four = module.name();
    let count = module.name();
    let bank = module.name();
    let to_bank = module.name();
    let registers = module.name();
    let to_out = module.name();
    let out = module.name();
    let entry = module.name();
    let body = module.name();

    let model = what_the_module_is(module, kind, extended, out, &[void, signature, float, four]);

    let two = module.name();
    module.add(opcodes::TYPE_VECTOR, &[two.number(), float.number(), 2]);
    let three = module.name();
    module.add(opcodes::TYPE_VECTOR, &[three.number(), float.number(), 3]);
    module.add(opcodes::TYPE_INT, &[count.number(), 32, 0]);

    let (to_carried, to_size) =
        where_a_shader_works(module, &[count, four, float, bank, to_bank, to_out, out]);

    let (carried, how_big, which_vertex) = what_a_shader_says(
        module,
        kind,
        model,
        &[to_size, to_out, to_carried, out, entry],
    );

    let to_one = module.name();
    module.add(opcodes::TYPE_POINTER, &[to_one.number(), 7, four.number()]);

    let (constants, to_constant) = what_a_shader_is_handed(module, count, four);

    let zero_index = module.name();
    module.add(opcodes::CONSTANT, &[count.number(), zero_index.number(), 0]);

    let (memory, to_memory, numbers) = what_a_fetch_reads(module, count, wanted);

    let looked_at = if samples {
        Some(what_a_shader_looks_at(module, float, count))
    } else {
        None
    };

    let Standing {
        boolean,
        four_bools,
        nothing,
        all_ones,
        biggest,
        never,
        indices,
    } = the_values(module, float, four, count);

    /* Registers a shader has not written yet read as nothing.
     *
     * Left alone they are whatever the memory held, which a host is entitled to
     * make anything at all of, and a shader reading one before writing it is
     * ordinary rather than a mistake. */
    let empty = module.name();
    module.add(opcodes::CONSTANT_NULL, &[bank.number(), empty.number()]);

    /* Somewhere to keep what carries between stretches that are not run one
     * after another, made here because a place to keep something belongs at
     * the front of a function and nowhere else. */
    let to_whole = module.name();
    module.add(
        opcodes::TYPE_POINTER,
        &[to_whole.number(), 7, count.number()],
    );
    let to_boolean = module.name();
    module.add(
        opcodes::TYPE_POINTER,
        &[to_boolean.number(), 7, boolean.number()],
    );
    let to_float = module.name();
    module.add(
        opcodes::TYPE_POINTER,
        &[to_float.number(), 7, float.number()],
    );
    let no_number = module.name();
    module.add(opcodes::CONSTANT, &[float.number(), no_number.number(), 0]);
    /* The lowest eight bits of a word, which is how many times round a loop
     * goes out of the count holding it. */
    let a_byte = module.name();
    module.add(opcodes::CONSTANT, &[count.number(), a_byte.number(), 255]);
    /* One number for each stretch, and one past the last to mean the shader is
     * done, so that where it is up to can be written. */
    /* Small whole numbers, for naming a bit of a word and a place among the
     * stretches. A number cannot be made inside the function that reads it. */
    let mut whole_numbers = Vec::new();
    for at in 0..=32u32 {
        let one = module.name();
        module.add(opcodes::CONSTANT, &[count.number(), one.number(), at]);
        whole_numbers.push(one);
    }
    /* Filled by the loop above for every number from nought to thirty two. */
    assert_eq!(whole_numbers.len(), 33);
    let mut which_stretch = Vec::new();
    for at in 0..=stretches {
        let one = module.name();
        let at = u32::try_from(at).unwrap_or(u32::MAX);
        module.add(opcodes::CONSTANT, &[count.number(), one.number(), at]);
        which_stretch.push(one);
    }
    let where_it_is_up_to = module.name();
    let times_round = module.name();
    let predicate_kept = module.name();
    let previous_kept = module.name();
    let exported_kept = module.name();

    /* The test a title sets on how solid a place is.
     *
     * The console throws a place away before drawing it where how solid it is
     * does not answer this, and a host has no such switch, so the test is
     * written into the shader itself. Which test it is and what it is against
     * are settled when a pair of shaders is built rather than when one is read,
     * so a single reading serves every test a title asks for. */
    let cutoff = match kind {
        Kind::Pixel => {
            let which = module.name();
            module.add(
                opcodes::SPEC_CONSTANT,
                &[count.number(), which.number(), DRAWN_WHATEVER_IT_IS],
            );
            module.annotate(71, &[which.number(), SAID_WHEN_BUILT, WHICH_TEST]);
            let against = module.name();
            module.add(
                opcodes::SPEC_CONSTANT,
                &[float.number(), against.number(), 0],
            );
            module.annotate(71, &[against.number(), SAID_WHEN_BUILT, WHAT_IT_IS_AGAINST]);
            Some((which, against))
        }
        Kind::Vertex => None,
    };

    module.add(
        opcodes::FUNCTION,
        &[void.number(), entry.number(), 0, signature.number()],
    );
    module.add(opcodes::LABEL, &[body.number()]);
    module.add(
        opcodes::VARIABLE,
        &[to_bank.number(), registers.number(), 7, empty.number()],
    );
    module.add(
        opcodes::VARIABLE,
        &[
            to_whole.number(),
            where_it_is_up_to.number(),
            7,
            zero_index.number(),
        ],
    );
    module.add(
        opcodes::VARIABLE,
        &[
            to_whole.number(),
            times_round.number(),
            7,
            zero_index.number(),
        ],
    );
    module.add(
        opcodes::VARIABLE,
        &[
            to_boolean.number(),
            predicate_kept.number(),
            7,
            never.number(),
        ],
    );
    module.add(
        opcodes::VARIABLE,
        &[
            to_float.number(),
            previous_kept.number(),
            7,
            no_number.number(),
        ],
    );
    module.add(
        opcodes::VARIABLE,
        &[to_one.number(), exported_kept.number(), 7, nothing.number()],
    );

    what_a_shader_starts_with(
        module,
        &Edges {
            kind,
            float,
            four,
            to_one,
            registers,
            all_ones,
            how_big,
            which_vertex,
        },
        &indices,
        &carried,
    );

    Named {
        kind,
        extended,
        four,
        to_one,
        out,
        carried,
        registers,
        boolean,
        float,
        two,
        three,
        constants,
        to_constant,
        zero_index,
        four_bools,
        nothing,
        all_ones,
        biggest,
        never,
        indices,
        whole: count,
        memory,
        to_memory,
        numbers,
        looked_at,
        cutoff,
        a_byte,
        times_round,
        no_number,
        whole_numbers,
        which_stretch,
        where_it_is_up_to,
        predicate_kept,
        previous_kept,
        exported_kept,
    }
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
