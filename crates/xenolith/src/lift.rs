//! The lift subcommand.
//!
//! Emits C for the functions analysis found, alongside the interface that C is
//! written against. Some functions will not lift, which is the expected outcome
//! rather than a failure, so the report is the deliverable and the subcommand
//! succeeds either way.
//!
//! What it does not do is claim the result can run. The interface says what the
//! emitted code needs; nothing here provides it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use clap::Parser;
use miette::{Context, IntoDiagnostic, Result, miette};
use xenolith_analysis::analyze;
use xenolith_lift::{
    COMMAND_STREAM_SOURCE, DRAWING_SOURCE, RENDERER_SOURCE, RUNTIME_HEADER, RUNTIME_SOURCE,
    SOUND_SOURCE, Unlifted, declaration_of, lift, name_of,
};

use crate::input::{Source, number};

/// Bytes of C a translation unit holds before the next function starts a new
/// one.
///
/// A retail title emits hundreds of megabytes, which as one unit takes minutes
/// to compile and cannot be built in parallel at all, since a translation unit
/// is the only unit of parallelism a C build has.
const DEFAULT_PART_SIZE: u64 = 4 * 1024 * 1024;

/// Header the emitted units include, holding every declaration.
const DECLARATIONS: &str = "lifted.h";

/// Arguments of the lift subcommand.
#[derive(Debug, Parser)]
pub(crate) struct Args {
    /// Where the image comes from.
    #[command(flatten)]
    pub(crate) source: Source,

    /// Directory to write the emitted C and the runtime header into.
    #[arg(long, value_name = "PATH")]
    pub(crate) out: PathBuf,

    /// Bytes of C per translation unit, before the next function starts one.
    #[arg(long, value_name = "BYTES")]
    pub(crate) part_size: Option<String>,

    /// List the instructions that stopped functions, most blocking first.
    #[arg(long)]
    pub(crate) blockers: bool,

    /// List every function that was not lifted, with what stopped it.
    #[arg(long)]
    pub(crate) unlifted: bool,

    /// Treat an address as a function, for one a built program reached and
    /// discovery did not.
    ///
    /// A program that dispatches to an address nothing claimed reports where it
    /// was going. Some of those are functions that leave nothing to have found
    /// them by: no prologue, and named only from a table of pointers that
    /// cannot be told apart from any other run of numbers. Reaching one is the
    /// evidence, and this is how to give it back.
    #[arg(long = "root", value_name = "ADDRESS")]
    pub(crate) roots: Vec<String>,

    /// Read addresses to treat as functions from a file, one per line.
    ///
    /// A run of a built program with `XENOLITH_TRACE_DISPATCH` set reports
    /// every address it wanted and could not reach. Feeding that back is how
    /// one run's worth of evidence is used at once.
    #[arg(long = "roots-from", value_name = "PATH")]
    pub(crate) roots_from: Option<PathBuf>,
}

/// Returns a percentage with three decimal places, avoiding a division by zero.
fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "0.000".to_owned();
    }
    format!(
        "{}.{:03}",
        part * 100 / whole,
        (part * 100_000 / whole) % 1000
    )
}

/// Collects emitted functions into translation units of a bounded size.
///
/// A unit is written as soon as it passes its budget rather than at the end, so
/// that emitting a title holds one unit in memory rather than the whole
/// program.
struct Units {
    /// Directory the units are written into.
    directory: PathBuf,
    /// Bytes a unit reaches before the next function starts a new one.
    budget: u64,
    /// Text of the unit being filled, empty when no unit is open.
    text: String,
    /// Address of the first function in the unit being filled.
    first: u32,
    /// Address of the last function added to the unit being filled.
    last: u32,
    /// Size of each unit written so far, in the order they were written.
    written: Vec<u64>,
}

impl Units {
    /// Starts collecting into a directory.
    fn new(directory: &Path, budget: u64) -> Self {
        Self {
            directory: directory.to_path_buf(),
            budget,
            text: String::new(),
            first: 0,
            last: 0,
            written: Vec::new(),
        }
    }

    /// Adds one function's code, writing the open unit first when it is full.
    ///
    /// The budget is checked between functions, so a unit ends slightly over it
    /// rather than part way through a body.
    fn push(&mut self, address: u32, code: &str) -> Result<()> {
        if self.text.is_empty() {
            self.first = address;
        } else if self.text.len() as u64 >= self.budget {
            self.write()?;
            self.first = address;
        }

        self.last = address;
        self.text.push_str(code);
        self.text.push('\n');
        Ok(())
    }

    /// Writes the open unit, naming it after the addresses it covers.
    fn write(&mut self) -> Result<()> {
        if self.text.is_empty() {
            return Ok(());
        }

        let name = format!("lifted.{:08x}-{:08x}.c", self.first, self.last);
        let path = self.directory.join(&name);
        let source = format!(
            "/* Emitted by xenolith: functions {:#010x} through {:#010x}. */\n\n\
             #include \"{DECLARATIONS}\"\n\n{}",
            self.first, self.last, self.text
        );

        std::fs::write(&path, &source)
            .into_diagnostic()
            .wrap_err_with(|| format!("writing {}", path.display()))?;

        self.written.push(source.len() as u64);
        self.text.clear();
        Ok(())
    }

    /// Writes whatever is still open and reports the sizes written.
    fn finish(mut self) -> Result<Vec<u64>> {
        self.write()?;
        Ok(self.written)
    }
}

/// Removes units a previous run wrote into the same directory.
///
/// Only the emitted naming is matched. A unit left behind by a run that split
/// differently would define the same functions a second time, so leaving it
/// would produce a directory that cannot be built.
fn clear_units(directory: &Path) -> Result<usize> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Ok(0);
    };

    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let named = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("lifted."));
        if named && path.extension().is_some_and(|extension| extension == "c") {
            std::fs::remove_file(entry.path())
                .into_diagnostic()
                .wrap_err_with(|| format!("removing {}", entry.path().display()))?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Writes one of the files the build needs.
fn write_into(directory: &Path, name: &str, text: &str) -> Result<()> {
    let path = directory.join(name);
    std::fs::write(&path, text)
        .into_diagnostic()
        .wrap_err_with(|| format!("writing {}", path.display()))
}

/// Writes the decoded image, which the runtime loads rather than compiling in.
fn write_bytes_into(directory: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let path = directory.join(name);
    std::fs::write(&path, bytes)
        .into_diagnostic()
        .wrap_err_with(|| format!("writing {}", path.display()))
}

/// The program that boots the runtime and enters the guest.
///
/// It takes an address so that a caller can enter anywhere rather than only at
/// the entry point, which is what running one function of a title against
/// emulated hardware needs.
const BOOT_PROGRAM: &str = r#"/* Emitted by xenolith: boot the runtime and enter the guest.
 *
 * This links and it does not play a game. The first import the guest reaches
 * ends it, which is an accurate account of how far the translation has got.
 */

#include "lifted.h"

#include <stdio.h>
#include <stdlib.h>

int main(int count, char **arguments) {
    const char *image = count > 1 ? arguments[1] : "image.bin";
    uint32_t address = ENTRY_POINT;
    if (count > 2) {
        address = (uint32_t)strtoul(arguments[2], 0, 0);
    }
    if (address == 0) {
        fprintf(stderr, "xenolith: no entry point was recorded, name one\n");
        return 1;
    }

    uint8_t *base = xenolith_boot(image, LOAD_ADDRESS);
    if (base == 0) {
        return 1;
    }

    /* What kernel build the image was made against, which a title's device
     * layout depends on. The container carries it beside the import libraries,
     * and nought says it did not. */
    xenolith_built_against(BUILT_AGAINST);

    /* What the title is allowed to do, from the system flags word its container
     * carries. A title asks whether it holds a numbered privilege before it
     * does the thing, and one told it holds none shuts itself down. */
    xenolith_privileges(PRIVILEGES);

    xenolith_function entered = xenolith_lookup(address);
    if (entered == 0) {
        fprintf(stderr, "xenolith: %#010x is not a lifted function\n", address);
        return 1;
    }

    static xenolith_context state;

    /* A title expects to have been given a stack. Left at zero the first frame
     * subtracts from it and wraps to the top of the space, which works only
     * because all of it is mapped, and puts every frame somewhere nothing
     * chose. The linkage area a caller writes sits above the pointer, so the
     * pointer starts below the top rather than at it. */
    state.r[1] = XENOLITH_STACK_TOP - 64;

    /* And the block it reaches the kernel's own counters through. A title that
     * finds nothing there reads processor nought on every thread, and its vector
     * code picks a slot from that number and steps a counter in it. */
    state.r[13] = xenolith_processor_block(base);

    entered(&state, base);
    return 0;
}
"#;

/// Returns that program with the addresses this title uses.
fn main_program(
    base: u32,
    entry: Option<u32>,
    built_against: u32,
    privileges: Option<u32>,
) -> String {
    BOOT_PROGRAM
        .replace(
            "ENTRY_POINT",
            &entry.map_or_else(|| "0".to_owned(), |address| format!("{address:#010x}u")),
        )
        .replace("LOAD_ADDRESS", &format!("{base:#010x}u"))
        .replace("BUILT_AGAINST", &format!("{built_against:#010x}u"))
        .replace(
            "PRIVILEGES",
            &privileges.map_or_else(|| "0".to_owned(), |flags| format!("{flags:#010x}u")),
        )
}

/// Writes everything the build needs besides the units themselves.
///
/// The traps, the table, and the image are all specific to one title, so they
/// are written here rather than shipped, and the runtime that reads them is
/// shipped rather than written here.
fn write_support(
    out: &Path,
    image: &xenolith_xex::Image,
    referenced: &BTreeSet<u32>,
    emitted: &BTreeSet<u32>,
    built_against: u32,
    privileges: Option<u32>,
) -> Result<()> {
    // Everything the emitted code names but did not get a body becomes a trap,
    // so the program links and says where it went rather than walking past the
    // thing that is missing.
    let missing: Vec<u32> = referenced.difference(emitted).copied().collect();
    let mut traps = String::from(
        "/* Emitted by xenolith: the addresses that could not be lifted.\n\
         \x20*\n\
         \x20* Each is defined rather than left out so that the program links, and each\n\
         \x20* traps rather than returning so that reaching one is reported instead of\n\
         \x20* being walked past.\n\
         \x20*/\n\n#include \"lifted.h\"\n\n",
    );
    for address in &missing {
        let _ = writeln!(
            traps,
            "void {}(xenolith_context *ctx, uint8_t *base) {{ xenolith_unlifted(ctx, base, {address:#010x}u); }}",
            name_of(*address)
        );
    }
    write_into(out, "unlifted.c", &traps)?;

    // Only the lift knows which addresses became functions, so the table an
    // indirect branch resolves against is written here.
    let mut table = String::from(
        "/* Emitted by xenolith: every address that became a function.\n\
         \x20*\n\
         \x20* Sorted, so an address is found by halving rather than by walking.\n\
         \x20*/\n\n#include \"lifted.h\"\n\n         typedef struct entry {\n    uint32_t address;\n    xenolith_function function;\n} entry;\n\n         static const entry entries[] = {\n",
    );
    for address in emitted {
        let _ = writeln!(table, "    {{ {address:#010x}u, {} }},", name_of(*address));
    }
    table.push_str(
        "};\n\n         xenolith_function xenolith_lookup(uint32_t address) {\n         \x20   size_t low = 0, high = sizeof entries / sizeof *entries;\n         \x20   while (low < high) {\n         \x20       size_t middle = low + (high - low) / 2;\n         \x20       if (entries[middle].address == address) { return entries[middle].function; }\n         \x20       if (entries[middle].address < address) { low = middle + 1; } else { high = middle; }\n         \x20   }\n         \x20   return 0;\n         }\n",
    );
    write_into(out, "table.c", &table)?;

    // The runtime loads the image rather than having it compiled in, since a
    // title is tens of megabytes and a C array of it would be neither quick to
    // build nor readable.
    write_bytes_into(out, "image.bin", image.bytes())?;
    write_into(
        out,
        "main.c",
        &main_program(
            image.base_address(),
            image.entry_point(),
            built_against,
            privileges,
        ),
    )?;

    Ok(())
}

/// The makefile that builds what was emitted.
///
/// It links now rather than stopping at an archive. A unit compiling says it is
/// well formed on its own; only a link says the units agree with each other
/// about what exists and that nothing is defined twice.
const BUILD_FILE: &str = r"# Emitted by xenolith. Run with -j to build the units in parallel.
#
# This links into a program. That program will not play a game: nothing here
# services an import, so the first one the guest reaches ends it. What linking
# proves is that the emitted code is complete, which compiling a unit at a time
# does not.

CC ?= cc
CFLAGS ?= -O2 -Wall -Wextra -Wno-infinite-recursion
LDLIBS ?= -lm -lpthread

# Where a title's drawing goes, if this machine has anywhere to put it.
#
# Built against what is installed rather than required: without a graphics
# driver to build against, everything else still builds and the program says it
# has nowhere to draw rather than failing to link.
DRAWING := $(shell pkg-config --exists vulkan 2>/dev/null && echo yes)
ifeq ($(DRAWING),yes)
CFLAGS += -DXENOLITH_WITH_VULKAN
LDLIBS += -lvulkan
endif

# A window to show it in, on the same terms. Either way of asking a desktop for
# one is a window, so this is asked of both together; which one is used is asked
# of each below, because a host can have one, the other, or both.
SHOWING := $(shell pkg-config --exists x11 2>/dev/null && echo yes)
TALKING := $(shell pkg-config --exists wayland-client wayland-protocols 2>/dev/null \
	&& command -v wayland-scanner >/dev/null 2>&1 && echo yes)
ifneq ($(strip $(SHOWING) $(TALKING)),)
CFLAGS += -DXENOLITH_WITH_WINDOW
endif

ifeq ($(SHOWING),yes)
CFLAGS += -DXENOLITH_WITH_X11
LDLIBS += -lX11
endif

# The part that says how a window is placed and titled is a description rather
# than a library, turned into code by the tool that comes with it.
ifeq ($(TALKING),yes)
SHELL_XML := $(shell pkg-config --variable=pkgdatadir wayland-protocols)/stable/xdg-shell/xdg-shell.xml
CFLAGS += -DXENOLITH_WITH_WAYLAND $(shell pkg-config --cflags wayland-client)
LDLIBS += $(shell pkg-config --libs wayland-client)
MADE := xdg-shell-client-protocol.h xdg-shell-protocol.c
endif

SOURCES := $(wildcard lifted.*.c) unlifted.c table.c shaders.c xenolith.c vulkan.c main.c
ifeq ($(TALKING),yes)
SOURCES += xdg-shell-protocol.c
endif
OBJECTS := $(SOURCES:.c=.o)

all: lifted

lifted: $(OBJECTS)
	$(CC) $(CFLAGS) -o $@ $(OBJECTS) $(LDLIBS)

# Saying a shader while a title runs, for the ones it loads from the data beside
# its executable rather than out of the executable itself. Reading the
# executable cannot find one of those, so this answers for them.
#
# Built against what is beside the emitted code rather than required: without it
# everything else still builds, and a title handing over a shader that was not
# found simply says so.
SAYING := $(wildcard libxenolith_shader.a)
ifneq ($(SAYING),)
CFLAGS += -DXENOLITH_CAN_SAY_A_SHADER
LDLIBS += $(SAYING)
endif

# The description turned into code, before anything that reads it is built.
xdg-shell-client-protocol.h:
	wayland-scanner client-header $(SHELL_XML) $@

xdg-shell-protocol.c:
	wayland-scanner private-code $(SHELL_XML) $@

%.o: %.c lifted.h xenolith.h $(MADE)
	$(CC) $(CFLAGS) -c -o $@ $<

clean:
	rm -f $(OBJECTS) $(MADE) lifted

.PHONY: all clean
";

/// Writes the runtime the emitted code is built against.
///
/// Four files rather than one, because the command stream and the sound are
/// subjects of their own and the runtime includes them. They go beside the
/// runtime because that is where it looks for them.
fn write_the_runtime(out: &Path) -> Result<()> {
    for (name, text) in [
        ("xenolith.h", RUNTIME_HEADER),
        ("xenolith.c", RUNTIME_SOURCE),
        ("command_stream.c", COMMAND_STREAM_SOURCE),
        ("sound.c", SOUND_SOURCE),
        ("vulkan.c", RENDERER_SOURCE),
        ("drawing.c", DRAWING_SOURCE),
    ] {
        let path = out.join(name);
        std::fs::write(&path, text)
            .into_diagnostic()
            .wrap_err_with(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// Puts the shader library beside the emitted code, where the makefile looks.
///
/// A title loads some shaders from the data beside its executable rather than
/// out of the executable, and reading the executable cannot find one of those.
/// The library answers for them, and it is copied rather than required: without
/// it everything else still builds, and a title handing over one of those
/// shaders says so instead of drawing nothing.
///
/// The path is the one the build that made this tool left it at, which is the
/// only place it can be: nothing else knows where a target directory is.
fn write_the_shader_library(out: &Path) {
    let Some(beside) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    else {
        return;
    };
    let library = beside.join("libxenolith_shader.a");
    if !library.is_file() {
        eprintln!(
            "note: {} is not there, so a shader a title loads while it runs will not be said",
            library.display()
        );
        return;
    }
    if let Err(error) = std::fs::copy(&library, out.join("libxenolith_shader.a")) {
        eprintln!(
            "note: {} could not be copied, so a shader a title loads while it runs will not be said: {error}",
            library.display()
        );
    }
}

/// Makes the output directory and writes everything that does not depend on
/// which functions were lifted.
///
/// Kept apart from the lift itself because none of it looks at the analysis:
/// the runtime, the shader table and the empty units are the same whatever the
/// title turns out to hold.
fn prepare_the_output(out: &Path, image: &xenolith_xex::Image) -> Result<()> {
    std::fs::create_dir_all(out)
        .into_diagnostic()
        .wrap_err_with(|| format!("making {}", out.display()))?;
    clear_units(out)?;
    write_the_runtime(out)?;
    write_shaders(out, image)
}

/// Runs the lift subcommand.
///
/// # Errors
///
/// Returns an error when the input cannot be read, decoded, or analyzed, or
/// when the output directory cannot be written. Functions that could not be
/// lifted are reported rather than treated as a failure.
pub(crate) fn run(args: &Args) -> Result<()> {
    let budget = match &args.part_size {
        Some(text) => u64::from(number(text)?),
        None => DEFAULT_PART_SIZE,
    };
    if budget == 0 {
        return Err(miette!(
            help = "a unit has to hold at least one function",
            "a part size of zero would write a unit per byte"
        ));
    }

    let loaded = args.source.load()?;
    let built_against = loaded.built_against;
    let image = loaded.image;
    let imports = loaded.imports?;
    let program = analyze(&image, &named_roots(args)?);

    if program.function_count() == 0 {
        return Err(miette!(
            help = "lifting needs functions, and analysis found none",
            "nothing was discovered to lift"
        ));
    }

    prepare_the_output(&args.out, &image)?;

    let mut lifted = 0u64;
    let mut thunks = 0u64;
    let mut refused = Vec::new();
    let mut blocking: BTreeMap<&str, u64> = BTreeMap::new();
    let mut referenced: BTreeSet<u32> = BTreeSet::new();
    let mut emitted: BTreeSet<u32> = BTreeSet::new();
    let mut units = Units::new(&args.out, budget);

    for function in program.functions() {
        referenced.insert(function.start);
        match lift(&image, function, &imports) {
            Ok(result) => {
                lifted += 1;
                if imports.contains_key(&function.start) {
                    thunks += 1;
                }
                referenced.extend(result.calls);
                emitted.insert(function.start);
                units.push(function.start, &result.code)?;
            }
            Err(unlifted) => {
                *blocking.entry(unlifted.mnemonic).or_default() += 1;
                refused.push(unlifted);
            }
        }
    }

    let entries = lift_helper_entries(
        &image,
        &program,
        &imports,
        &mut referenced,
        &mut emitted,
        &mut units,
        &mut blocking,
        &mut refused,
    )?;

    let sizes = units.finish()?;

    // Everything the emitted code names is declared, which is more than the
    // discovered functions: a call into a register save helper lands partway
    // through one, and discovery does not claim those. Which of them are named
    // is only known once every function has been lifted, so the header is
    // written after the units that include it.
    let mut header = String::from(
        "/* Emitted by xenolith. Every function is written under the\n\
         \x20* instructions it came from, so the two can be read against each\n\
         \x20* other.\n\
         \x20*\n\
         \x20* A guest function that recurses on every path is a statement about\n\
         \x20* the program that was translated rather than about the\n\
         \x20* translation, so a compiler warning that reads intent has none to\n\
         \x20* read here. Build with -Wno-infinite-recursion.\n\
         \x20*/\n\n\
         #ifndef XENOLITH_LIFTED_H\n\
         #define XENOLITH_LIFTED_H\n\n\
         #include \"xenolith.h\"\n\n",
    );
    for address in &referenced {
        header.push_str(&declaration_of(*address));
    }
    header.push_str("\n#endif\n");

    let path = args.out.join(DECLARATIONS);
    std::fs::write(&path, &header)
        .into_diagnostic()
        .wrap_err_with(|| format!("writing {}", path.display()))?;

    write_support(
        &args.out,
        &image,
        &referenced,
        &emitted,
        built_against.unwrap_or(0),
        loaded.privileges,
    )?;
    write_the_shader_library(&args.out);
    write_into(&args.out, "Makefile", BUILD_FILE)?;

    report(
        args,
        &Outcome {
            lifted,
            entries,
            thunks,
            refused,
            blocking,
            declarations: referenced.len(),
            sizes,
        },
        built_against,
    );

    Ok(())
}

/// Emits a body for every helper entry the output names, returning how many.
///
/// A call into a register save helper lands partway through one, and discovery
/// does not claim those: a call to a helper is a call to the helper rather than
/// to a function beginning wherever the caller entered it. Left without a body
/// they become traps, and since better than a third of a title's functions save
/// registers this way, the program stops on the first thing it does.
///
/// These are counted apart from the discovered functions, because they are
/// entries into one rather than functions the analysis found.
#[expect(clippy::too_many_arguments, reason = "one pass over what lifting left")]
fn lift_helper_entries<'a>(
    image: &xenolith_xex::Image,
    program: &'a xenolith_analysis::Program,
    imports: &xenolith_lift::Imports,
    referenced: &mut BTreeSet<u32>,
    emitted: &mut BTreeSet<u32>,
    units: &mut Units,
    blocking: &mut BTreeMap<&'a str, u64>,
    refused: &mut Vec<Unlifted>,
) -> Result<u64> {
    let mut entries = 0u64;

    for address in referenced.difference(emitted).copied().collect::<Vec<_>>() {
        let Some(function) = xenolith_analysis::helper_entry(image, program.helpers(), address)
        else {
            continue;
        };
        match lift(image, &function, imports) {
            Ok(result) => {
                entries += 1;
                referenced.extend(result.calls);
                emitted.insert(address);
                units.push(address, &result.code)?;
            }
            Err(unlifted) => {
                *blocking.entry(unlifted.mnemonic).or_default() += 1;
                refused.push(unlifted);
            }
        }
    }

    Ok(entries)
}

/// Returns the addresses named as roots on the command line.
///
/// A program that dispatches to an address nothing claimed reports where it was
/// going, and some of those leave nothing to have found them by. Reaching one is
/// the evidence, and this is how it is given back.
fn named_roots(args: &Args) -> Result<Vec<u32>> {
    let mut roots: Vec<u32> = args
        .roots
        .iter()
        .map(|text| number(text))
        .collect::<Result<Vec<_>>>()?;

    if let Some(path) = &args.roots_from {
        let text = std::fs::read_to_string(path)
            .into_diagnostic()
            .wrap_err_with(|| format!("reading roots from {}", path.display()))?;
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if !line.is_empty() {
                roots.push(number(line)?);
            }
        }
    }

    roots.sort_unstable();
    roots.dedup();
    Ok(roots)
}

/// What lifting produced, in the form the report reads it.
struct Outcome<'a> {
    /// How many functions were emitted.
    lifted: u64,
    /// How many helper entries were emitted beside them.
    entries: u64,
    /// How many of those were import thunks rather than translated code.
    thunks: u64,
    /// The functions that were not emitted, and what stopped each.
    refused: Vec<Unlifted>,
    /// How many functions each instruction stopped.
    blocking: BTreeMap<&'a str, u64>,
    /// How many functions the emitted code declares.
    declarations: usize,
    /// Size of each translation unit written.
    sizes: Vec<u64>,
}

/// Prints what lifting produced.
fn report(args: &Args, outcome: &Outcome<'_>, built_against: Option<u32>) {
    let total = outcome.lifted + outcome.refused.len() as u64;
    println!("functions          {total:>10}");
    println!(
        "  lifted           {:>10}  ({} percent)",
        outcome.lifted,
        percent(outcome.lifted, total)
    );
    println!("    import thunks  {:>10}", outcome.thunks);
    println!("  not lifted       {:>10}", outcome.refused.len());
    println!("helper entries     {:>10}", outcome.entries);
    println!("declarations       {:>10}", outcome.declarations);
    println!("units              {:>10}", outcome.sizes.len());
    println!(
        "  largest          {:>10} bytes",
        outcome.sizes.iter().copied().max().unwrap_or(0)
    );
    match built_against {
        Some(build) => println!("kernel build      {build:#010x}"),
        None => println!("kernel build      unknown, the container did not say"),
    }

    println!("\nwritten to {}", args.out.display());
    println!("build it with make -j8, which links into a program");
    println!("give -j a number: a job for each unit is more than a machine has");
    println!("run it with the image beside it to start at the recorded entry point");

    if args.blockers {
        // The instruction that appears most is not the one to model next. The
        // one blocking the most functions is, and they are not the same list.
        // An import thunk appears in neither, since it lifts.
        let mut ranked: Vec<(&str, u64)> = outcome
            .blocking
            .iter()
            .map(|(mnemonic, count)| (*mnemonic, *count))
            .collect();
        ranked.sort_by_key(|(_, count)| std::cmp::Reverse(*count));

        println!("\ninstructions blocking the most functions");
        for (mnemonic, count) in ranked.iter().take(20) {
            println!("  {mnemonic:<14} {count:>8}");
        }
    }

    if args.unlifted {
        println!("\nfunctions that were not lifted");
        for unlifted in &outcome.refused {
            println!(
                "  {:#010x}  stopped at {:#010x} on {}",
                unlifted.function, unlifted.address, unlifted.mnemonic
            );
        }
    }
}

/// Writes the shaders an image holds, said in a host graphics driver's terms.
///
/// A title hands a shader to the hardware while it runs, out of its own data, so
/// what the runtime gets is words rather than a name. Every shader the image
/// holds is translated here and kept beside the sum of the words it was made
/// from, so the runtime can find it again by what it was given.
///
/// A shader this has no reading of is left out rather than guessed at, and the
/// runtime says so when a title hands it one that is not here.
fn write_shaders(out: &Path, image: &xenolith_xex::Image) -> Result<()> {
    let words: Vec<u32> = image
        .bytes()
        .chunks_exact(4)
        .filter_map(|four| four.try_into().ok())
        .map(u32::from_be_bytes)
        .collect();

    let mut file = String::from(
        "/* Emitted by xenolith: the shaders an image holds, in a host's terms.\n\
         \x20*\n\
         \x20* Each is kept beside the sum of the words it was made from, because what\n\
         \x20* a title hands over is words rather than a name.\n\
         \x20*/\n\n#include \"xenolith.h\"\n\n",
    );

    /* A shader is one kind or the other, and which it is a title says when it
     * hands it over rather than the words saying so. Both readings are written
     * out, and the runtime takes the one a title asked for. The reading that is
     * wrong for a shader is usually still a module, so what rules one out is a
     * reading being refused rather than anything here choosing. */
    let mut entries = Vec::new();
    let mut written = 0usize;
    for found in xenolith_shader::scan(&words) {
        let Some(body) = words.get(found.at..found.at.saturating_add(found.words)) else {
            continue;
        };

        for (number, kind) in [
            (0u32, xenolith_shader::Kind::Vertex),
            (1u32, xenolith_shader::Kind::Pixel),
        ] {
            let Ok(translated) = xenolith_shader::translate(kind, body) else {
                continue;
            };

            let which = written;
            written += 1;

            write_one_shader(&mut file, which, &translated);

            entries.push((
                number,
                xenolith_shader::sum_of(body),
                body.len(),
                which,
                translated.words.len(),
                translated.streams.len(),
                translated.pictures.len(),
            ));
        }
    }

    write_shader_table(&mut file, &entries);

    write_into(out, "shaders.c", &file)
}

/// Writes one translated shader: the module itself, where it goes for its
/// vertices, and which pictures it looks at.
fn write_one_shader(file: &mut String, which: usize, translated: &xenolith_shader::Translated) {
    let _ = writeln!(file, "static const uint32_t said_{which}[] = {{");
    for word in &translated.words {
        let _ = writeln!(file, "    {word:#010x}u,");
    }
    file.push_str("};\n\n");

    if !translated.streams.is_empty() {
        let _ = writeln!(file, "static const uint32_t reads_{which}[] = {{");
        for stream in &translated.streams {
            let _ = writeln!(
                file,
                "    {}u, {}u, {}u, {}u,",
                stream.slot, stream.stride, stream.offset, stream.words
            );
        }
        file.push_str("};\n\n");
    }

    if !translated.pictures.is_empty() {
        let _ = writeln!(file, "static const uint32_t looks_{which}[] = {{");
        for place in &translated.pictures {
            let _ = writeln!(file, "    {place}u,");
        }
        file.push_str("};\n\n");
    }
}

/// Writes the table the runtime finds a shader again by, and the three ways it
/// is asked for.
fn write_shader_table(
    file: &mut String,
    entries: &[(u32, u32, usize, usize, usize, usize, usize)],
) {
    file.push_str(concat!(
        "typedef struct said {\n",
        "    uint32_t kind;\n",
        "    uint32_t sum;\n",
        "    uint32_t words;\n",
        "    const uint32_t *said;\n",
        "    uint32_t length;\n",
        "    const uint32_t *reads;\n",
        "    uint32_t streams;\n",
        "    const uint32_t *looks;\n",
        "    uint32_t pictures;\n",
        "} said;\n\n",
        "static const said everything[] = {\n",
    ));
    for (kind, sum, words, which, length, streams, pictures) in entries {
        let reads = if *streams == 0 {
            String::from("0")
        } else {
            format!("reads_{which}")
        };
        let looks = if *pictures == 0 {
            String::from("0")
        } else {
            format!("looks_{which}")
        };
        let _ = writeln!(
            file,
            "    {{ {kind}u, {sum:#010x}u, {words}u, said_{which}, {length}u, {reads}, {streams}u, {looks}, {pictures}u }},"
        );
    }
    file.push_str("};\n\n");

    /* The count is a named constant rather than a sizeof in each loop. An empty
     * table is the common case, and a title whose image holds no shader that
     * translated is built with one, where the sizeof folded to zero and the loop
     * condition became an unsigned comparison against zero. Held to warnings as
     * errors, that stopped the build of any title with no translated shader. */
    file.push_str(
        "static const size_t everything_there = sizeof everything / sizeof *everything;\n\n",
    );

    file.push_str(concat!(
        "const uint32_t *xenolith_said(uint32_t kind, uint32_t sum, uint32_t words,\n",
        "                              uint32_t *length) {\n",
        "    for (size_t at = 0; at < everything_there; at++) {\n",
        "        if (everything[at].kind == kind && everything[at].sum == sum\n",
        "            && everything[at].words == words) {\n",
        "            if (length != 0) { *length = everything[at].length; }\n",
        "            return everything[at].said;\n",
        "        }\n",
        "    }\n",
        "    return 0;\n",
        "}\n\n",
        "const uint32_t *xenolith_reads(uint32_t kind, uint32_t sum, uint32_t words,\n",
        "                               uint32_t *streams) {\n",
        "    for (size_t at = 0; at < everything_there; at++) {\n",
        "        if (everything[at].kind == kind && everything[at].sum == sum\n",
        "            && everything[at].words == words) {\n",
        "            if (streams != 0) { *streams = everything[at].streams; }\n",
        "            return everything[at].reads;\n",
        "        }\n",
        "    }\n",
        "    return 0;\n",
        "}\n\n",
        "const uint32_t *xenolith_looks_at(uint32_t kind, uint32_t sum, uint32_t words,\n",
        "                                  uint32_t *pictures) {\n",
        "    for (size_t at = 0; at < everything_there; at++) {\n",
        "        if (everything[at].kind == kind && everything[at].sum == sum\n",
        "            && everything[at].words == words) {\n",
        "            if (pictures != 0) { *pictures = everything[at].pictures; }\n",
        "            return everything[at].looks;\n",
        "        }\n",
        "    }\n",
        "    return 0;\n",
        "}\n",
    ));
}
