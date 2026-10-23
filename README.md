# xenolith

A static recompiler for Xbox 360 executables. It reads a XEX container, decodes
the PowerPC code inside it, works out where the functions are, and writes C that
compiles into a program.

Linux only, and deliberately so. There is no Windows support and none planned.

Fuller documentation lives in `docs/`, covering each stage of the pipeline, the
oracle each stage is checked against, and every subcommand. Build it with
`cd docs && bun install && bun run dev`.

## Status

This turns a retail game into C that compiles and runs its own startup. It does
not yet turn one into a game you can play.

Measured against two retail titles, with
`xenolith analyze <title>` and `xenolith lift <title> --out <dir>`:

| | title A | title B |
|---|---|---|
| functions discovered | 21,671 | 35,894 |
| executable words claimed | 859,562 (97.154%) | 2,675,836 (98.981%) |
| functions lifted to C | 21,671 (100%) | 35,894 (100%) |
| of those, import thunks | 241 | 189 |
| emitted C compiles | yes, all of it | yes, all of it |
| helper entries emitted beside them | 57 | 47 |
| reaches the renderer | yes | yes |
| issues draws with its own shaders | yes | yes |
| runs until stopped | no, it faults | yes |

Both titles lift completely, both compile, and both reach the renderer and draw
with their own translated shaders. What differs is what they do next: one takes a
fault path of its own and raises an exception, and the other keeps running until
it is stopped, drawing a fixed number of times and then waiting.

Neither shows a picture. Whether a frame is visible is not measured by anything
here, and the runtime reports what it drew rather than what was on screen.

## Building and running

```sh
cargo build --release
target/release/xenolith lift ../xen/corpus/ac6_extract/default.xex --out /tmp/lifted
cd /tmp/lifted && make -j16 && ./lifted
```

The lift writes C, the runtime it is written against, a `Makefile`, and the
decoded image beside it. `make` links the units into one program. Run it where
the image is.

A key is needed to decode an encrypted container. It is read from `--key-file`,
then `XENOLITH_XEX_KEY`, then `~/.config/xenolith/xex.key`. No key and no game
data is in this repository.

## What it does not do

- **Play a game.** Both titles stop in their own failure states before a frame
  is shown.
- **Reproduce a console's settings.** A privilege, the video cable and two
  display settings are answered from what a title's own startup requires,
  because what a console was set to is not a property of the title. Every such
  answer is reported as an assumption when the run starts.
- **Run without a host C compiler.** Most of the test suite checks the emitted C
  by compiling it, and a suite that skipped those would be comparing strings.
- **Model the whole instruction set.** What the two titles exercise is modelled.
  What they do not exercise is not, and a function holding an unmodelled
  instruction is refused whole rather than emitted with a hole in it.
- **Run its strongest oracle everywhere.** The differential against hardware
  needs a big endian ppc64 cross toolchain and an emulator. It runs in the gate
  when both are installed and says so when they are not.

## Licence

Dual licensed under MIT or Apache 2.0, at your option. See `LICENSE-MIT` and
`LICENSE-APACHE`.
