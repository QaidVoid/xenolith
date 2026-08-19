# Key material

A retail XEX is encrypted, and this project ships no key. This page explains
what the key is, why one is needed, and how to supply it.

## What is actually encrypted

A XEX carries a per title session key in its header. That session key is what
the image is encrypted with, and it is different for every game.

The session key itself is stored wrapped, encrypted with one of a small number
of static key encryption keys built into the console. Retail titles use the
retail one. There is a separate devkit key, and titles built for a devkit use
that instead.

So decoding a retail title is two steps:

1. Unwrap the session key from the header using the static key.
2. Decrypt the image with the session key.

The static key is the only thing this project does not have. It is 16 bytes, it
is the same for every retail title, and it has been public for well over a
decade. It is not derived from your console, it is not tied to any particular
game, and knowing it does not decrypt anything you do not already have.

It is not shipped here because a project that distributes it is distributing a
console's key material, and there is no need to.

## Supplying it

Three sources are consulted, in order:

1. `--key-file <PATH>`, if given.
2. The `XENOLITH_XEX_KEY` environment variable, as 32 hexadecimal digits.
3. `~/.config/xenolith/xex.key`.

The file form is usually what you want, since it survives across shells and does
not end up in your history.

```sh
mkdir -p ~/.config/xenolith
printf '%s' '<32 hexadecimal digits>' > ~/.config/xenolith/xex.key
chmod 600 ~/.config/xenolith/xex.key
```

Whitespace and underscores are ignored, so any grouping you find readable works.

## Working without one

Every subcommand except `inspect` accepts `--raw`, which reads an image
something else already decoded. That needs no key at all.

```sh
xenolith analyze image.bin --raw --base 0x82000000
```

Nothing describes the layout of a bare image, so it is treated as one executable
span. That is a worse starting point than a container, which carries a real
section table, but it makes work possible on a title whose key is not to hand.

`inspect` without `--decode` reads only the headers, which are not encrypted, so
it always works.

## When a title is not encrypted

Some titles are stored unencrypted. `inspect` reports which, on the `encryption`
line, and decoding one needs no key. The tool only asks for key material when
the container says the image is encrypted.
