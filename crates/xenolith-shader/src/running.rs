//! Saying a shader in a host's terms while a title is running.
//!
//! Most of what a title hands the hardware is in its executable, and that is
//! read once when the executable is turned into a program. Some of it is not:
//! a title loads shaders from the data beside its executable, and those exist
//! only in a running title's memory. Reading the executable can never find one
//! of those however well the rest of this crate handles it.
//!
//! So a running program is given a way to ask. It hands over the words a title
//! handed the hardware and gets back what a driver takes.
//!
//! What comes back is one run of words rather than a shape both sides have to
//! agree on, because a shape shared across two languages is a thing that can be
//! got subtly wrong and that nothing checks. The run begins with three counts
//! and everything after them is read off in order.

use crate::{Kind, Translated, translate};

/// How many words come before the answer itself: how long the module is, how
/// many streams a shader fetches from, and how many pictures it reads.
pub const COUNTS: usize = 3;

/// How many words each stream takes: which fetch constant says where it is, how
/// far apart two vertices sit, how far into one this reads, how much of it, and
/// whether it is fetched by a number the shader worked out.
pub const WORDS_A_STREAM: usize = 5;

/// Says a shader in a host's terms, for a program with one to say while it is
/// running.
///
/// Returns how many words the answer takes, which is nought where the shader
/// could not be said. Where that is more than there was room for, nothing is
/// written and the caller is free to ask again with more.
///
/// # Safety
///
/// `from` must point at `count` readable words, and `into` at `room` writable
/// ones. Either may be null only where its count is nought.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenolith_say_a_shader(
    kind: u32,
    from: *const u32,
    count: u32,
    into: *mut u32,
    room: u32,
) -> u32 {
    let Some(kind) = Kind::from_word(kind) else {
        return 0;
    };
    let Ok(count) = usize::try_from(count) else {
        return 0;
    };
    if from.is_null() || count == 0 {
        return 0;
    }

    // SAFETY: the caller says these words are there to read.
    let words = unsafe { std::slice::from_raw_parts(from, count) };
    /* A title may hand a shader over with a header in front of it, which is
     * what a shader kept in the data beside the executable carries. The run
     * is read from where the shader begins rather than refused for what sits
     * before it. */
    let said = if let Ok(said) = translate(kind, words) {
        said
    } else {
        let Some(found) = crate::scan(words).into_iter().next() else {
            return 0;
        };
        let Some(inside) = words.get(found.at..found.at + found.words) else {
            return 0;
        };
        let Ok(said) = translate(kind, inside) else {
            return 0;
        };
        said
    };

    let answer = laid_out(&said);
    let Ok(length) = u32::try_from(answer.len()) else {
        return 0;
    };
    if length > room || into.is_null() {
        return length;
    }

    // SAFETY: the caller says there is room for this many words, and the line
    // above has established that the answer is no longer than that.
    let there = unsafe { std::slice::from_raw_parts_mut(into, answer.len()) };
    there.copy_from_slice(&answer);
    length
}

/// Lays a translation out as the run of words a running program reads.
fn laid_out(said: &Translated) -> Vec<u32> {
    let mut all = Vec::with_capacity(
        COUNTS + said.words.len() + said.streams.len() * WORDS_A_STREAM + said.pictures.len(),
    );

    all.push(u32::try_from(said.words.len()).unwrap_or_default());
    all.push(u32::try_from(said.streams.len()).unwrap_or_default());
    all.push(u32::try_from(said.pictures.len()).unwrap_or_default());
    all.extend_from_slice(&said.words);
    for stream in &said.streams {
        all.push(stream.slot);
        all.push(stream.stride);
        all.push(stream.offset);
        all.push(stream.words);
        all.push(u32::from(stream.looked_up));
    }
    all.extend_from_slice(&said.pictures);
    all
}
