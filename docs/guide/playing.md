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
`libxenolith_shader.a` beside it, which every lift copies into its output so
that a game can have the shaders it loads from its data files translated while
it runs. Run the lift with that binary, not a copy elsewhere, so the library is
found.

Instead of building it, you can download it from the
[releases](https://github.com/QaidVoid/xenolith/releases). Each archive holds
`xenolith` and `libxenolith_shader.a`. Keep the two side by side wherever you
unpack them, for the same reason.

```sh
tar -xJf xenolith-<version>-x86_64-linux.tar.xz
```

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

Keep each game in a folder of its own. The lift links everything beside
`default.xex` into the build, so a folder shared with other things gets those
linked too.

Check the executable reads before going further. `inspect` reads only the
headers, so it works without the key:

```sh
xenolith inspect ~/games/example/default.xex
```

## 5. Lift it

```sh
xenolith lift ~/games/example/default.xex --out ~/games/example-build
```

This takes a minute or two and writes the translated C, the runtime, a
`Makefile` and the decoded image into `~/games/example-build`. It ends with a
report:

```
functions               36533
  lifted                36533  (100.000 percent)
...
name               Example Game
id                 example-game
game files linked          17

written to /home/you/games/example-build
build it:  make -C /home/you/games/example-build -j"$(nproc)"
run it:    cd /home/you/games/example-build && ./example-game
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

"Game files linked" counts the files from the game's folder linked into the
build, which is where the program looks for them. A name the build already holds
is left alone, so lifting again links nothing new.

## 6. Build it

```sh
make -C ~/games/example-build -j"$(nproc)"
```

This compiles a few hundred files of C, which takes several minutes the first
time. The result is the program named by the id, here
`~/games/example-build/example-game`.

## 7. Run it

```sh
cd ~/games/example-build
./example-game
```

The program reads the game's files from the directory it runs in, so start it
from the build directory. A window opens with the game in it.

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

Everything the game saves goes in `content/` inside the build directory, here
`~/games/example-build/content/`. Each save the game makes is a folder there named
the way the game named it.

- To back saves up, copy `content/`.
- To start fresh, move `content/` away. The game makes a new one.
- Lifting again into the same directory keeps `content/`. Deleting the build
  directory deletes the saves with it, so copy them out first.

## 9. After updating xenolith

Pull, rebuild the tool, and lift again into the same directory:

```sh
cd xenolith
git pull
cargo build --release
xenolith lift ~/games/example/default.xex --out ~/games/example-build
make -C ~/games/example-build -j"$(nproc)"
```

The lift replaces the translated code, the runtime and the shader library, and
keeps the links and the saves. Lift again rather than only running `make`:
`make` alone rebuilds the old translation.

## When it goes wrong

**The lift says the executable is encrypted and no key material was found.**
The key is not where the tool looks. See step 3.

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
