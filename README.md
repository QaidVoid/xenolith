# xenolith

A static recompiler for Xbox 360 games. It reads a XEX executable, finds every
function in it, translates the PowerPC code to C, and builds that C together
with a runtime into a native Linux program that runs the game.

There is no per game configuration. Function boundaries, jump tables, register
save helpers and imports are all recovered from the executable itself, and
anything that cannot be recovered is reported rather than guessed.

Linux only.

## Status

Two retail titles lift completely and compile without warnings. Both boot,
reach the renderer and draw with their own translated shaders.

| | title A | title B |
|---|---|---|
| functions discovered | 22,006 | 36,737 |
| executable words claimed | 868,194 (98.130%) | 2,678,143 (99.067%) |
| functions lifted to C | 22,006 (100%) | 36,737 (100%) |
| of those, import thunks | 241 | 189 |
| helper entries emitted beside them | 57 | 47 |
| emitted C compiles | yes | yes |

Title A is the one taken furthest. It plays its logos, opening film and menus,
runs the mission briefing and flies the first mission, with music and voices.
Some scenes still draw wrong, and it is not yet playable end to end.

## Quick start

You need a Rust toolchain, a C compiler, and the key material for the title
(see [Key material](docs/guide/keys.md)). Vulkan, a desktop session, PulseAudio
and libavcodec are picked up when present, for graphics, a window, sound and
music.

```sh
cargo build --release
target/release/xenolith lift path/to/default.xex --out /tmp/lifted
cd /tmp/lifted
make -j"$(nproc)"
./lifted
```

`lift` writes the translated C, the runtime, a `Makefile` and the decoded
image. Run the program from that directory, with the title's other files beside
it, since the game reads its data from there. Saves go in `content/`.

The key is read from `--key-file`, then `XENOLITH_XEX_KEY`, then
`~/.config/xenolith/xex.key`. No key and no game data is in this repository.

Controls: arrow keys for the pad, `z` and `x` for the face buttons, enter for
start.

## How it works

```
default.xex
  -> xenolith-xex       decrypt and decompress the container
  -> xenolith-ppc       decode PowerPC, AltiVec and VMX128 instructions
  -> xenolith-analysis  find functions, blocks and jump tables
  -> xenolith-lift      emit C for every function, plus the runtime
  -> xenolith-shader    translate the game's GPU shaders to SPIR-V
  -> cc                 one native program
```

The runtime is plain C that ships with the lift. It maps the guest address
space, services the kernel calls a game makes, runs guest threads as host
threads, replays the GPU command stream through Vulkan, decodes XMA audio
through libavcodec, and reads files beside the program.

## Repository layout

| path | what it holds |
|---|---|
| `crates/xenolith` | the `xenolith` command line tool |
| `crates/xenolith-xex` | the container reader |
| `crates/xenolith-ppc` | the instruction decoder |
| `crates/xenolith-analysis` | function and jump table discovery |
| `crates/xenolith-lift` | the C emitter, and the runtime in `runtime/` |
| `crates/xenolith-shader` | the shader translator |
| `docs/` | the documentation site |
| `scripts/` | the checks the project is gated on |

## Documentation

The full documentation lives in [`docs/`](docs/): a user guide, a walk through
each stage of the pipeline, how each stage is verified, and a reference for the
commands, environment variables and runtime interface. To browse it as a site:

```sh
cd docs && bun install && bun run dev
```

## Testing

```sh
scripts/check          # format, lints, tests, docs and text checks
scripts/check-oracles  # the comparisons that need a title, a cross toolchain or an emulator
```

`cargo test` passes with no game data present. Tests that need a real title,
a big endian PowerPC toolchain or an emulator read their paths from the
environment and are skipped without them. See
[How it is checked](docs/verification/index.md).

## Limitations

- **Not every scene renders correctly yet**, and some GPU features are missing.
  Shaders that use something unsupported are reported and their draws skipped.
- **Console settings are assumed.** The video cable, display mode, region,
  language, signed in profile and similar settings are not part of a game, so
  the runtime answers with what the game requires and reports each assumption
  when it starts.
- **Only what the two titles use is modelled.** A function containing an
  instruction the translator does not model is refused whole, never emitted with
  a hole in it.
- **A host C compiler is required to run the tests**, since most of them check
  the emitted C by compiling it.

## Licence

Dual licensed under MIT or Apache 2.0, at your option. See `LICENSE-MIT` and
`LICENSE-APACHE`.
