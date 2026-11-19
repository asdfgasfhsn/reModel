# Shader packs: for any game

Shared words, roles and rules: [README.md](README.md). Nothing here is
built. The idea was added by the user on 2026-11-19: a shader engine in
the spirit of RetroArch's, where people bring shaders that apply to any
number of games. RetroArch is the inspiration, not something to import.

Every other pack is for one game. This is the one kind that is not.

## The problem

How the finished picture is treated (scanlines, a phosphor mask, colour
grading, sharpening, film grain) has nothing to do with which game is
running. Today reModel has a fixed set of such treatments and no way to
add one without changing reModel.

**Observed.**

- **The host exists.** After a frame is composed, reModel already runs
  passes of exactly this kind: a CRT pass (`crates/gpu/src/crt.wgsl`),
  bloom (`bloom.wgsl`) and the look's ink pass (`look.wgsl`). They are
  WGSL, they are opt-in, and each is byte-exact when off
  (`docs/enhancements.md`, "CRT", "Bloom", "Looks").
- **The order is fixed in code**: 3D, the look's ink, bloom's blur, the
  2D layers, compose, then CRT (`crates/gpu/src/lib.rs:2848-2971`). Bloom
  is added inside the compose pass, before the HUD, so the HUD is never
  smeared; only the CRT pass runs on the finished picture
  (`docs/enhancements.md`, "Bloom").
- **They are chosen as picture styles** in the player
  (`docs/play.md`, "Picture styles"), and each has its own settings.
- **The picture is not a small frame scaled up.** The 3D layer is drawn
  at the output's size; only the 2D layers are made at the game's
  496x384. The CRT pass draws "one scanline for each of the game's 384
  lines whatever the output size".
- **Shaders are checked when tests run**, through naga
  (`crates/gpu/src/lib.rs:3384`), and the renderer is on wgpu 30
  (`crates/gpu/Cargo.toml:14`).
- **There is a precedent for an optional native library**: FSR and DLSS
  were weighed as an optional feature with the built-in path kept
  (`docs/enhancements.md`, "FSR 2 and DLSS").

**From other projects' documentation** (not their code; several pages
could not be read in full).

- RetroArch's "slang" presets chain passes, each with its own scale,
  filtering and float targets, fed by the frame, earlier passes, their own
  last output, a few frames of history and lookup images. Parameters
  declared in a shader become sliders. Presets can layer over a base
  preset.
- Its inputs are the colour picture and its history only. No emulator
  surveyed documents giving depth or anything else to a user's shader.
- The existing shader collections are large and of mixed licence,
  including some that forbid commercial use. Loading shaders a user
  supplies avoids the question; shipping the collection would not.
- A standalone library can run these presets on wgpu. By its
  documentation it is not yet at version 1, builds a C++ shader compiler,
  and does not run every shader under wgpu.
- DuckStation accepts slang presets and ReShade's format. ReShade's
  depth access is a guess made from outside a game; reModel's depth is
  exact.

## Design

reModel's own shader packs: the ideas that make RetroArch's system good,
in reModel's shader language, with inputs only reModel can give.

### What is taken from RetroArch

Chained passes, each with its own scale and filtering. A pass reading
earlier passes, its own last output and a few frames of history. Lookup
images. Parameters declared in the pack that become sliders. Presets as
small files over a base. Choosing globally or per game.

### The format

A folder with a manifest and WGSL files, in `<data dir>/shaders/<name>/`,
usable with every game.

```
name   = Aperture grille
format = 1
param  mask   0.4  0..1    0.05  "Mask strength"
param  curve  0.02 0..0.1  0.005 "Screen curve"
lut    grain  grain.png
pass   scan.wgsl   stage=screen  scale=output  inputs=frame
pass   glow.wgsl   stage=screen  scale=0.5     float  inputs=pass:1
pass   final.wgsl  stage=screen  scale=output  inputs=frame,pass:1,pass:2,lut:grain
```

**A pass** is one fragment function with a fixed signature. reModel
supplies the vertex stage and the bindings, so an author writes only what
a pixel becomes.

**Two stages**, matching where reModel's own passes run today. A
`scene` pass works on the 3D layer before the HUD and 2D layers are put
over it, which is where bloom adds its glow. A `screen` pass works on
the finished game picture, which is where the CRT pass runs.

**What a pass can read:**

| Input | Is |
|---|---|
| `frame` | For a `screen` pass, the composed game picture before the bezel and the player's menus; for a `scene` pass, the 3D layer |
| `pass:N` | An earlier pass's output |
| `feedback:N` | That pass's own output from the last frame |
| `history:N` | The frame from N frames ago, up to four. Feedback and history restart on a camera cut, a state load and a switch of machine ([scene-understanding.md](scene-understanding.md), "State across frames") |
| `lut:NAME` | An image in the pack |
| Uniforms | Output size, the game's size (496x384) and where its picture sits, frame count, time, the parameters |
| `depth`, `class`, `light`, `motion` | The buffers of [scene-understanding.md](scene-understanding.md), if the pack says it `uses` them |

The last row is what reModel can offer that the others cannot: a fog
that follows real depth, grain that spares the sky, a glow that takes
only lit surfaces. A pack that `uses` a buffer runs without it when the
buffer is unavailable (a game on the chip path) and can test for that; a
pack that `needs` one is refused there with a message.

**Parameters** become rows in the Picture page, as the built-in styles'
settings are, and can be changed live. A named set of values is a
preset, a small text file that can be shared on its own.

**Limits, enforced by the loader:** at most eight passes, four frames of
history, a memory budget for targets, and thirty-two parameters. A pack
is validated with naga when loaded, and an error names the file and line.
A pack whose passes cost more than a set share of the frame for a second
is switched off with a message.

**Safety.** A shader pack runs after the frame is composed and reads
only. It cannot reach the machine, files or the network, and cannot
change a state hash.

**The proof** is to move the built-in CRT pass into a `screen` shader
pack and get byte-for-byte the same picture. Bloom is the test of the
`scene` stage: its blur is ordinary passes, but its glow is added inside
compose, so matching it exactly may need compose to accept a `scene`
pack's output where it accepts bloom's today. reModel then ships both as
its first shader packs, in its own original code.

### Not proposed: importing RetroArch's shaders

The user's intent is inspiration, not import, and the survey supports
that. Running existing presets would mean a second project's library
tied to reModel's wgpu version, a C++ build step, gaps in which shaders
work, and a collection of mixed licences. Those shaders also expect a
small source picture to scale up, where reModel draws 3D at the output's
size, and they could not use depth or the lighting layer. An author who
wants a particular effect can port it to a reModel shader pack, where
its licence allows.

### Where it sits

Order is fixed: the look (toon and ink, in and after the 3D pass), then
the 3D layer with any `scene` passes, compose, any `screen` passes, then
the bezel and the player's menus. A
shader pack is chosen globally or per game in the Picture page. A game
pack's `look.txt` may name a shader pack it suggests; if the user does
not have it, the look works without it and says so.

## Interfaces

| Where | What |
|---|---|
| `<data dir>/shaders/<name>/shader.txt` | The manifest above |
| Picture page | Choose a shader pack, tune its parameters, save a preset |
| `--shader NAME`, `--shader-set key=value` | The same for the CLI and `scene render` |
| `shader check DIR` | Validates, lists inputs and parameters, and times each pass on a scene |
| `shader list` | Installed shader packs and presets |

## The roles

- **Player.** Picks "Aperture grille" for every game, and a different
  preset for one.
- **Artist or shader author.** Edits a `.wgsl` file; the player reloads
  it on save, as it does a look, and shows a compile error with its line.
- **Art agent.** `shader check --json` gives pass or fail and
  milliseconds per pass. Budget: 2 calls, 1 picture.
- **Pack lead.** A game pack can suggest a shader pack without carrying
  it.
- **Tool engineer.** New buffers offered to shaders are added in one
  table.
- **Tester.** Off is exact. The built-in CRT as a pack equals the
  built-in CRT.

## Testing

- **Without ROMs:** the manifest parser; a pass chain on a synthetic
  frame with known output; limits refused with a message; a broken shader
  reported with its line; feedback and history restarting on a reset; the
  CRT identity test, and bloom's if it can be made exact.
- **With ROMs:** state hashes equal with any shader pack on.

## Performance

Bloom costs 0.2 to 0.25 ms at 1080p and 0.65 to 0.75 ms at 4K
(`docs/enhancements.md`, "Bloom"; documented, not re-measured), which is
the scale of a light shader pack. A heavy community preset can cost many
times that; no figures were found. `shader check` reports the real cost
per pass on the user's machine, and the loader's time limit is the
backstop.

## The browser

WGSL passes are the renderer's own language and the shared renderer runs
in the browser, so shader packs would be nearly free there. It is not
scheduled, in line with the decision to keep the browser out of uplift
work.

## Open questions

- **Can bloom be matched exactly** as a `scene` pack, or only closely?
- **Should a shader see the bezel**, for reflections on it?
- **Photo mode**: should captures run through the chosen shader pack?
- **Gating by class** gives a shader coarse knowledge of what a pixel
  is. Is exposing the artwork or model ID useful, or too game-specific
  for a pack meant for any game?

## MVP: a few days

1. `shader.txt` with a single pass, parameters as Picture rows, and
   reload on save.
2. The built-in CRT moved into a shader pack, with the identity test.
3. Multi-pass, feedback and history, and the `scene` stage, with bloom
   moved over.
