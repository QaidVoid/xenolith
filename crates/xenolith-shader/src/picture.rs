//! Reading a picture, and the fetches a shader asks about one with.
//!
//! Most of what a shader does with a picture is read it at a place and let the
//! hardware blend the texels about that place. A shader that blends them
//! itself, the way one softens the edge of a shadow, asks the hardware three
//! more things instead: which level it would read at, how far a place lies
//! between texels, and to read the texels at a level it says rather than one it
//! chooses. Each is answered here with what a host's own hardware says.

use crate::bindings::Pictures;
use crate::operands::read_whole;
use crate::preamble::Named;
use crate::spirv::Module;
use crate::{Fetch, Kind, Refused, opcodes};

/// What says whether an instruction runs: the way the stretch it is in reads
/// the predicate, if it reads it, and the predicate.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Runs {
    pub(crate) under: Option<bool>,
    pub(crate) predicate: u32,
}

/// Gives the name of whether a fetch runs, or nothing where it always does.
pub(crate) fn whether_a_fetch_runs(
    module: &mut Module,
    named: &Named,
    fetch: Fetch,
    runs: Runs,
) -> Option<u32> {
    if runs.under.is_none() && !fetch.is_predicated() {
        return None;
    }
    if runs.under.unwrap_or_else(|| fetch.predicate_holds()) {
        return Some(runs.predicate);
    }
    let other_way = module.name();
    module.add(
        opcodes::LOGICAL_NOT,
        &[named.boolean.number(), other_way.number(), runs.predicate],
    );
    Some(other_way.number())
}

/// Reads a picture at the place a register names.
///
/// Where in the picture to read is a pair of numbers running from nothing to one
/// across it, which is how the hardware counts and how a host counts too, so
/// they are handed over as they were found. Which two of the register's numbers
/// they are the fetch says, and a title that keeps two places in one register
/// reads the second by naming its last two lanes. How far down the sizes of the
/// picture to read is not said by anything unless the shader said it: it
/// follows from how fast the place being read moves from one part of the screen
/// to the next, and the hardware a host has works that out itself.
///
/// # Errors
///
/// Reports [`Refused`] where the shader names a picture or a register this has
/// no reading of.
pub(crate) fn look_at_a_picture(
    module: &mut Module,
    named: &Named,
    fetch: Fetch,
    runs: Option<u32>,
) -> Result<(), Refused> {
    let pictures = named.looked_at.ok_or(Refused::Fetch)?;
    let (reading, across) = where_a_picture_is_read(module, named, pictures, fetch)?;

    let got = module.name();
    if fetch.reads_at_the_level_said() {
        let level = module.name();
        module.add(
            opcodes::LOAD,
            &[
                named.float.number(),
                level.number(),
                pictures.level.number(),
            ],
        );
        module.add(
            opcodes::IMAGE_SAMPLE_EXPLICIT_LOD,
            &[
                named.four.number(),
                got.number(),
                reading,
                across,
                2,
                level.number(),
            ],
        );
    } else if matches!(named.kind, Kind::Vertex) {
        /* Reading the whole picture rather than a smaller copy of it, said
         * outright because nothing here can be worked out from the screen. */
        module.add(
            opcodes::IMAGE_SAMPLE_EXPLICIT_LOD,
            &[
                named.four.number(),
                got.number(),
                reading,
                across,
                2,
                named.no_number.number(),
            ],
        );
    } else {
        module.add(
            opcodes::IMAGE_SAMPLE_IMPLICIT_LOD,
            &[named.four.number(), got.number(), reading, across],
        );
    }

    put_what_was_read(module, named, fetch, got.number(), runs)
}

/// Puts which level of a picture the hardware would read at a place into every
/// lane of what the fetch brings back.
///
/// A vertex has no screen to work a level out from, so a vertex shader is told
/// the first, which is the one it reads at anyway.
///
/// # Errors
///
/// Reports [`Refused`] where the shader names a picture or a register this has
/// no reading of.
pub(crate) fn the_level_a_place_reads(
    module: &mut Module,
    named: &Named,
    fetch: Fetch,
    runs: Option<u32>,
) -> Result<(), Refused> {
    let pictures = named.looked_at.ok_or(Refused::Fetch)?;
    let (reading, across) = where_a_picture_is_read(module, named, pictures, fetch)?;

    let level = if matches!(named.kind, Kind::Pixel) {
        let both = module.name();
        module.add(
            opcodes::IMAGE_QUERY_LOD,
            &[named.two.number(), both.number(), reading, across],
        );
        /* The second of the two is the level as worked out, before it is kept
         * to the levels the picture has, which is what the console says. */
        let level = module.name();
        module.add(
            opcodes::COMPOSITE_EXTRACT,
            &[named.float.number(), level.number(), both.number(), 1],
        );
        level.number()
    } else {
        named.no_number.number()
    };

    let got = module.name();
    module.add(
        opcodes::COMPOSITE_CONSTRUCT,
        &[
            named.four.number(),
            got.number(),
            level,
            level,
            level,
            level,
        ],
    );
    put_what_was_read(module, named, fetch, got.number(), runs)
}

/// Puts how far a place lies past the middle of the texel before it, across
/// and down, into the first two lanes of what the fetch brings back.
///
/// That is how much of the texel after it a blend of the two takes, which is
/// what a shader blending them itself weighs them by.
///
/// # Errors
///
/// Reports [`Refused`] where the shader names a picture or a register this has
/// no reading of.
pub(crate) fn how_far_between_texels(
    module: &mut Module,
    named: &Named,
    fetch: Fetch,
    runs: Option<u32>,
) -> Result<(), Refused> {
    let pictures = named.looked_at.ok_or(Refused::Fetch)?;
    let (reading, across) = where_a_picture_is_read(module, named, pictures, fetch)?;
    let size = how_big(module, named, pictures, reading);

    let texels = module.name();
    module.add(
        opcodes::FLOAT_MULTIPLY,
        &[named.two.number(), texels.number(), across, size],
    );
    let half = halves(module, named, 1)?;
    let halves_both = module.name();
    module.add(
        opcodes::COMPOSITE_CONSTRUCT,
        &[named.two.number(), halves_both.number(), half, half],
    );
    let from_the_middle = module.name();
    module.add(
        opcodes::FLOAT_SUBTRACT,
        &[
            named.two.number(),
            from_the_middle.number(),
            texels.number(),
            halves_both.number(),
        ],
    );
    let past = module.name();
    module.add(
        opcodes::EXT_INST,
        &[
            named.two.number(),
            past.number(),
            named.extended.number(),
            opcodes::GLSL_FRACT,
            from_the_middle.number(),
        ],
    );

    let got = module.name();
    module.add(
        opcodes::VECTOR_SHUFFLE,
        &[
            named.four.number(),
            got.number(),
            past.number(),
            named.nothing.number(),
            0,
            1,
            2,
            3,
        ],
    );
    put_what_was_read(module, named, fetch, got.number(), runs)
}

/// Keeps the number a register names as the level later fetches read at,
/// where they ask to.
///
/// # Errors
///
/// Reports [`Refused`] where the shader names a register this has no reading
/// of.
pub(crate) fn read_pictures_at_a_level(
    module: &mut Module,
    named: &Named,
    fetch: Fetch,
    runs: Option<u32>,
) -> Result<(), Refused> {
    let pictures = named.looked_at.ok_or(Refused::Fetch)?;
    let from = read_whole(module, named, fetch.source(), true);
    let level = module.name();
    module.add(
        opcodes::COMPOSITE_EXTRACT,
        &[
            named.float.number(),
            level.number(),
            from,
            u32::from(fetch.source_swizzle() & 3),
        ],
    );

    let kept = match runs {
        None => level.number(),
        Some(runs) => {
            let was = module.name();
            module.add(
                opcodes::LOAD,
                &[named.float.number(), was.number(), pictures.level.number()],
            );
            let chosen = module.name();
            module.add(
                opcodes::SELECT,
                &[
                    named.float.number(),
                    chosen.number(),
                    runs,
                    level.number(),
                    was.number(),
                ],
            );
            chosen.number()
        }
    };
    module.add(opcodes::STORE, &[pictures.level.number(), kept]);
    Ok(())
}

/// The picture a fetch reads together with how it is read, and the place in it
/// the fetch reads at, moved as far as the fetch says.
fn where_a_picture_is_read(
    module: &mut Module,
    named: &Named,
    pictures: Pictures,
    fetch: Fetch,
) -> Result<(u32, u32), Refused> {
    let where_in_it = read_whole(module, named, fetch.source(), true);
    let swizzle = u32::from(fetch.source_swizzle());
    let across = module.name();
    module.add(
        opcodes::VECTOR_SHUFFLE,
        &[
            named.two.number(),
            across.number(),
            where_in_it,
            where_in_it,
            swizzle & 3,
            (swizzle >> 2) & 3,
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
            pictures.to_one.number(),
            at.number(),
            pictures.all.number(),
            which.number(),
        ],
    );
    let reading = module.name();
    module.add(
        opcodes::LOAD,
        &[pictures.read_one.number(), reading.number(), at.number()],
    );

    let (moved_across, moved_down) = fetch.moved_by();
    if (moved_across, moved_down) == (0, 0) {
        return Ok((reading.number(), across.number()));
    }

    /* A move is counted in halves of a texel, and a place in fractions of the
     * whole picture, so the move is divided by how big the picture is. */
    let size = how_big(module, named, pictures, reading.number());
    let by_across = halves(module, named, moved_across)?;
    let by_down = halves(module, named, moved_down)?;
    let by = module.name();
    module.add(
        opcodes::COMPOSITE_CONSTRUCT,
        &[named.two.number(), by.number(), by_across, by_down],
    );
    let as_a_fraction = module.name();
    module.add(
        opcodes::FLOAT_DIVIDE,
        &[
            named.two.number(),
            as_a_fraction.number(),
            by.number(),
            size,
        ],
    );
    let moved = module.name();
    module.add(
        opcodes::FLOAT_ADD,
        &[
            named.two.number(),
            moved.number(),
            across.number(),
            as_a_fraction.number(),
        ],
    );
    Ok((reading.number(), moved.number()))
}

/// How many texels across and down the largest level of a picture is.
fn how_big(module: &mut Module, named: &Named, pictures: Pictures, reading: u32) -> u32 {
    let alone = module.name();
    module.add(
        opcodes::IMAGE,
        &[pictures.picture.number(), alone.number(), reading],
    );
    let size = module.name();
    module.add(
        opcodes::IMAGE_QUERY_SIZE_LOD,
        &[
            pictures.two_whole.number(),
            size.number(),
            alone.number(),
            named.zero_index.number(),
        ],
    );
    let as_numbers = module.name();
    module.add(
        opcodes::CONVERT_UNSIGNED_TO_FLOAT,
        &[named.two.number(), as_numbers.number(), size.number()],
    );
    as_numbers.number()
}

/// A number of halves, made out of the small whole numbers already named,
/// because a number cannot be made inside the function that uses it.
fn halves(module: &mut Module, named: &Named, how_many: i32) -> Result<u32, Refused> {
    let whole = |at: u32| {
        named
            .whole_numbers
            .get(at as usize)
            .copied()
            .ok_or(Refused::Fetch)
    };
    let counted = whole(how_many.unsigned_abs())?;
    let two = whole(2)?;

    let as_a_number = module.name();
    module.add(
        opcodes::CONVERT_UNSIGNED_TO_FLOAT,
        &[named.float.number(), as_a_number.number(), counted.number()],
    );
    let two_as_a_number = module.name();
    module.add(
        opcodes::CONVERT_UNSIGNED_TO_FLOAT,
        &[named.float.number(), two_as_a_number.number(), two.number()],
    );
    let halved = module.name();
    module.add(
        opcodes::FLOAT_DIVIDE,
        &[
            named.float.number(),
            halved.number(),
            as_a_number.number(),
            two_as_a_number.number(),
        ],
    );
    if how_many >= 0 {
        return Ok(halved.number());
    }
    let turned = module.name();
    module.add(
        opcodes::FLOAT_NEGATE,
        &[named.float.number(), turned.number(), halved.number()],
    );
    Ok(turned.number())
}

/// Puts what a fetch brought back into the lanes of its destination it names,
/// where it runs.
fn put_what_was_read(
    module: &mut Module,
    named: &Named,
    fetch: Fetch,
    got: u32,
    runs: Option<u32>,
) -> Result<(), Refused> {
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
            0..=3 => (got, says),
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

    let put = match runs {
        None => put.number(),
        Some(runs) => {
            let everywhere = module.name();
            module.add(
                opcodes::COMPOSITE_CONSTRUCT,
                &[
                    named.four_bools.number(),
                    everywhere.number(),
                    runs,
                    runs,
                    runs,
                    runs,
                ],
            );
            let chosen = module.name();
            module.add(
                opcodes::SELECT,
                &[
                    named.four.number(),
                    chosen.number(),
                    everywhere.number(),
                    put.number(),
                    already,
                ],
            );
            chosen.number()
        }
    };

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
    module.add(opcodes::STORE, &[where_to.number(), put]);
    Ok(())
}
