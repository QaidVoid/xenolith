# Lifting a title

`lift` is the subcommand that produces C. This page covers what it writes, how
to build and run the result, and how to read the report. For a walk from a
fresh machine to a running game, see [Playing a game](/guide/playing).

The examples here lift with `--id game`, so the program is called `game`.

## The output

```sh
xenolith lift default.xex --out ./build --id game
```

The output directory holds everything needed to build the title on its own:

| file | what it is |
|---|---|
| `lifted.<first>-<last>.c` | a translation unit holding the functions in that address range |
| `lifted.h` | every function declaration |
| `table.c` | every lifted function, for indirect branches to resolve against |
| `unlifted.c` | a trap for every address that could not be lifted |
| `shaders.c` | every shader found in the image, translated to SPIR-V |
| `main.c` | boots the runtime and enters the title |
| `xenolith.h`, `xenolith.c` and the rest | the runtime |
| `libxenolith_host.a` | what the program reads the game and says shaders with, copied from beside the tool |
| `Makefile` | builds and links everything into the program, named by the id |

Nothing of the game's own is written. A lift from a bare image given with
`--raw` writes `image.bin` as well, since there is no copy of the game for the
program to read.

[What a lift writes](/reference/output) lists every file and the build options.
Units a previous lift left in the same directory are removed first.

### Why it is split

A retail title emits hundreds of megabytes of C. As one file no compiler builds
it in reasonable time, and a C build only parallelises across files. The output
is split at a byte budget (4 MiB by default, `--part-size` to change it) rather
than a function count, because function sizes vary by orders of magnitude. A
function is never split across units.

## Naming the program

A program is named for its game. The lift reads the name the game gives itself
out of a resource in its executable, the same name the console's dashboard
shows, and makes an id from it: its letters and digits in lower case, joined by
dashes.

| | titles | default | to choose it |
|---|---|---|---|
| name | the window | the game's own name, or `xenolith` where it carries none | `--name` |
| id | the program file, the desktop's grouping of its windows, and the sound mixer's list | made from the name | `--id` |

An id has to do as a file name and as a desktop's id at once, so it is lower
case letters, digits, dots, dashes and underscores, starting with a letter or a
digit. The lift refuses any other.

## Where the program finds the game

The program carries the translation and nothing of the game's own. When it runs
it reads the game from the player's copy: the folder `games/<id>/` in
xenolith's directory of the user's data, `~/.local/share/xenolith` unless moved
(see [Environment](/reference/environment)), or whatever folder `XENOLITH_GAME`
names. It decodes `default.xex` there with the player's key, and refuses a copy
whose image is not the one it was lifted from, which is what another version or
region of the game comes to.

The lift links the folder it was given into that place, so a program runs as
soon as it is built. Whatever is there already is left alone. The game's files
are read from the folder and never written: a title writes only what it saves,
as it does on a console whose disc cannot be written.

## Building and running

```sh
make -C ./build -j"$(nproc)"
./build/game
```

The program finds the game itself, so it can be run from anywhere. The makefile
builds at `-O2` with `-Wall -Wextra` and picks up Vulkan, X11, Wayland and the
host library when they are present. The link matters as much
as the compile: only the link shows that tens of thousands of functions agree
about what exists and that nothing is defined twice.

The program enters the title at its recorded entry point. Given an image and an
address it enters there instead, which is useful for running one function:

```sh
./game image.bin 0x82090000
```

## What the emitted C looks like

Each function becomes a C function named from its address. Each basic block is
a label and each edge a branch to one. Every instruction is emitted under its
own disassembly, so the two can be read side by side.

```c
/* 0x82090000 */
void sub_82090000(xenolith_context *ctx, uint8_t *base) {
    uint32_t address;
    goto loc_82090000;

loc_82090000:;
    /* 0x82090000  mfspr r12, r8, r0 */
    ctx->r[12] = ctx->lr;
    /* 0x82090004  stw r12, -8(r1) */
    address = (uint32_t)ctx->r[1] + (uint32_t)(-8);
    xenolith_store32(base, address, (uint32_t)ctx->r[12]);
    /* 0x82090020  cmpli cr6, 0, r10, 0 */
    ctx->cr[6].lt = 0;
    ctx->cr[6].gt = ((uint64_t)(uint32_t)ctx->r[10]) > (0ull);
    ctx->cr[6].eq = ((uint64_t)(uint32_t)ctx->r[10]) == (0ull);
    ctx->cr[6].so = (uint8_t)(ctx->xer >> 31) & 1;
    /* 0x82090028  bc 12, 26 0x82090034 */
    if (ctx->cr[6].eq) { goto loc_82090034; }
    goto loc_8209002c;
```

Nothing is reordered or optimised by the emitter. The C compiler is better
placed to do that.

## Reading the report

```
functions               22006
  lifted                22006  (100.000 percent)
    import thunks         241
  not lifted                0
helper entries             57
declarations            22063
units                      39
  largest             4448200 bytes
kernel build      0x2017f400
name               EXAMPLE GAME
id                 example-game
game folder        /home/you/.local/share/xenolith/games/example-game (linked)
```

| line | meaning |
|---|---|
| functions | what discovery found |
| lifted | how many were emitted |
| import thunks | calls into the console's system libraries, emitted as calls into the runtime rather than translated |
| not lifted | functions refused because they hold an instruction the model cannot express |
| helper entries | entry points part way into the register save and restore helpers, which calls land on |
| declarations | every name the C refers to, which is more than the functions because of the helper entries |
| kernel build | which kit the game was built against, from its container |
| name, id | what the program is called, see [Naming the program](#naming-the-program) |
| game folder | where the program looks for the game, and whether this lift linked it there or found something there already |

## When something is not lifted

```sh
xenolith lift default.xex --out ./build --blockers
```

`--blockers` ranks the instructions that stopped functions by how many functions
each one blocked. Modelling the top entry moves the coverage by roughly its
count. `--unlifted` lists every function that was not emitted, with the address
and mnemonic that stopped it.

## Feeding a run back

A running title that reaches an address nothing was lifted for reports it.
`XENOLITH_TRACE_DISPATCH` makes it report every such address and carry on:

```sh
XENOLITH_TRACE_DISPATCH=1 ./game 2>&1 | grep '^dispatch ' | awk '{print $2}' | sort -u > roots.txt
xenolith lift default.xex --out ./build --roots-from roots.txt
```

This helps when the address is inside the image: it is a function discovery
missed, typically one reached only through a table the title builds at run
time. An address outside the image, or one that is not code, is the title
failing for some other reason, and lifting more will not help.

```sh
xenolith disasm default.xex --start 0x82871af0 --length 4
# error: 0x82871af0 is not in an executable section
```
