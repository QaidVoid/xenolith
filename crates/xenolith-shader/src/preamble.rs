//! What a module says before any instruction runs.
//!
//! A shader is handed things: the block of constants a title fills in, the
//! window it reads vertices through, the pictures it reads, and what carries
//! from it to whatever runs next. All of that has to be named and given a type
//! before the first instruction, because a host will not take a module that
//! declares something after using it.
//!
//! Nothing here decides what a shader means. It puts the outside of a shader
//! into the module, and the instruction translation works inside that.

use std::collections::BTreeMap;

use crate::Kind;
use crate::opcodes;
use crate::spirv::Module;

/// What a module needs named before anything is done in it.
pub(crate) struct Named {
    pub(crate) kind: Kind,
    pub(crate) extended: crate::spirv::Name,
    pub(crate) four: crate::spirv::Name,
    pub(crate) to_one: crate::spirv::Name,
    pub(crate) out: crate::spirv::Name,
    pub(crate) carried: Vec<crate::spirv::Name>,
    pub(crate) registers: crate::spirv::Name,
    pub(crate) boolean: crate::spirv::Name,
    pub(crate) float: crate::spirv::Name,
    pub(crate) two: crate::spirv::Name,
    pub(crate) three: crate::spirv::Name,
    pub(crate) constants: crate::spirv::Name,
    pub(crate) to_constant: crate::spirv::Name,
    pub(crate) zero_index: crate::spirv::Name,
    pub(crate) four_bools: crate::spirv::Name,
    pub(crate) nothing: crate::spirv::Name,
    pub(crate) all_ones: crate::spirv::Name,
    pub(crate) biggest: crate::spirv::Name,
    pub(crate) never: crate::spirv::Name,
    pub(crate) indices: Vec<crate::spirv::Name>,
    pub(crate) whole: crate::spirv::Name,
    pub(crate) memory: crate::spirv::Name,
    pub(crate) to_memory: crate::spirv::Name,
    pub(crate) numbers: std::collections::BTreeMap<u32, crate::spirv::Name>,
    /* Somewhere to keep what carries from one stretch of a shader to the next
     * where the stretches are not run one after another. A shader that jumps
     * runs its stretches in an order only known while it runs, so what one
     * leaves for another cannot be held in a name the way a straight run's is;
     * it is kept in memory a shader has to itself. Made for every shader
     * because a place to keep something has to be made where a function
     * begins, and left alone by the shaders that run straight through. */
    pub(crate) a_byte: crate::spirv::Name,
    pub(crate) times_round: crate::spirv::Name,
    pub(crate) no_number: crate::spirv::Name,
    pub(crate) whole_numbers: Vec<crate::spirv::Name>,
    pub(crate) which_stretch: Vec<crate::spirv::Name>,
    pub(crate) where_it_is_up_to: crate::spirv::Name,
    pub(crate) predicate_kept: crate::spirv::Name,
    pub(crate) previous_kept: crate::spirv::Name,
    pub(crate) exported_kept: crate::spirv::Name,
    /// The pictures a shader was handed, named only where it reads one.
    pub(crate) looked_at: Option<(crate::spirv::Name, crate::spirv::Name, crate::spirv::Name)>,
    /// The test a title sets on how solid a place has to be to be drawn at all,
    /// named for a pixel shader only: which test it is, and what it is against.
    pub(crate) cutoff: Option<(crate::spirv::Name, crate::spirv::Name)>,
}

/// The values a translated shader needs standing ready before its first
/// instruction: what a comparison produces, the two vectors every operation
/// reaches for, and the number each register and constant is reached by.
///
/// These are kept apart from the rest of the preamble because a constant
/// cannot be written inside the function that uses it, so all of them have to
/// exist before the body starts.
pub(crate) fn the_values(
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
pub(crate) struct Standing {
    pub(crate) boolean: crate::spirv::Name,
    pub(crate) four_bools: crate::spirv::Name,
    pub(crate) nothing: crate::spirv::Name,
    pub(crate) all_ones: crate::spirv::Name,
    pub(crate) biggest: crate::spirv::Name,
    pub(crate) never: crate::spirv::Name,
    pub(crate) indices: Vec<crate::spirv::Name>,
}

/// The places a shader puts what it produces, and the words naming where it
/// starts.
///
/// Where a shader starts is said after its outputs exist, because saying it has
/// to name every one of them, and a module keeps that with the first things it
/// says however late it was worked out.
pub(crate) fn the_outputs(
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
pub(crate) fn what_a_shader_starts_with(
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
pub(crate) struct Edges {
    pub(crate) kind: Kind,
    pub(crate) float: crate::spirv::Name,
    pub(crate) four: crate::spirv::Name,
    pub(crate) to_one: crate::spirv::Name,
    pub(crate) registers: crate::spirv::Name,
    pub(crate) all_ones: crate::spirv::Name,
    pub(crate) how_big: Option<crate::spirv::Name>,
    pub(crate) which_vertex: Option<(crate::spirv::Name, crate::spirv::Name)>,
}

/// Makes the window a shader reads the vertices it goes and gets through, and
/// names every other number a fetch works an address out of.
///
/// Kept apart from the rest of the preamble because none of it is about what a
/// shader is, only about what it is handed.
pub(crate) fn what_a_fetch_reads(
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
pub(crate) fn what_the_module_is(
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
pub(crate) fn where_a_shader_works(
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
pub(crate) fn what_a_shader_says(
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
pub(crate) fn what_a_shader_is_handed(
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
pub(crate) const TEXTURES: u32 = 32;

/// What a shader reads a picture through.
///
/// A texture is not memory a shader works an address into. It is read through
/// hardware that knows how the picture is laid out, which of its sizes to read
/// at, and how to blend between the parts of it. A host has that hardware too,
/// so what is named here is a picture together with the way of reading it, one
/// for each place the console keeps, and a fetch says which of them it means.
pub(crate) fn what_a_shader_looks_at(
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
pub(crate) const SAID_WHEN_BUILT: u32 = 1;

/// The numbers the two settled constants are asked for by.
pub(crate) const WHICH_TEST: u32 = 0;
pub(crate) const WHAT_IT_IS_AGAINST: u32 = 1;

/// The test that draws a place however solid it is, which is what a title that
/// asks for no test at all comes to. Its three bits stand for drawing a place
/// less solid than the test, exactly as solid, and more solid, so all three
/// together draw everything and a driver folds the whole test away.
pub(crate) const DRAWN_WHATEVER_IT_IS: u32 = 7;

/// Writes everything a shader needs before its first instruction: what the
/// module can do, the types it works in, where its registers live, and every
/// number one of them is reached by.
#[allow(clippy::too_many_lines)]
pub(crate) fn preamble(
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

/// Which export a vertex shader puts the position of the vertex in.
///
/// The console's own number for it, and what this title's shaders use: they
/// export to nought and one, which are carried on to whatever runs next, and to
/// this, which is where the vertex went.
pub(crate) const POSITION: u8 = 62;

/// How many things a shader carries on to whatever runs after it.
///
/// Sixteen, which is how many the console has. Fewer was tried, on the grounds
/// that the shaders looked at named only the first four, and a title turned out
/// to name up to the seventh: a shader naming one past the end is refused
/// whole, so whatever it drew is not drawn at all. The number the hardware has
/// is the only one that cannot be wrong.
pub(crate) const CARRIED: u32 = 16;

/// How many more than the constants a shader names are handed to it.
///
/// Two: what the hardware would have done to a position after the shader put it
/// out, as something to multiply by and something to add. On the console that is
/// a set of registers rather than a constant, and whether it is applied at all
/// is a register too, so a host cannot be told once and for all. Handing it over
/// beside the constants is how the same shader serves both a title that leaves
/// the hardware to do it and one that has already done it itself.
pub(crate) const BESIDE_THE_CONSTANTS: u32 = 12;

/// Where among the constants the truths a shader asks after when it jumps are
/// kept: after the scale and the offset, two lots of four words holding the two
/// hundred and fifty six of them a bit apiece.
pub(crate) const THE_TRUTHS: u32 = ALL_THE_CONSTANTS + 2;

/// Where among the constants the counts sit: eight lots of four words holding
/// the thirty two a shader reads to know how many times to go round a loop.
pub(crate) const THE_COUNTS: u32 = ALL_THE_CONSTANTS + 4;

/// How many constants the hardware holds altogether.
///
/// Twice what one shader names, because the two kinds of shader do not share
/// them: what a vertex shader calls its first constant is the file's first, and
/// what a pixel shader calls its first is the file's two hundred and fifty
/// seventh. The file runs to where the constants a fetch reads begin, which is
/// what says there are this many and not some other number.
pub(crate) const ALL_THE_CONSTANTS: u32 = 512;

/// How many constants a shader is handed.
///
/// The console's own number for the float constants a shader can name, and more
/// than the highest one this title's shaders reach.
pub(crate) const CONSTANTS: u32 = 256;

/// How many registers a shader is given.
///
/// The console's own number, and what the six bits naming a destination can
/// reach.
pub(crate) const REGISTERS: u32 = 64;
