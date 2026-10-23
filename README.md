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
| functions discovered | 21,703 | 35,964 |
| executable words claimed | 859,593 (97.158%) | 2,675,920 (98.984%) |
| functions lifted to C | 21,703 (100%) | 35,964 (100%) |
| of those, import thunks | 241 | 189 |
| emitted C compiles | yes, all of it | yes, all of it |
| helper entries emitted beside them | 57 | 47 |
| reaches the renderer | yes | yes |
| issues draws with its own shaders | yes | yes |
| runs until stopped | yes | yes |
| shows what it copies out | yes | not yet looked at |

Both titles lift completely, both compile, both reach the renderer and draw
with their own translated shaders, and both keep running until they are
stopped. Title A copies its frames out of the graphics hardware and hands them
to the display, and those copies are what is shown. They are not yet the right
picture: the shapes a title draws arrive, but what it draws into, what it reads
back as pictures and how it composes a frame are only partly modelled.

Whether a frame is right is not measured by anything here. The runtime reports
what it drew, and `XENOLITH_FRAMES` writes out what was shown so it can be
looked at.

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

- **Play a game.** Title A runs and shows frames, but they are not yet the
  picture the title drew.
- **Reproduce a console's settings.** A privilege, the video cable, two display
  settings, the region, the language and a signed in profile are answered from
  what a title's own startup requires, because what a console was set to is not
  a property of the title. Every such answer is reported as an assumption when
  the run starts.
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
