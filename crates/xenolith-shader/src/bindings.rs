//! What a shader is handed from outside itself.
//!
//! A shader reads a block of constants a title fills in, a window on the memory
//! a fetch works an address into, and the pictures it looks at. Each of those is
//! a place outside the shader, and each has to be named and given a type before
//! the first instruction, so what they are is said here and nowhere else.

use std::collections::BTreeMap;

use crate::constants::{ALL_THE_CONSTANTS, BESIDE_THE_CONSTANTS, TEXTURES};
use crate::opcodes;
use crate::spirv::Module;

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

/// The pictures a shader was handed, and what reading one needs named.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pictures {
    /// Every place a picture is kept, one after another.
    pub(crate) all: crate::spirv::Name,
    /// The type of a pointer to one of them.
    pub(crate) to_one: crate::spirv::Name,
    /// The type of one of them, together with the way it is read.
    pub(crate) read_one: crate::spirv::Name,
    /// The type of one of them alone.
    pub(crate) picture: crate::spirv::Name,
    /// The type of two whole numbers, which is what a picture's size is said in.
    pub(crate) two_whole: crate::spirv::Name,
    /// Where a level a shader says to read later pictures at is kept, made
    /// where the function begins.
    pub(crate) level: crate::spirv::Name,
}

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
) -> Pictures {
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
    let all = module.name();
    module.add(opcodes::VARIABLE, &[to_all.number(), all.number(), 0]);
    module.annotate(71, &[all.number(), 34, 0]);
    module.annotate(71, &[all.number(), 33, 2]);

    let to_one = module.name();
    module.add(
        opcodes::TYPE_POINTER,
        &[to_one.number(), 0, read_one.number()],
    );
    let two_whole = module.name();
    module.add(
        opcodes::TYPE_VECTOR,
        &[two_whole.number(), count.number(), 2],
    );

    Pictures {
        all,
        to_one,
        read_one,
        picture,
        two_whole,
        level: module.name(),
    }
}
