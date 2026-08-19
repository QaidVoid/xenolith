# Getting started

## Building

Nothing beyond a Rust toolchain is needed. Rust 1.85 or later.

```sh
git clone https://github.com/QaidVoid/xenolith
cd xenolith
cargo build --release
```

The binary lands at `target/release/xenolith`.

```sh
cargo test
```

The suite passes on a clean checkout with no game data present. Tests that need
a real title read a path from the environment and skip when it is unset, so a
green result there says less than it looks like. See
[Environment](/reference/environment) for what to set to run them for real.

## Looking at a container

`inspect` reads the headers only, so it works on an encrypted title with no key
to hand.

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

Everything past this point needs the image decoded, which for a retail title
needs a key. That is [its own page](/guide/keys).

## Reading some code

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

A word the decoder does not recognize is printed as `.long` with a question
mark, never as a plausible guess.

## Finding the functions

```sh
xenolith analyze default.xex
```

The report gives how many functions were discovered, how they were found, what
proportion of the executable words are claimed by one, and how many indirect
branches had their jump tables recovered. `--functions` and `--tables` list them.

## Writing C

```sh
xenolith lift default.xex --out ./lifted --blockers
make -C ./lifted -j
```

`lift` writes a directory: numbered translation units, a header holding every
declaration, the runtime interface header, and a makefile that builds them.

```
functions               27447
  lifted                26908  (98.036 percent)
    import thunks         156
  not lifted              539
declarations            27569
units                      88
  largest             6015615 bytes
```

`--blockers` ranks the instructions that stopped functions by how many functions
each blocked, which is the list to work from if you want to move the number.

The build stops at `liblifted.a` rather than a link, because nothing implements
the runtime the units call. That is covered in [Lifting a title](/guide/lifting).
