//! Going and getting a shader's data.
//!
//! A draw hands a shader a number and nothing else. Where the vertex actually
//! is comes from the shader fetching it, out of memory the title filled in
//! beforehand, at an address it works out from that number. So these are not a
//! detail of a shader: for a vertex shader they are where the geometry is.
//!
//! Which slots in a stretch hold a fetch is said by the control flow, so what
//! decides is [`crate::layout::stretches`] and this only reads what is there
//! once that has said so.

use crate::opcodes;
use crate::operands::read_whole;
use crate::preamble::Named;
use crate::spirv::Module;
use crate::values::extract;
use crate::{ALU_WORDS, Fetch, FetchKind, Format, Kind, Refused};

/// Reads one word out of the window on a title's memory.
pub(crate) fn one_word(module: &mut Module, named: &Named, at: u32) -> u32 {
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
pub(crate) fn the_way_a_fetch_says(
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
pub(crate) fn the_numbers_in_the_words(
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
pub(crate) fn taken_apart(
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
pub(crate) fn go_and_get(
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
pub(crate) fn look_at_a_picture(
    module: &mut Module,
    named: &Named,
    fetch: Fetch,
) -> Result<(), Refused> {
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
pub(crate) fn go_and_fetch(
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
pub(crate) fn what_it_reads_from(
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
