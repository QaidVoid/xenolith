//! What the fetches in a shader need said before its first instruction.
//!
//! Nothing can be named inside the function that uses it, so every number a
//! fetch works an address or an index out of has to be gathered by reading the
//! whole shader first. That reading writes nothing, and is kept apart from the
//! writing for that reason.

use crate::fetching::what_it_reads_from;
use crate::layout::Step;
use crate::{ALU_WORDS, Alu, Fetch, FetchKind, Refused, Stream};

/// Everything the fetches in a shader need said before its first instruction.
pub(crate) struct Fetches {
    /// Every number one of them works an address or an index out of.
    pub(crate) wanted: Vec<u32>,
    /// Where the shader goes for each of its vertex streams.
    pub(crate) streams: Vec<Stream>,
    /// Whether any of them reads a texture, which needs saying because a shader
    /// that reads none must not name a texture it never looks at.
    pub(crate) samples: bool,
    /// Which places it reads one from, so that a host need only make ready the
    /// pictures a shader will actually look at.
    pub(crate) pictures: Vec<u32>,
}

/// Reports [`Refused`] where a fetch is one this has no reading of.
pub(crate) fn what_the_fetches_need(words: &[u32], running: &[Step]) -> Result<Fetches, Refused> {
    /* What every fetch works its address out of, gathered before anything is
     * written, because a number cannot be named inside the function that uses
     * it and these are not small enough to be among the ones already there. */
    let mut wanted = Vec::new();
    let mut streams = Vec::new();
    let mut samples = false;
    let mut pictures = Vec::new();
    let mut carried_over: Option<(u32, u32)> = None;
    let mut carried_looked_up = false;
    /* Which lanes of each register something has written so far, in the order
     * the shader runs. A fetch by a lane nothing has written yet is by the
     * number the hardware put there, which is the vertex's own. */
    let mut written = [0u8; 64];
    for step in running {
        let one = step.at;
        let at = one.checked_mul(ALU_WORDS).ok_or(Refused::NotAShader)?;
        if !step.fetches {
            if let Some(alu) = Alu::at(words, at)
                && !alu.exports()
            {
                mark_written(&mut written, alu.vector_destination(), alu.vector_lanes());
                mark_written(&mut written, alu.scalar_destination(), alu.scalar_lanes());
            }
            continue;
        }
        let Some(fetch) = Fetch::at(words, at) else {
            return Err(Refused::NotAShader);
        };
        let looked_up = if fetch.follows_another() {
            carried_looked_up
        } else {
            written
                .get(usize::from(fetch.source()))
                .is_some_and(|lanes| lanes & (1 << fetch.source_component()) != 0)
        };
        mark_written(
            &mut written,
            fetch.destination(),
            lanes_a_fetch_writes(fetch),
        );
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
        carried_looked_up = looked_up;
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
            looked_up,
        });
    }

    Ok(Fetches {
        wanted,
        streams,
        samples,
        pictures,
    })
}

/// Notes that something wrote these lanes of a register.
fn mark_written(written: &mut [u8; 64], register: u8, lanes: u8) {
    if let Some(held) = written.get_mut(usize::from(register)) {
        *held |= lanes;
    }
}

/// Which lanes of its destination a fetch writes: every one its swizzle does
/// not say to leave alone.
fn lanes_a_fetch_writes(fetch: Fetch) -> u8 {
    let swizzle = fetch.destination_swizzle();
    (0..4u16).fold(0, |lanes, lane| {
        if (swizzle >> (lane * 3)) & 7 == 7 {
            lanes
        } else {
            lanes | (1 << lane)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::what_the_fetches_need;
    use crate::layout::Step;

    /// A fetch of four floats by the given register and lane, into the given
    /// register, from the given constant, a vertex every four words.
    fn a_fetch_by(source: u32, lane: u32, into: u32, constant: u32) -> [u32; 3] {
        [
            (source << 5) | (into << 12) | (constant << 20) | (lane << 30),
            (38 << 16) | 0x688,
            4,
        ]
    }

    /// An instruction whose four lane half writes only the first lane of a
    /// register.
    fn a_write_to(register: u32) -> [u32; 3] {
        [register | (1 << 16), 0, 0]
    }

    fn streams_of(instructions: &[([u32; 3], bool)]) -> Vec<bool> {
        let words: Vec<u32> = instructions.iter().flat_map(|(one, _)| *one).collect();
        let running: Vec<Step> = instructions
            .iter()
            .enumerate()
            .map(|(at, (_, fetches))| Step {
                at,
                fetches: *fetches,
                under: None,
            })
            .collect();
        let Ok(fetches) = what_the_fetches_need(&words, &running) else {
            panic!("the fetches were refused");
        };
        fetches.streams.iter().map(|one| one.looked_up).collect()
    }

    #[test]
    fn a_fetch_by_the_number_a_vertex_arrives_with_is_by_the_vertex() {
        assert_eq!(streams_of(&[(a_fetch_by(0, 0, 1, 0), true)]), [false]);
    }

    #[test]
    fn a_fetch_by_a_number_the_shader_worked_out_is_looked_up() {
        let looked_up = streams_of(&[
            (a_fetch_by(0, 0, 1, 0), true),
            (a_write_to(0), false),
            (a_fetch_by(0, 0, 2, 1), true),
        ]);
        assert_eq!(looked_up, [false, true]);
    }

    #[test]
    fn a_fetch_by_what_another_fetch_brought_back_is_looked_up() {
        let looked_up = streams_of(&[
            (a_fetch_by(0, 0, 0, 0), true),
            (a_fetch_by(0, 0, 2, 1), true),
            (a_fetch_by(1, 0, 3, 2), true),
        ]);
        assert_eq!(looked_up, [false, true, false]);
    }
}
