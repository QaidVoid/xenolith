//! What came of translating a shader, and why one was not translated.
//!
//! The translation itself is in [`crate::dispatch`]. What is here is what a
//! caller reads: the words of the module, where a title's memory each stream
//! comes from, which pictures the shader reads, and the list of reasons a shader
//! can be refused.
//!
//! The reasons are a closed set rather than a message, so a caller can say what
//! was not read and a new one cannot be added without every match on it being
//! looked at. A refusal is a value, not a silent gap: an operation this has no
//! reading of stops the translation, because a shader that draws the wrong thing
//! is harder to find than one that was never translated.

use crate::{ScalarOperation, VectorOperation};

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
    /// Whether the number it fetches by is one the shader worked out rather
    /// than the vertex's own.
    ///
    /// A title keeps a table this way, read at a number the shader made out of
    /// what else it fetched or out of the vertex's own number: a matrix for
    /// each part of a scene, or the corners and centre of each particle. Such
    /// a stream is not one vertex after another, so a host laying vertices out
    /// for itself has to leave it whole and in its own order.
    pub looked_up: bool,
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
