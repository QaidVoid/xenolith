//! What the fetches in a shader need said before its first instruction.
//!
//! Nothing can be named inside the function that uses it, so every number a
//! fetch works an address or an index out of has to be gathered by reading the
//! whole shader first. That reading writes nothing, and is kept apart from the
//! writing for that reason.

use crate::fetching::what_it_reads_from;
use crate::layout::Step;
use crate::{ALU_WORDS, Fetch, FetchKind, Refused, Stream};

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
