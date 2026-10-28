# Playing a game

This page walks through turning a game you own into a program and playing it,
from a fresh machine to the title screen. Each step says what it does and what
you should see. [Lifting a title](/guide/lifting) covers the same ground in more
depth for someone working on the translation itself.

The examples use a game kept in `~/games/example` and built into
`~/games/example-build`. Any paths work.

## 1. Install what the build needs

You need a Rust toolchain and a C compiler to build anything. Install Rust
with [rustup](https://rustup.rs), since distributions often package an older
release than the one this project needs. Graphics, a
window, sound and music are each picked up when present, and a program built
without one of them still runs and says what it is missing.

| need | for | when it is needed |
|---|---|---|
| Rust 1.85 or later | building xenolith | always |
| a C compiler, `make`, `pkg-config` | building a game | always |
| Vulkan headers and loader, and a Vulkan driver | drawing | when the game is built and run |
| X11 headers, or Wayland headers with `wayland-protocols` and `wayland-scanner` | a window | when the game is built |
| PulseAudio (or PipeWire's PulseAudio server), or ALSA | sound | when the game runs |
| libavcodec 58 or 60 | music and voices, which games keep as XMA | when the game runs |

On Arch Linux:

```sh
sudo pacman -S --needed base-devel vulkan-icd-loader vulkan-headers \
    libx11 wayland wayland-protocols libpulse ffmpeg
```

On Debian or Ubuntu:

```sh
sudo apt install build-essential pkg-config libvulkan-dev \
    libx11-dev libwayland-dev wayland-protocols libpulse0 libavcodec-dev
```

On Fedora:

```sh
sudo dnf install gcc make pkgconf-pkg-config vulkan-loader-devel \
    libX11-devel wayland-devel wayland-protocols-devel pulseaudio-libs \
    ffmpeg-free-devel
```

Also install the Vulkan driver for your graphics card (`vulkan-radeon`,
`vulkan-intel` or the NVIDIA driver on Arch, `mesa-vulkan-drivers` on Debian and
Fedora). `vulkaninfo --summary` lists the devices Vulkan can see.

Sound and music libraries are looked for while the game runs, not while it
builds, so adding one later needs no rebuild. The libavcodec development package
is listed because it provides the unversioned library name, which is tried when
neither version 58 nor 60 is installed.

## 2. Build xenolith

```sh
git clone https://github.com/QaidVoid/xenolith
cd xenolith
cargo build --release
```

The tool is `target/release/xenolith`. The build also leaves
`libxenolith_host.a` beside it, which every lift copies into its output. A
built game reads its executable from your copy with it, and translates the
shaders it loads from its data files while it runs. Run the lift with that
binary, not a copy elsewhere, so the library is found.

Instead of building it, you can download it from the
[releases](https://github.com/QaidVoid/xenolith/releases). Each archive holds
`xenolith` and `libxenolith_host.a`. Keep the two side by side wherever you
unpack them, for the same reason.

```sh
gh attestation verify xenolith-<version>-x86_64-linux.tar.xz --repo QaidVoid/xenolith
tar -xJf xenolith-<version>-x86_64-linux.tar.xz
```

The first line checks the archive was built by this project's release workflow
from a commit in its repository, using the attestation published with the
release. It needs the GitHub command line tool, and can be skipped.

The commands below call it `xenolith`. Either put its directory on your `PATH`
or write the path out.

## 3. Supply the key

A retail game's executable is encrypted with a key this project does not ship.
Put the 32 hexadecimal digits in a file:

```sh
mkdir -p ~/.config/xenolith
printf '%s' '<32 hexadecimal digits>' > ~/.config/xenolith/xex.key
chmod 600 ~/.config/xenolith/xex.key
```

[Key material](/guide/keys) explains what the key is and the other ways to give
it.

## 4. Get the game's files

xenolith reads a folder holding the game's `default.xex` and every other file
from its disc. From a disc image, unpack it with a tool that reads the Xbox
disc file system, such as `extract-xiso` or `xdvdfs`:

```sh
mkdir -p ~/games/example
extract-xiso -d ~/games/example -x example.iso
```

Keep each game in a folder of its own, anywhere you like. The built program
finds it through a link the lift makes in step 5.

Check the executable reads before going further. `inspect` reads only the
headers, so it works without the key:

```sh
xenolith inspect ~/games/example/default.xex
```

## 5. Lift it

```sh
xenolith lift ~/games/example/default.xex --out ~/games/example-build
```

This takes a minute or two and writes the translated C, the runtime and a
`Makefile` into `~/games/example-build`. Nothing of the game's own goes there.
It ends with a report:

```
functions               36533
  lifted                36533  (100.000 percent)
...
name               Example Game
id                 example-game
game folder        /home/you/.local/share/xenolith/games/example-game (linked)

written to /home/you/games/example-build
build it:  make -C /home/you/games/example-build -j"$(nproc)"
run it:    /home/you/games/example-build/example-game
```

The name is the one the game gives itself, read out of its executable. It titles
the window. The id is made from the name and names the program file, and it is
what your desktop groups the window by and what the sound mixer lists the game
under. To choose either yourself:

```sh
xenolith lift ~/games/example/default.xex --out ~/games/example-build \
    --name "Example" --id example
```

An id is lower case letters, digits, dots, dashes and underscores. A game that
carries no name of its own is called `xenolith`.

"Game folder" is where the program looks for your copy of the game when it
runs: `games/<id>` in xenolith's directory, `~/.local/share/xenolith`. The lift
links the folder you gave it there. If something is there already it is left
alone, so you can also put the game's files there yourself, or point the
program anywhere else with `XENOLITH_GAME`:

```sh
XENOLITH_GAME=~/games/example ./example-game
```

## 6. Build it

```sh
make -C ~/games/example-build -j"$(nproc)"
```

This compiles a few hundred files of C, which takes several minutes the first
time. The result is the program named by the id, here
`~/games/example-build/example-game`.

## 7. Run it

```sh
~/games/example-build/example-game
```

The program finds the game itself, so it can be started from anywhere. It reads
`default.xex` from your copy with your key, checks it is the version the
program was built from, and opens a window with the game in it. Your copy is
only ever read: what the game saves goes elsewhere (step 8).

### Controls

| key | button |
|---|---|
| arrow keys | the pad |
| `z`, `x`, `c`, `v` | A, B, X, Y |
| enter | start |
| escape | back |
| `q`, `e` | left and right shoulder |
| `1`, `3` | left and right trigger |
| `w`, `a`, `s`, `d` | left stick |
| `i`, `j`, `k`, `l` | right stick |
| `f`, `h` | left and right stick pressed |

### What it prints

A run prints a few lines when it starts, saying what it assumed about the
console, such as the video cable and the region, and what this machine could
not provide. None of those is an error. The lines worth reading are:

| a line beginning | means | what to do |
|---|---|---|
| `xenolith: nothing here draws, because no graphics driver answered` | Vulkan found no device | install the Vulkan driver for your card |
| `xenolith: no frame will be shown, because` | no window could be opened | run it inside a desktop session |
| `xenolith: no sound will be heard:` | no sound device was found | check PulseAudio or PipeWire is running |
| `xenolith: compressed sound is not decoded:` | libavcodec is missing | install ffmpeg's libavcodec |

## 8. Where saves go

Everything the game saves goes in a directory of its own among your data, named
by the program's id:

```
~/.local/share/xenolith/saves/example-game/
```

Each save the game makes is a folder there named the way the game named it.

Where `XDG_DATA_HOME` is set, xenolith's directory is under it instead of under
`~/.local/share`, and `XENOLITH_DATA` names xenolith's directory outright. Its
games and saves move with it, so that saves go in
`$XENOLITH_DATA/saves/example-game/`.

Saves are kept apart from the build, so lifting again, deleting the build or
moving the program elsewhere leaves them where they are. Two builds with the
same id share their saves.

- To back saves up, copy the game's saves directory.
- To start fresh, move it away. The game makes a new one.

## 9. After updating xenolith

Pull, rebuild the tool, and lift again into the same directory:

```sh
cd xenolith
git pull
cargo build --release
xenolith lift ~/games/example/default.xex --out ~/games/example-build
make -C ~/games/example-build -j"$(nproc)"
```

The lift replaces the translated code, the runtime and the host library. Your
copy of the game and your saves are not in the build, so nothing touches them.
Lift again rather than only running `make`: `make` alone rebuilds the old
translation.

## Sharing a build

A built program holds the translation and the runtime, and nothing of the
game's own: no executable, no data, no image. It reads all of that from the
player's copy when it runs. So a build can be handed to someone who owns the
game, as the one program file.

They need:

1. The program, built on a machine with the same graphics and sound libraries
   they have, or on an older one.
2. Their own copy of the game, unpacked into
   `~/.local/share/xenolith/games/<id>/`, or anywhere they name with
   `XENOLITH_GAME`. The id is the program's file name.
3. Their own key, as in step 3.

The program checks their copy is the version it was built from and refuses
another, saying so, rather than running code that is not there.

The program still holds a translation of the game's code. Whether handing one
to someone is allowed where you are is for you to find out before you do.

## When it goes wrong

**The lift says the executable is encrypted and no key material was found.**
The key is not where the tool looks. See step 3. A built program looks in the
same places, and says the same.

**The program says the game is not at a folder.** It looked there for your copy
and found no `default.xex`. Put the game's files in that folder, or name the
folder holding them with `XENOLITH_GAME`.

**The program says the game is not the one it was built from.** Your copy is
another version or region of the game than the one lifted. Lift your own copy
instead.

**The build fails on a missing header.** One of the development packages from
step 1 is missing. The makefile only turns a capability on when `pkg-config`
finds it, so a header error means the package is half there. After installing,
run `make clean` and then `make`, so that every file is built with the new
capability.

**The game stops with an address that was not lifted.** The translation reached
code that nothing found while lifting. [Feeding a run back](/guide/lifting#feeding-a-run-back)
shows how to give those addresses to the next lift.

**Something draws wrong, or a sound is missing.** That is a gap in the
translation rather than in your setup. A run says when it refused a shader or
skipped a draw, and those lines are what to report.
