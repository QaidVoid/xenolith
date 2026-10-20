//! Writing what a host graphics driver takes.
//!
//! A shader read out of a title is instructions for hardware that is not here,
//! so something has to say the same thing in a form a host understands. That
//! form is SPIR-V: a run of words, each instruction carrying its own length, in
//! a module whose header says how many names it uses.
//!
//! Nothing here decides what a shader means. This puts instructions in a file
//! and counts the names, and what to put in it is decided elsewhere. Keeping
//! the two apart is what lets the writing be checked on its own, which it is:
//! what this produces is handed to the validator a host driver would use.

/// The word every SPIR-V module starts with.
const MAGIC: u32 = 0x0723_0203;

/// Which version of the form this writes, as the major and minor the header
/// wants. One point three is what a Vulkan one point one driver takes.
const VERSION: u32 = 0x0001_0300;

/// Who wrote it. Zero is the number kept for a tool with none of its own, which
/// this is until one is asked for.
const GENERATOR: u32 = 0;

/// A name inside a module.
///
/// Every type, constant, variable and instruction that produces something is
/// named by one of these, and the header says how many there are so a driver
/// can make room before it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Name(u32);

impl Name {
    /// The number a name is written as.
    #[must_use]
    pub(crate) fn number(self) -> u32 {
        self.0
    }
}

/// The operands of an instruction carrying text, which SPIR-V stores as words
/// with a zero byte marking the end.
fn with_text(before: &[u32], text: &str, after: &[u32]) -> Vec<u32> {
    let mut operands = before.to_vec();
    let mut word = 0u32;
    let mut filled = 0;

    for byte in text.bytes().chain(std::iter::once(0)) {
        word |= u32::from(byte) << (filled * 8);
        filled += 1;
        if filled == 4 {
            operands.push(word);
            word = 0;
            filled = 0;
        }
    }
    if filled != 0 {
        operands.push(word);
    }

    operands.extend_from_slice(after);
    operands
}

/// A module being written.
#[derive(Debug, Default)]
pub(crate) struct Module {
    head: Vec<u32>,
    annotations: Vec<u32>,
    words: Vec<u32>,
    next: u32,
}

impl Module {
    /// A module with nothing in it yet.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            head: Vec::new(),
            annotations: Vec::new(),
            words: Vec::new(),
            next: 1,
        }
    }

    /// A name nothing else has.
    pub(crate) fn name(&mut self) -> Name {
        let taken = Name(self.next);
        self.next += 1;
        taken
    }

    /// Adds one instruction: what it does, what it produces, and the rest of
    /// what it says.
    ///
    /// The length goes in the top half of the first word, which is why an
    /// instruction cannot be added a piece at a time.
    pub(crate) fn add(&mut self, operation: u16, operands: &[u32]) {
        let Ok(length) = u16::try_from(operands.len() + 1) else {
            return;
        };
        self.words
            .push((u32::from(length) << 16) | u32::from(operation));
        self.words.extend_from_slice(operands);
    }

    /// Adds an instruction that says something about a name rather than doing
    /// anything.
    ///
    /// These are kept apart because a module says them all together, before the
    /// first type it names. Written where they were thought of, a driver
    /// refuses the module for being out of order, which is the sort of thing
    /// only the thing that reads it will tell you.
    pub(crate) fn annotate(&mut self, operation: u16, operands: &[u32]) {
        let Ok(length) = u16::try_from(operands.len() + 1) else {
            return;
        };
        self.annotations
            .push((u32::from(length) << 16) | u32::from(operation));
        self.annotations.extend_from_slice(operands);
    }

    /// Adds an instruction that comes before anything a module names: what it
    /// can do, how it reads memory, where it starts, and how it is run.
    pub(crate) fn add_early(&mut self, operation: u16, operands: &[u32]) {
        let Ok(length) = u16::try_from(operands.len() + 1) else {
            return;
        };
        self.head
            .push((u32::from(length) << 16) | u32::from(operation));
        self.head.extend_from_slice(operands);
    }

    /// Adds an instruction carrying text before anything a module names.
    pub(crate) fn add_early_with_text(
        &mut self,
        operation: u16,
        before: &[u32],
        text: &str,
        after: &[u32],
    ) {
        let operands = with_text(before, text, after);
        self.add_early(operation, &operands);
    }

    /// Adds an instruction carrying text, which SPIR-V stores as words with a
    /// zero byte marking the end.
    ///
    /// What comes after the text is taken as well, because the instruction
    /// saying where a shader starts names the text and then everything the
    /// shader talks to the outside with.
    #[cfg(test)]
    pub(crate) fn add_with_text(
        &mut self,
        operation: u16,
        before: &[u32],
        text: &str,
        after: &[u32],
    ) {
        let operands = with_text(before, text, after);
        self.add(operation, &operands);
    }

    /// The whole module, header and all, as the words a driver reads.
    #[must_use]
    pub(crate) fn finish(&self) -> Vec<u32> {
        let mut all =
            Vec::with_capacity(self.head.len() + self.annotations.len() + self.words.len() + 5);
        all.push(MAGIC);
        all.push(VERSION);
        all.push(GENERATOR);
        all.push(self.next);
        all.push(0);
        all.extend_from_slice(&self.head);
        all.extend_from_slice(&self.annotations);
        all.extend_from_slice(&self.words);
        all
    }
}

#[cfg(test)]
mod tests {
    use super::{MAGIC, Module};

    #[test]
    fn a_module_starts_with_the_word_that_says_what_it_is() {
        let module = Module::new();
        assert_eq!(module.finish().first().copied(), Some(MAGIC));
    }

    #[test]
    fn the_header_says_how_many_names_are_used() {
        let mut module = Module::new();
        let first = module.name();
        let second = module.name();
        assert_ne!(first, second);
        /* One past the last handed out, which is what the header asks for. */
        assert_eq!(module.finish().get(3).copied(), Some(3));
    }

    #[test]
    fn an_instruction_carries_its_own_length() {
        let mut module = Module::new();
        module.add(17, &[1, 2]);
        let words = module.finish();
        assert_eq!(words.get(5).copied(), Some((3u32 << 16) | 0x11));
    }

    #[test]
    fn text_ends_with_a_byte_of_nothing() {
        let mut module = Module::new();
        module.add_with_text(5, &[9], "ab", &[]);
        let words = module.finish();
        /* One word of operands before the text, then "ab" and the end. */
        assert_eq!(words.get(6).copied(), Some(9));
        assert_eq!(
            words.get(7).copied(),
            Some(u32::from(b'a') | (u32::from(b'b') << 8))
        );
    }
}
