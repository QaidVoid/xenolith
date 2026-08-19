# Imports

An import is a call from the game into something the console provided. The
container says where every one of them is, and this page covers what is actually
written there and what the recompiler does with it.

## What the container says

The import libraries header lists, per library, a set of addresses within the
image and nothing else. What is at those addresses says the rest.

Each address holds a word of the shape:

```
   31      24 23      16 15                     0
  +----------+----------+------------------------+
  |   kind   | library  |        ordinal         |
  +----------+----------+------------------------+
```

The library byte is the index of the library the address was listed under, which
makes reading the word a check rather than an assumption. A record whose index
disagrees with the list that held it is rejected, because calling the import it
appears to name would call the wrong one.

## Slots and thunks

**Kind 0 is a slot.** A word in read only data that the console's loader
overwrites with the address of the import. Emitted code reads it from guest
memory like any other data, so there is nothing to translate.

**Kind 1 is a thunk.** A four word stub in executable memory:

```
0x823d75cc  01010035 ? .long 0x01010035
0x823d75d0  02010035 ? .long 0x02010035
0x823d75d4  7d6903a6   mtspr r11, r9, r0
0x823d75d8  4e800420   bcctr 20, 0
```

The first two words are placeholders the loader overwrites with the halves of
the address it resolved. The two after them build a jump to whatever those hold.

Before this was read, those addresses were the single largest thing stopping
functions from lifting on both titles: 187 functions on one and 156 on the
other, all reported as having no blocks, because the first word of each is not
an instruction and never was.

A record declared to be a thunk is confirmed to have a thunk's shape before it is
believed: a second word holding the same record with the next kind, then the jump
through the count register. A record that does not is rejected naming its
address.

## What is emitted

```c
/* 0x823d681c  import: xam.xex ordinal 651 */
void sub_823d681c(xenolith_context *ctx, uint8_t *base) {
    xenolith_import(ctx, base, "xam.xex", 651);
}
```

The alternative would be to write plausible `lis` and `ori` words over the
placeholders so the thunk disassembles, but they would have to name an address
that does not exist. A direct call says what is actually known.

The library is named rather than numbered because the index is a property of one
image's header ordering and means nothing outside it. A runtime asked for
`xam.xex` ordinal 651 can answer; one asked for library 0 ordinal 651 has to be
told what 0 meant.

## What is not done

**Ordinals are not mapped to names.** Doing so would take a table per library
version, which is per title configuration arriving by another route. The ordinal
is what the container says, so the ordinal is what is reported.

**No import is implemented.** `xenolith_import` is a declaration. What ordinal
651 of `xam.xex` does is the environment's problem, and there is no environment.

## Reading them

```sh
xenolith inspect default.xex --imports
```

```
  import records (492)
    xam.xex            88 thunks     88 slots
    xboxkrnl.exe      153 thunks    163 slots

    0x82000400  slot   xam.xex          ordinal 651
    0x823d681c  thunk  xam.xex          ordinal 651
```

The counts hold up in a way that is worth noticing. One title's `xam.xex` lists
176 records, 88 slots and 88 thunks, so every import has both. Its
`xboxkrnl.exe` lists 316, of which 163 are slots and 153 are thunks, so ten
imports are data with no thunk, which is what a data import looks like.

Reading the records needs the image decoded, so this flag needs key material.
The summary that works without a key keeps reporting only what the headers say.
