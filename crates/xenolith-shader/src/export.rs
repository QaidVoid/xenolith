//! What a shader puts out, and what it throws away.
//!
//! A pixel shader writes a colour per place on the screen, and a vertex shader
//! writes where the vertex went and what carries on with it. Both are stores to
//! something outside the shader, so this is where the inside of a shader meets
//! whatever runs next.
//!
//! The test a title sets on how solid a place has to be before it is drawn is
//! here too: the hardware discards a place that fails it, and a host says the
//! same by ending the invocation.

use crate::Kind;
use crate::arithmetic::read_a_constant;
use crate::opcodes;
use crate::preamble::{ALL_THE_CONSTANTS, Named};
use crate::spirv::Module;

/// Writes out what a shader puts out, and the end of it.
pub(crate) fn what_it_puts_out(module: &mut Module, named: &Named, exported: Option<u32>) {
    if let Some(value) = exported {
        /* What the hardware would have done to this, done here instead: a
         * console applies a scale and an offset to what a vertex shader puts
         * out, and a host has no such switch. */
        let value = if matches!(named.kind, Kind::Vertex) {
            let scale = read_a_constant(module, named, ALL_THE_CONSTANTS);
            let offset = read_a_constant(module, named, ALL_THE_CONSTANTS + 1);
            let scaled = module.name();
            module.add(
                opcodes::FLOAT_MULTIPLY,
                &[named.four.number(), scaled.number(), value, scale],
            );
            let moved = module.name();
            module.add(
                opcodes::FLOAT_ADD,
                &[named.four.number(), moved.number(), scaled.number(), offset],
            );
            moved.number()
        } else {
            value
        };

        if let Some((which, against)) = named.cutoff {
            throw_away_what_is_not_solid_enough(module, named, value, which, against);
        }

        module.add(opcodes::STORE, &[named.out.number(), value]);
    }

    module.add(opcodes::RETURN, &[]);
    module.add(opcodes::FUNCTION_END, &[]);
}

/// Writes the test a title sets on how solid a place has to be to be drawn.
///
/// The console makes this test itself, before what a pixel shader worked out
/// reaches the surface, and a host has nothing that does it, so the shader ends
/// by throwing the place away where the test does not hold.
///
/// The three bits of the test say whether a place less solid than the test,
/// exactly as solid, and more solid are each drawn, so the whole test is those
/// three comparisons and nothing else. Written this way it needs no reading of
/// which test it is: the bits and the comparisons stand side by side, and a
/// driver folds both away when the test draws everything.
pub(crate) fn throw_away_what_is_not_solid_enough(
    module: &mut Module,
    named: &Named,
    value: u32,
    which: crate::spirv::Name,
    against: crate::spirv::Name,
) {
    let Some(none) = named.whole_numbers.first().copied() else {
        return;
    };

    let solid = module.name();
    module.add(
        opcodes::COMPOSITE_EXTRACT,
        &[named.float.number(), solid.number(), value, 3],
    );

    let mut drawn = None;
    for (comparison, bit) in [
        (opcodes::FLOAT_ORDERED_LESS_THAN, 1usize),
        (opcodes::FLOAT_ORDERED_EQUAL, 2),
        (opcodes::FLOAT_ORDERED_GREATER_THAN, 4),
    ] {
        let Some(one) = named.whole_numbers.get(bit).copied() else {
            return;
        };

        let holds = module.name();
        module.add(
            comparison,
            &[
                named.boolean.number(),
                holds.number(),
                solid.number(),
                against.number(),
            ],
        );
        let kept = module.name();
        module.add(
            opcodes::BITWISE_AND,
            &[
                named.whole.number(),
                kept.number(),
                which.number(),
                one.number(),
            ],
        );
        let asked = module.name();
        module.add(
            opcodes::INTEGER_NOT_EQUAL,
            &[
                named.boolean.number(),
                asked.number(),
                kept.number(),
                none.number(),
            ],
        );
        let both = module.name();
        module.add(
            opcodes::LOGICAL_AND,
            &[
                named.boolean.number(),
                both.number(),
                holds.number(),
                asked.number(),
            ],
        );

        drawn = Some(match drawn {
            None => both.number(),
            Some(before) => {
                let either = module.name();
                module.add(
                    opcodes::LOGICAL_OR,
                    &[
                        named.boolean.number(),
                        either.number(),
                        before,
                        both.number(),
                    ],
                );
                either.number()
            }
        });
    }

    let Some(drawn) = drawn else {
        return;
    };

    let thrown = module.name();
    let after = module.name();
    module.add(opcodes::SELECTION_MERGE, &[after.number(), 0]);
    module.add(
        opcodes::BRANCH_CONDITIONAL,
        &[drawn, after.number(), thrown.number()],
    );
    module.add(opcodes::LABEL, &[thrown.number()]);
    module.add(opcodes::KILL, &[]);
    module.add(opcodes::LABEL, &[after.number()]);
}
