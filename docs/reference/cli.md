# Commands

```
xenolith <COMMAND>

  inspect  Summarize an Xbox 360 executable
  disasm   Disassemble a range of an Xbox 360 executable
  analyze  Report the functions, blocks, and jump tables of an executable
  lift     Emit C for the functions of an executable
```

## Shared input options

`disasm`, `analyze`, and `lift` take the same input shapes.

| option | meaning |
|---|---|
| `<FILE>` | the XEX file, or a decoded image when `--raw` is given |
| `--raw` | treat the input as an image something else already decoded |
| `--base <ADDR>` | address a raw image loads at, default `0x82000000` |
| `--key-file <PATH>` | a file holding the static key as 32 hexadecimal digits |

Addresses and sizes are accepted in decimal or with a `0x` prefix.

`--raw` needs no key material, which is what makes work possible against a title
whose key is not to hand. Nothing describes the layout of a bare image, so it is
treated as one executable span.

## inspect

Summarizes a container from its headers, so it works on an encrypted title with
no key.

| option | meaning |
|---|---|
| `--decode` | decode the image as well as reading its headers |
| `--imports` | list every import with its address, library, ordinal, and kind |
| `--key-file <PATH>` | a file holding the static key |

`--imports` reads what each record names, which is written in the image rather
than in the headers, so it decodes the image and needs whatever key material
that takes.

```sh
xenolith inspect default.xex
xenolith inspect default.xex --imports
```

## disasm

| option | meaning |
|---|---|
| `--start <ADDR>` | address to start at |
| `--length <BYTES>` | how many bytes to disassemble |
| `--end <ADDR>` | address to stop before |
| `--sweep` | report coverage over every executable section instead of printing code |
| `--allow-data` | disassemble a range not marked executable |

A misaligned start is refused rather than rounded, and a range outside the image
is refused rather than truncated.

`--sweep` decodes every executable word and reports how many decoded, how many
did not, and which encodings the failures were. That is the number to watch when
adding to the instruction table.

```sh
xenolith disasm default.xex --start 0x82090000 --length 256
xenolith disasm default.xex --sweep
```

## analyze

| option | meaning |
|---|---|
| `--functions` | list every discovered function with its range and how it was found |
| `--tables` | list every recovered jump table with its targets |

The report gives the function count, how each was found, what proportion of
executable words a function claims, and how many indirect branches were resolved
against how many were not.

The function count is followed by one line per origin, and they sum to it:
`entry point`, `exported`, `called`, `found by shape`, `in the record`,
`named by a table`, and `by surroundings`. The first three are functions
something proves; the rest are functions the image or a shape suggests.

```sh
xenolith analyze default.xex
xenolith analyze default.xex --tables
```

## lift

| option | meaning |
|---|---|
| `--out <PATH>` | directory to write the emitted C into, required |
| `--part-size <BYTES>` | bytes of C per translation unit, default 4 MiB |
| `--roots-from PATH` | read addresses to treat as functions from a file, one per line |
| `--root ADDRESS` | treat an address as a function, for one a built program reached and discovery did not |
| `--blockers` | list the instructions that stopped functions, most blocking first |
| `--unlifted` | list every function that was not lifted, with what stopped it |
| `--name <TEXT>` | what the program's window is titled, in place of the name the game gives itself |
| `--id <ID>` | what the program file is called and what a desktop and a sound server know it by, in place of the one made from the name |

Writes numbered translation units, a header of declarations, the runtime, a
boot program, the host library and a makefile, and for a bare image given with
`--raw` the decoded image as well. Units a previous run wrote into
the same directory are removed first, since two units defining the same function
will not build.

The program is named for the game. Without `--name` the window takes the name
the game's executable carries, and without `--id` the program file takes an id
made from that name, such as `example-game`. An id is lower case
letters, digits, dots, dashes and underscores, and any other is refused.

Nothing of the game's own is written for a container: the program reads the
game from the player's copy when it runs. So the folder holding the executable
is linked at `games/<id>` in xenolith's directory of the user's data, where the
program looks for it, unless something is there already.

Functions that could not be lifted are the expected outcome rather than a
failure. The subcommand reports them and succeeds.

```sh
xenolith lift default.xex --out ./build --blockers
make -C ./build -j"$(nproc)"
```

See [Lifting a title](/guide/lifting) for what the output holds and how to read
the report.
