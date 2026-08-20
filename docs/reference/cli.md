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

Writes numbered translation units, a header of declarations, the runtime
interface header, and a makefile. Units a previous run wrote into the same
directory are removed first, since two units defining the same function will not
build.

Functions that could not be lifted are the expected outcome rather than a
failure. The subcommand reports them and succeeds.

```sh
xenolith lift default.xex --out ./lifted --blockers
make -C ./lifted -j8
```

See [Lifting a title](/guide/lifting) for what the output holds and how to read
the report.
