# Getting started

This page builds the tool and tours what it can tell you about an executable.
To go straight to playing a game, see [Playing a game](/guide/playing).

## What you need

| need | for |
|---|---|
| Rust 1.85 or later | building the tool (the repository pins a toolchain in `rust-toolchain.toml`) |
| a C compiler | building a lifted title, and most of the test suite |
| the static key | decoding a retail title, see [Key material](/guide/keys) |
| Vulkan and its loader | drawing |
| X11 or Wayland | a window to show what is drawn |
| `wayland-protocols` and `wayland-scanner` | the Wayland window |
| PulseAudio (or PipeWire's PulseAudio server), or ALSA | sound |
| libavcodec 58 or 60 | music and voices, which a title keeps as XMA |

Everything after the C compiler is optional. A lifted title builds without
it and reports, in one line each, what it is running without.

## Building

```sh
git clone https://github.com/QaidVoid/xenolith
cd xenolith
cargo build --release
```

The binary is `target/release/xenolith`. Building also leaves
`libxenolith_host.a` beside it, which a lift copies into its output. A built
game reads its executable from the player's copy with it, and translates the
shaders it loads from its data files while it runs.

```sh
cargo test
```

The suite passes on a clean checkout with no game data. Tests that need a real
title, a cross toolchain or an emulator are skipped unless their paths are set;
see [Environment](/reference/environment).

## Looking at a container

`inspect` reads the headers only, so it works on an encrypted title without a
key.

```sh
xenolith inspect default.xex
```

```
default.xex

  format            Xex2
  title id          0x4b4e07da
  media id          0x617138eb
  version           0.0.0.1 (base 0.0.0.1)
  disc              1 of 1

  file size         13.6 MiB
  base address      0x82000000
  image size        0x11c0000 (17.7 MiB)
  entry point       0x826d2238
  encryption        encrypted
  compression       basic (3 blocks)

  sections (4)
    0x82000000..0x82100000    1.0 MiB  rodata   r--
    0x82100000..0x82b50000   10.3 MiB  code     r-x
    0x82b50000..0x83190000    6.2 MiB  data     rw-
    0x83190000..0x831c0000  192.0 KiB  rodata   r--

  imports (2)
    xam.xex           124 imports   version 2.0.3424.0 (min 2.0.2858.0)
    xboxkrnl.exe      264 imports   version 2.0.3424.0 (min 2.0.2858.0)
```

## Reading code

```sh
xenolith disasm default.xex --start 0x82090000 --length 64
```

```
0x82090000  7d8802a6   mfspr r12, r8, r0
0x82090004  9181fff8   stw r12, -8(r1)
0x82090008  fbe1fff0   std r31, -16(r1)
0x8209000c  9421ffa0   stwu r1, -96(r1)
0x82090010  3d608205   lis r11, -32251
```

A word the decoder does not recognise is printed as `.long` with a question
mark, never as a plausible guess.

## Finding the functions

```sh
xenolith analyze default.xex
```

The report gives how many functions were found and how, what share of the
executable words they claim, and how many indirect branches had their jump
tables recovered. `--functions` and `--tables` list them.

## Lifting and running a title

[Playing a game](/guide/playing) walks through it step by step, from the
packages to install to where the saves go. In short:

```sh
xenolith lift ~/games/example/default.xex --out ~/games/example-build
make -C ~/games/example-build -j"$(nproc)"
~/games/example-build/example-game
```

The lift names the program after the game, links the game's folder where the
program looks for it, and ends by printing the two commands that follow it.
[Lifting a title](/guide/lifting) covers the output in more detail.
