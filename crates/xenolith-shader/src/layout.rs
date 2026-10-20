//! Reading a shader's control flow into the stretches it runs.
//!
//! A control flow instruction is forty eight bits and two share three words.
//! Each says which stretch of the others to run, and where the next one is.
//! What comes out is the instructions in the order they are stored, each
//! marked with whether the flow said it fetches rather than computes.

use crate::{ALU_WORDS, Flow, FlowOperation};

/// One instruction the control flow says to run.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Step {
    /// Which instruction it is.
    pub(crate) at: usize,
    /// Whether the control flow marked it a fetch rather than arithmetic.
    pub(crate) fetches: bool,
    /// Whether the stretch it belongs to runs only where the predicate holds,
    /// and which way round.
    pub(crate) under: Option<bool>,
}

/// The instructions a shader's control flow says to run, in order.
///
/// A stretch may be one that runs only where the predicate holds, which is
/// carried on each instruction rather than turned into a branch: the module
/// this writes is one straight run, so what a predicated instruction writes is
/// chosen between its answer and what was already there.
pub(crate) fn stretches(words: &[u32]) -> Option<Vec<Step>> {
    let mut running = Vec::new();
    let mut at = 0;
    let mut ended = false;

    while let Some((low, high)) = Flow::pair(words, at) {
        for flow in [low, high] {
            if ended {
                continue;
            }
            let under = match flow.operation() {
                FlowOperation::Run | FlowOperation::RunAndEnd => None,
                FlowOperation::RunIfPredicated
                | FlowOperation::RunIfPredicatedAndEnd
                | FlowOperation::RunIfPredicatedClean
                | FlowOperation::RunIfPredicatedCleanAndEnd => Some(flow.predicate_holds()),
                FlowOperation::Nothing | FlowOperation::Reserve => continue,
                _ => return None,
            };

            let first = usize::from(flow.address());
            for step in 0..usize::from(flow.count()) {
                running.push(Step {
                    at: first.checked_add(step)?,
                    fetches: flow.fetches_at(step),
                    under,
                });
            }
            if flow.ends_the_shader() {
                ended = true;
            }
        }
        at += ALU_WORDS;
        if ended {
            return Some(running);
        }
    }

    None
}

/// Where a stretch goes once it has run.
#[derive(Debug, Clone, Copy)]
pub(crate) enum WhereNext {
    /// On to the stretch after it.
    Along,
    /// Nowhere: the shader is done.
    Ends,
    /// Straight to another stretch, asking after nothing.
    Goes { to: usize },
    /// Begins a loop: how many times round is read, and where to go when it is
    /// no times at all.
    Rounds { count: u8, past: usize },
    /// Ends a loop: one off how many are left, and back to the top while any
    /// remain.
    Again { top: usize },
    /// To another stretch, where what it asks after reads the way it says.
    Jumps {
        to: usize,
        when: bool,
        asking: Asking,
    },
}

/// What a jump asks after to decide whether it goes.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Asking {
    /// The predicate, as the stretches that run on it do.
    Predicate,
    /// One of the truths a title handed over, named by where it sits.
    Truth(u8),
}

/// One stretch of a shader that jumps: what it runs and where it goes after.
pub(crate) struct Stretch {
    pub(crate) steps: Vec<Step>,
    pub(crate) next: WhereNext,
}

/// Reads the control flow as stretches that each say where they go, for a
/// shader whose stretches are not simply run one after another.
///
/// A shader that jumps cannot be laid out as one run of instructions, because
/// which of them run and in what order is only settled while it runs. So each
/// control flow instruction becomes a stretch of its own, kept at the place the
/// jumps name it by, and what it does when it has run is written down beside
/// it.
///
/// Nothing is guessed at: an operation with no reading here stops the whole
/// reading, so a shader that loops or calls is refused rather than translated
/// into something that runs and means something else.
pub(crate) fn stretches_that_jump(words: &[u32]) -> Option<Vec<Stretch>> {
    let mut all: Vec<Stretch> = Vec::new();
    let mut at = 0;
    let mut ended = false;

    while let Some((low, high)) = Flow::pair(words, at) {
        for flow in [low, high] {
            if ended {
                break;
            }
            let operation = flow.operation();
            match operation {
                /* Nothing to run and nowhere of its own to go. Kept all the
                 * same, because the jumps name a stretch by where it sits. */
                FlowOperation::Nothing | FlowOperation::Reserve | FlowOperation::FetchDone => {
                    all.push(Stretch {
                        steps: Vec::new(),
                        next: WhereNext::Along,
                    });
                }
                FlowOperation::LoopBegins => {
                    all.push(Stretch {
                        steps: Vec::new(),
                        next: WhereNext::Rounds {
                            count: flow.loop_named(),
                            past: usize::from(flow.jump_target()),
                        },
                    });
                }
                FlowOperation::LoopEnds => {
                    all.push(Stretch {
                        steps: Vec::new(),
                        next: WhereNext::Again {
                            top: usize::from(flow.jump_target()),
                        },
                    });
                }
                FlowOperation::JumpIf => {
                    if flow.goes_regardless() {
                        all.push(Stretch {
                            steps: Vec::new(),
                            next: WhereNext::Goes {
                                to: usize::from(flow.jump_target()),
                            },
                        });
                        continue;
                    }
                    all.push(Stretch {
                        steps: Vec::new(),
                        next: WhereNext::Jumps {
                            to: usize::from(flow.jump_target()),
                            when: flow.predicate_holds(),
                            asking: if flow.is_predicated() {
                                Asking::Predicate
                            } else {
                                Asking::Truth(flow.bool_index())
                            },
                        },
                    });
                }
                FlowOperation::Run
                | FlowOperation::RunAndEnd
                | FlowOperation::RunIfPredicated
                | FlowOperation::RunIfPredicatedAndEnd
                | FlowOperation::RunIfPredicatedClean
                | FlowOperation::RunIfPredicatedCleanAndEnd => {
                    let under = match operation {
                        FlowOperation::RunIfPredicated
                        | FlowOperation::RunIfPredicatedAndEnd
                        | FlowOperation::RunIfPredicatedClean
                        | FlowOperation::RunIfPredicatedCleanAndEnd => Some(flow.predicate_holds()),
                        _ => None,
                    };
                    let first = usize::from(flow.address());
                    let mut steps = Vec::new();
                    for step in 0..usize::from(flow.count()) {
                        steps.push(Step {
                            at: first.checked_add(step)?,
                            fetches: flow.fetches_at(step),
                            under,
                        });
                    }
                    let next = if flow.ends_the_shader() {
                        ended = true;
                        WhereNext::Ends
                    } else {
                        WhereNext::Along
                    };
                    all.push(Stretch { steps, next });
                }
                _ => return None,
            }
        }
        at += ALU_WORDS;
        if ended {
            break;
        }
    }

    /* A run of stretches that never stops is not a shader: something would run
     * off the end of what was handed over. */
    if !ended {
        return None;
    }
    Some(all)
}
