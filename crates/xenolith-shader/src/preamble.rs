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

use crate::Kind;
use crate::bindings::{
    Pictures, what_a_fetch_reads, what_a_shader_is_handed, what_a_shader_looks_at,
};
use crate::constants::{
    DRAWN_WHATEVER_IT_IS, KEPT_AS_A_FRACTION, NO_REGISTER, REGISTERS, SAID_WHEN_BUILT,
    WHAT_IT_IS_AGAINST, WHERE_THE_PIXEL_GOES, WHICH_TEST,
};
use crate::edges::{
    Edges, Said, Standing, WhereThePixelIs, the_values, what_a_shader_says,
    what_a_shader_starts_with,
};
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
    /// Where a shader keeps the address it reads constants on from, and the
    /// least and most it can be, as numbers with a point, with the half it is
    /// rounded by.
    pub(crate) address: crate::spirv::Name,
    pub(crate) address_bounds: (crate::spirv::Name, crate::spirv::Name),
    pub(crate) half: crate::spirv::Name,
    /// Where a pixel shader that says how far away its pixel is says it.
    pub(crate) how_far: Option<crate::spirv::Name>,
    /// The pictures a shader was handed, named only where it reads one.
    pub(crate) looked_at: Option<Pictures>,
    /// The test a title sets on how solid a place has to be to be drawn at all,
    /// named for a pixel shader only: which test it is, and what it is against.
    pub(crate) cutoff: Option<(crate::spirv::Name, crate::spirv::Name)>,
    /// Whether what is drawn into keeps a colour as a fraction of the whole,
    /// named for a pixel shader only.
    pub(crate) fraction: Option<crate::spirv::Name>,
}

/// Says what a module can do, which kind of shader it is, where what it
/// produces goes, and the types every one of them works in.
pub(crate) fn what_the_module_is(
    module: &mut Module,
    kind: Kind,
    extended: crate::spirv::Name,
    out: crate::spirv::Name,
    types: &[crate::spirv::Name; 4],
    samples: bool,
) -> u32 {
    let [void, signature, float, four] = *types;

    module.add_early(17, &[1]);
    /* Asking how big a picture is, and which of its levels a place reads,
     * which a shader that blends a picture's texels itself does. */
    if samples {
        module.add_early(17, &[50]);
    }
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

    (to_carried, to_size)
}

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
    says_how_far: bool,
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

    let model = what_the_module_is(
        module,
        kind,
        extended,
        out,
        &[void, signature, float, four],
        samples,
    );

    let two = module.name();
    module.add(opcodes::TYPE_VECTOR, &[two.number(), float.number(), 2]);
    let three = module.name();
    module.add(opcodes::TYPE_VECTOR, &[three.number(), float.number(), 3]);
    module.add(opcodes::TYPE_INT, &[count.number(), 32, 0]);

    let (to_carried, to_size) =
        where_a_shader_works(module, &[count, four, float, bank, to_bank, to_out, out]);

    let Said {
        carried,
        how_big,
        which_vertex,
        where_it_is,
        how_far,
    } = what_a_shader_says(
        module,
        kind,
        model,
        &[to_size, to_out, to_carried, out, entry],
        says_how_far,
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
    let address = module.name();
    let mut a_number = |bits: u32| {
        let one = module.name();
        module.add(opcodes::CONSTANT, &[float.number(), one.number(), bits]);
        one
    };
    let address_bounds = (a_number(0xc380_0000), a_number(0x437f_0000));
    let half = a_number(0x3f00_0000);

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

    /* Whether what is drawn into keeps a colour as a fraction of the whole,
     * which the hardware holds what a shader puts out to and a host does not
     * where it draws into somewhere that keeps more than that. */
    let fraction = matches!(kind, Kind::Pixel).then(|| {
        let one = module.name();
        module.add(opcodes::SPEC_CONSTANT, &[count.number(), one.number(), 0]);
        module.annotate(71, &[one.number(), SAID_WHEN_BUILT, KEPT_AS_A_FRACTION]);
        one
    });

    /* Which register a title asked for where the pixel being drawn is, which
     * is the hardware's to say rather than the shader's and so is settled when
     * a pair of shaders is built. */
    let where_the_pixel_is = where_it_is.and_then(|said| {
        let into = module.name();
        module.add(
            opcodes::SPEC_CONSTANT,
            &[count.number(), into.number(), NO_REGISTER],
        );
        module.annotate(71, &[into.number(), SAID_WHEN_BUILT, WHERE_THE_PIXEL_GOES]);
        let half = module.name();
        module.add(
            opcodes::CONSTANT,
            &[float.number(), half.number(), 0x3f00_0000],
        );
        Some(WhereThePixelIs {
            said,
            into,
            whole: count,
            boolean,
            four_bools,
            registers_there_are: *indices.get(REGISTERS as usize)?,
            first: *indices.first()?,
            half,
            zero: no_number,
        })
    });

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
    module.add(
        opcodes::VARIABLE,
        &[to_whole.number(), address.number(), 7, zero_index.number()],
    );
    if let Some(pictures) = looked_at {
        module.add(
            opcodes::VARIABLE,
            &[
                to_float.number(),
                pictures.level.number(),
                7,
                no_number.number(),
            ],
        );
    }

    /* How far away the shape says the pixel is, which stands where the
     * shader's own word on it runs on a predicate that does not hold. */
    if let (Some(told), Some(here)) = (how_far, where_it_is) {
        let read = module.name();
        module.add(
            opcodes::LOAD,
            &[four.number(), read.number(), here.number()],
        );
        let away = module.name();
        module.add(
            opcodes::COMPOSITE_EXTRACT,
            &[float.number(), away.number(), read.number(), 2],
        );
        module.add(opcodes::STORE, &[told.number(), away.number()]);
    }

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
            where_the_pixel_is,
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
        fraction,
        a_byte,
        times_round,
        no_number,
        whole_numbers,
        which_stretch,
        where_it_is_up_to,
        predicate_kept,
        previous_kept,
        exported_kept,
        address,
        address_bounds,
        half,
        how_far,
    }
}
