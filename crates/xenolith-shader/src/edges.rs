//! What a shader says at its edges, and the values it needs to say it.
//!
//! What it produces, what it carries on to whatever runs after it, how big a
//! point is, and which vertex this is. A host looks for these by name, so they
//! are made together and the names are what the rest of the translation is
//! written against.
//!
//! Beside them are the values a comparison produces and the number every
//! register and constant is reached by. Those are not edges, but they are the
//! same kind of thing: named before the function begins, because a constant
//! cannot be written inside the function that uses it.

use crate::Kind;
use crate::constants::{ALL_THE_CONSTANTS, BESIDE_THE_CONSTANTS, CARRIED};
use crate::opcodes;
use crate::spirv::Module;

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
        where_the_pixel_is,
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

    if let Some(asked) = where_the_pixel_is {
        where_the_pixel_goes(module, float, four, to_one, registers, asked);
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
    pub(crate) where_the_pixel_is: Option<WhereThePixelIs>,
}

/// What putting where the pixel being drawn is into a register needs: where a
/// host says it is, which register a title asked for it in, and the values the
/// two are worked into the console's terms with.
#[derive(Clone, Copy)]
pub(crate) struct WhereThePixelIs {
    pub(crate) said: crate::spirv::Name,
    pub(crate) into: crate::spirv::Name,
    pub(crate) whole: crate::spirv::Name,
    pub(crate) boolean: crate::spirv::Name,
    pub(crate) four_bools: crate::spirv::Name,
    pub(crate) registers_there_are: crate::spirv::Name,
    pub(crate) first: crate::spirv::Name,
    pub(crate) half: crate::spirv::Name,
    pub(crate) zero: crate::spirv::Name,
}

/// Makes what a shader says at its edges: how big a point is, which vertex this
/// is, where the pixel being drawn is, and what it carries on to whatever runs
/// after it.
pub(crate) fn what_a_shader_says(
    module: &mut Module,
    kind: Kind,
    model: u32,
    names: &[crate::spirv::Name; 5],
    says_how_far: bool,
) -> Said {
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

    /* Where on what is drawn into the pixel being drawn is, which a pixel
     * shader can ask the hardware to put in one of its registers. */
    let where_it_is = matches!(kind, Kind::Pixel).then(|| {
        let one = module.name();
        module.add(opcodes::VARIABLE, &[to_carried.number(), one.number(), 1]);
        module.annotate(71, &[one.number(), 11, 15]);
        one
    });

    /* How far away the pixel being drawn is, where a pixel shader says so
     * itself rather than leaving it to the shape. Only a shader that says it is
     * given it, because a host that has to wait for the shader to know how far
     * away a pixel is can no longer throw away the pixels behind what is
     * already drawn before running it. */
    let how_far = (matches!(kind, Kind::Pixel) && says_how_far).then(|| {
        let one = module.name();
        module.add(opcodes::VARIABLE, &[to_size.number(), one.number(), 3]);
        module.annotate(71, &[one.number(), 11, 22]);
        one
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
        &[
            which_vertex.map(|(one, _)| one),
            how_big,
            where_it_is,
            how_far,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>(),
    );
    if how_far.is_some() {
        module.add_early(16, &[entry.number(), 12]);
    }

    Said {
        carried,
        how_big,
        which_vertex,
        where_it_is,
        how_far,
    }
}

/// What [`what_a_shader_says`] made.
pub(crate) struct Said {
    pub(crate) carried: Vec<crate::spirv::Name>,
    pub(crate) how_big: Option<crate::spirv::Name>,
    pub(crate) which_vertex: Option<(crate::spirv::Name, crate::spirv::Name)>,
    pub(crate) where_it_is: Option<crate::spirv::Name>,
    pub(crate) how_far: Option<crate::spirv::Name>,
}

/// Puts where the pixel being drawn is into the register a title asked for it
/// in, where it asked for one.
///
/// The console counts a pixel from its top left corner where a host counts it
/// from its middle, so half a pixel comes off; a title adds the half back itself
/// before reading a picture of the same size at that place. Where a title asked
/// for no register, the first is written with what it already held, which
/// leaves the register as it was without a branch for a driver to follow.
fn where_the_pixel_goes(
    module: &mut Module,
    float: crate::spirv::Name,
    four: crate::spirv::Name,
    to_one: crate::spirv::Name,
    registers: crate::spirv::Name,
    asked: WhereThePixelIs,
) {
    let wanted = module.name();
    module.add(
        opcodes::UNSIGNED_LESS_THAN,
        &[
            asked.boolean.number(),
            wanted.number(),
            asked.into.number(),
            asked.registers_there_are.number(),
        ],
    );
    let which = module.name();
    module.add(
        opcodes::SELECT,
        &[
            asked.whole.number(),
            which.number(),
            wanted.number(),
            asked.into.number(),
            asked.first.number(),
        ],
    );
    let where_to = module.name();
    module.add(
        opcodes::ACCESS_CHAIN,
        &[
            to_one.number(),
            where_to.number(),
            registers.number(),
            which.number(),
        ],
    );
    let held = module.name();
    module.add(
        opcodes::LOAD,
        &[four.number(), held.number(), where_to.number()],
    );

    let said = module.name();
    module.add(
        opcodes::LOAD,
        &[four.number(), said.number(), asked.said.number()],
    );
    let lane = |module: &mut Module, which: u32| {
        let one = module.name();
        module.add(
            opcodes::COMPOSITE_EXTRACT,
            &[float.number(), one.number(), said.number(), which],
        );
        let moved = module.name();
        module.add(
            opcodes::FLOAT_SUBTRACT,
            &[
                float.number(),
                moved.number(),
                one.number(),
                asked.half.number(),
            ],
        );
        moved.number()
    };
    let across = lane(module, 0);
    let down = lane(module, 1);
    let position = module.name();
    module.add(
        opcodes::COMPOSITE_CONSTRUCT,
        &[
            four.number(),
            position.number(),
            across,
            down,
            asked.zero.number(),
            asked.zero.number(),
        ],
    );

    let every_lane = module.name();
    module.add(
        opcodes::COMPOSITE_CONSTRUCT,
        &[
            asked.four_bools.number(),
            every_lane.number(),
            wanted.number(),
            wanted.number(),
            wanted.number(),
            wanted.number(),
        ],
    );
    let written = module.name();
    module.add(
        opcodes::SELECT,
        &[
            four.number(),
            written.number(),
            every_lane.number(),
            position.number(),
            held.number(),
        ],
    );
    module.add(opcodes::STORE, &[where_to.number(), written.number()]);
}
