# Photo mode

The native player's photo mode (`remodel-play`; F8, the pause menu's
**Photo mode**) freezes the game and lets the player frame a picture of it
the way current games' photo modes do: a free camera flown around the
frozen moment, a time scrub back through the last seconds, depth of field,
motion blur, a colour look, film touches, the HUD on or off, and a capture
to PNG well above the window's size, supersampled with every picture
setting at its best. The game itself is never touched: closing photo mode
goes on exactly where it was, and a run with a photo session in the middle
ends on the same state hash as one without (`make photo-check`).

It is native only; the browser build doesn't have it (below, "In the
browser").

## Using it

F8 (or **Photo mode** in the Esc menu, or `--photo` for scripted runs)
opens it on the frame shown. The game, its sound and its clock pause; a
compact dock along the bottom edge holds the settings (never a panel over the
picture), and the window shows the photo as it will be taken, at the
window's size. Enter, Space, the dock's red shutter button or the pad's A
takes it: drawn at the chosen size, read back and
written on another thread to
`reModel/<set>/<set>-<time>-<frame>.png` in your pictures folder (on
Linux the XDG pictures directory, `~/Pictures` unless `user-dirs.dirs`
says otherwise; `~/Pictures` on macOS; the Pictures known folder on
Windows, wherever it was moved; without one, `<data dir>/<set>/photos/`;
`--photo-dir DIR` puts them elsewhere; a photos folder chosen in System
settings, Folders, makes it `<folder>/<set>/`). The notice at the
top says when it is written. Esc, F8, the pad's B, Start or Guide leaves
photo mode and the game goes on.

| Control | Keyboard and mouse | Gamepad |
|---|---|---|
| Move (along the camera's own axes) | W, A, S, D; R up, F down; Shift 4x, Ctrl 1/4x | left stick |
| Look (turn in place) | arrow keys; right mouse drag | right stick |
| Orbit (turn about the point ahead: the focus distance, or the scene's scale) | left mouse drag | |
| Pan | middle mouse drag | |
| Move in and out | mouse wheel | |
| Roll | Q, E | click the left or right stick |
| Zoom (field of view) | Z, X | LT, RT |
| Focus on a point | double-click it | |
| HUD on or off | Tab | X |
| Dock on or off (Hide UI: only the picture is left, and a small "Show controls") | H | Y |
| Reset the camera | Backspace | Select |
| Tabs | `[` `]`, or click | LB, RB |
| The dock's cells | PageUp, PageDown to walk them, `,` `.` to change one; or click its arrows (held, they repeat) | d-pad up and down walk the cells (down off the last goes to the tab bar, up comes back), left and right change the one focused (held, they repeat) or move along the bar |
| Take a photo | Enter, Space, the shutter button | A (on a value; on a button A presses it) |
| Leave | Esc, F8, **Leave** | B, Start, Guide |

The dock shows one tab at a time, as a row of cells (name, value, `<` `>`;
text is never under 14 points at 720p), with the tab bar under it: **Hide**
(Y), the tabs, **Grid** (a rule-of-thirds grid over the picture, inside the
letterbox; it is not in the photo), **Leave** (B) and the shutter. The dock
and its margin take at most 140 points of a 720-point window; the photo is
drawn on its own, so none of it is ever in a shot. When a photo is written, a
notice at the top names the file. A setting that does nothing as things are is
left out, and a tab with none is not shown (with the chip geometrizer there is
no free camera or motion blur, so no Camera tab; with no history, no Time
tab).

- **Camera**: **Field of view** (zoom), **Roll**, **Near clip** (with the camera moved, polygons nearer than
  this share of the scene's scale are clipped (1% by default), so flying
  through a wall or a car shows what is beyond rather than a screen-filling
  polygon; "the game's" keeps the original near plane), **Reset camera**.
- **Time**: the time scrub, a strip over the last seconds (`--photo-history`,
  10 by default; click or drag along it, or left and right for a frame): the
  frame shown is run again from the history, so it is exactly what the game
  showed then. "Live" is the frame photo mode opened on.
- **Focus**: **Depth of field**, with **Focus distance** (in the scene's
  scale; it starts on what is a little below the middle of the picture, where
  third-person games put the player, and a double-click on the picture sets it
  to what is there), **Aperture** (the blur of something far away, as a share
  of the picture's height) and **Bokeh** (how much bright parts stand out as
  discs in the blur).
- **Look**: **Exposure** (stops), **Contrast**, **Saturation**, **Warmth**,
  **Tint**, and **Shading** (the game's setting, flat, vertex or pixel).
- **Effects**: **Vignette**, **Film grain**, **Aberration** (chromatic),
  **Bloom**, **Motion blur** (4 to 32 images averaged) and its **Shutter**
  (the share of the frame before that it covers).
- **Frame**: **Letterbox** (2.39:1, 1.85:1, 16:9, 4:3, 1:1 bars), **Photo
  size** (the window's, 1080p, 1440p, 4K (2160) or 8K (4320) lines, the window's
  picture's shape), **HUD** (the game's HUD over the photo, as the game drew it,
  whatever the camera; off by default), **2D background** (the tile layers
  behind the 3D: off gives the backdrop colour, for games whose sky is a tile
  layer that can't follow the free camera) and **Depth buffer**.

(Bloom, depth buffer and shading are the picture settings for the photo,
starting from the game's.)

## Scripted

`--photo LIST` runs photo mode without anyone at the controls, for checks
and for making the same picture again: `FRAME[:KEY=VALUE,...]`, sessions
separated by `;`. When the game reaches FRAME the player opens photo mode,
applies the settings in order, takes one photo (waiting for the PNG),
prints a `photo` line and closes it. Keys:

| Key | Value |
|---|---|
| `back` | scrub this many frames back from the live one |
| `orbit`, `tilt` | orbit about the pivot by degrees (sideways, up and down) |
| `yaw`, `pitch`, `roll` | turn the camera in place by degrees |
| `x`, `y`, `z` | move along the camera's axes (right, up, forward), in the scene's scale |
| `zoom` (or `fov`) | focal length factor |
| `dof` | depth of field: the aperture in percent of the height (0 off) |
| `focus` | focus distance in the scene's scale |
| `bokeh`, `exposure`, `contrast`, `saturation`, `warmth`, `tint`, `vignette`, `grain`, `aberration` | the look's values as the dock has them (exposure in stops) |
| `letterbox` | `W:H` or a ratio |
| `blur`, `shutter` | motion blur images and shutter (a share of a frame) |
| `hud`, `background`, `depth` | `on` or `off` |
| `shading` | `flat`, `vertex` or `pixel` |
| `bloom` | percent |
| `near` | near clip in percent of the scene's scale |
| `res` | `window`, `1080`, `1440`, `4k`, `8k` or a height |
| `name` | the file's name (without `.png`) |

With `--fast` (as `make photo-check` uses) the player runs the game as fast
as it goes and draws nothing, so a scripted run doesn't depend on the
window's redraws (a hidden window gets about one a second).

Example: `remodel-play --game stcc --input coin-start --frames 3000 --photo
"2800:back=60,orbit=30,tilt=-8,dof=1,blur=8,res=4k,name=stcc-pass"`.

## How it works

### Leaving the game alone

The game's machine runs on as it does without photo mode, apart from two
things that read it:

- **History.** Before each frame it runs, the player notes the inputs the
  frame runs with (input ports, analog channels, the gun counters:
  `model2::photo::Inputs`), and every 60 frames it takes a save state
  (`Machine::save_state`, as F5 does), handed to another thread to compress
  (`model2::photo::History`). It keeps `--photo-history` seconds of them.
  Taking a state waits for the geometrizer's frame in progress, which
  changes only when the main thread waits, not what the machine computes.
- **Photo mode's own machine** (`model2::photo::Studio`), built from the same
  ROMs the first time photo mode opens and run the way the game's runs
  (`Machine::configure_like`: interpreter or recompiled code, idle skip,
  geometrizer). To show frame F it loads the newest state at least two
  frames before it and runs the frames up to F again with their recorded
  inputs; with smooth motion's recording and the lighting side channel on
  (they are render-only), and its sound commands dropped. With run-ahead,
  the frame shown is run-ahead's: the studio runs those frames beyond the
  game's with the inputs the game has, as run-ahead's machine does. Without
  history (`--photo-history 0`, or right after a load) it copies the
  machine shown (`Machine::copy_from`, as run-ahead does) and runs two
  frames on, so the picture is the frame two after the one shown.

Opening photo mode runs no frame of the game; closing it starts the
renderer's copy of video memory afresh (it held the studio's) and the game
goes on. `make photo-check` runs a coin-start run to a frame twice, with
and without a scripted photo session in the middle (open, scrub back 100
frames, orbit, depth of field, motion blur, HUD, a 1080p photo), and
compares the state hashes at the end; it also checks that the studio's
machine, run again from the history to the frame the game is on, has the
game's state hash (the frames shown are the game's, not a lookalike).

### The free camera

The geometrizer is handed each object's transform already in the game
camera's view space (model to view: a rotation and a translation, set by
the display list's matrix and translation commands). Smooth motion records
those transforms each frame and can run the frame's display list through
the native geometrizer again with other values (`smooth::Frame`,
`docs/hw/geometrizer.md`, "Smooth motion"). Photo mode does the same with
every transform moved into another view: for a camera at `eye` with axes
`R` (in the game camera's view space), `M' = Rᵀ M` and `t' = Rᵀ (t - eye)`;
focal lengths are scaled by the zoom (`model2::photo::Camera::pose`). The
light vectors the list sets are in view space too and are turned by `Rᵀ`
as the normals are (`smooth::Tap::light`, hooked where both native
geometrizers read the light command), so lighting stays fixed to the scene
as the camera moves; specular highlights move with the eye, as they should.
The geometrizer then culls, clips and projects for the new view as it does
for the game's, and smooth shading and material maps get the replay's own
side channel. One mechanism serves every game with a native geometrizer
(every 2B and 2C game, and 2A and the original Model 2 through the fixed
geometrizer).

`remodel scene render NAME --camera SPEC` uses the same code on a saved
frame (docs/uplift-studio.md, "Scenes"): `scene make` keeps the frame's
display list and the geometrizer's state before it (`display.bin`,
`smooth::Frame::to_bytes`), and the render runs it through
`model2::photo::render`, so a scene can be seen from another camera with no
ROM and no running game. SPEC takes the camera words above (`orbit`, `tilt`,
`yaw`, `pitch`, `roll`, `x`, `y`, `z`, `zoom` or `fov`, `near`, `hud`); both
parse them with `model2::photo::Spec` and `Camera::set_word`. `back` has
nothing to scrub in a scene and is ignored with a warning. The pictures
match photo mode's for the same frame and camera: a test draws the scene
both ways and finds no pixel off by more than 16 in a channel.

What it can't show is what the game never sent. Objects the game culled
for its own camera are missing (turn the camera around and the scene
behind the game's camera is usually empty; the far side of a model the
game only ever shows from the front may be missing or open), and level of
detail stays as the game chose it for its own camera. Direct data blocks
(polygons the list gives in view space, `fn 2`) don't move with the camera.
Windows the game draws with another camera (a mirror, a course map) are
moved like the rest. 2D tile layers (backgrounds, skies some games draw
in tiles) stay fixed on the screen; the **2D background** row leaves them
out.

### The picture

The renderer draws the frame as it does in play, at the photo's size times
the supersampling factor, with the HUD and (if asked) the 2D background left
out (`gpu::Renderer::photo`, `crates/gpu/src/photo.rs` and `photo.wgsl`):

1. One image per motion blur sample, each the display list posed at its
   own place between the frame before and the frame shown (smooth motion's
   blend of the two frames' transforms, then the camera), summed with equal
   weights in a 16-bit float image.
2. The last image's depth: each polygon's 1/z drawn again into a float
   image with a depth test (see-through and shadow polygons left out), the
   view-space depth depth of field needs, whatever the depth buffer setting.
3. Depth of field, gathered: every pixel within the largest blur radius
   nearby (found per 16x16 tile) spreads its colour over a disc of its own
   blur radius (thin lens: the aperture times how far its 1/z is from the
   focus plane's, relative to it), as much per pixel as the disc is small;
   one behind the pixel can't spread over it further than the pixel's own
   blur, so a sharp object keeps its edge against a blurred background and
   a blurred one in front spills over what's behind it. Bright samples weigh
   more for bokeh. 96 samples a pixel, rotated per pixel (the noise averages
   out in the downsampling).
4. The look: white balance (warmth, tint), exposure, contrast about mid
   grey, saturation, vignette; then the HUD, where the frame drawn with it
   differs from the frame drawn without it (both with the game's own
   geometry, so 3D HUD windows stay where the game put them).
5. The finish into the output: each output pixel the mean of its
   supersampled pixels, chromatic aberration (red and blue shifted apart
   towards the edges), grain, and letterbox bars.

Picture settings for photos: temporal anti-aliasing off (supersampling is
the better anti-aliasing for a still), 4x multisampling only where the
picture is not supersampled (below), the photo's
shading, bloom and depth buffer rows, finer brightness levels with pixel
shading, and 16x anisotropic filtering of texture pack images when a pack
is on. Bloom's blur chain gets one more level per doubling of the size, so
its glow is as wide as in the window.

Supersampling and memory: as many times the photo's size a side (up to
4) as keeps the drawn picture under about 36 million pixels, within the
GPU's largest texture and within a budget of about 1.4 GB. The photo pass
keeps about 28 bytes a drawn pixel (measured with the driver allocator's
report on an RTX 2080 SUPER: the picture, its depth and depth test, the 3D
layer and its depth buffer, the 16-bit float picture the look works on, the
bloom chain), 8 more each with the HUD and with motion blur (their
pictures and the sum). 1080p is drawn at 4x (7680x4320), 1440p at 3x, 4K at
2x (7680x4320 for 4K: about 0.93 GB; with the HUD and motion blur, 4K is
drawn at 1x). 4x multisampling goes with 1x only (it
adds 32 bytes a pixel for its colour and depth, and supersampling
anti-aliases better): 4K and 5K at 1x get it, 8K is drawn at 1x without
(33 million pixels at about 31 bytes is 1 GB; with the multisampled targets
it was 2 GB).

The photo pass's textures (and the renderer's 3D layer, depth, multisampled
targets and bloom levels) are kept between draws, so the live preview, which
draws a photo every frame at the window's size, allocates nothing after the
first, and are freed after each photo and on leaving photo mode (the preview
then goes back to its footprint of about 200 MB at 1280x720). The game's
temporal history is dropped on entering: photos are drawn without it.

If the GPU runs out of memory (the budget is a guess: a desktop and its
compositor can hold most of the card, and the free memory isn't known), the
photo is drawn again one step down and the message says what it ended up at
("drawn at 3840x2160 with 4x MSAA, after the GPU ran out of memory at
7680x4320"): each smaller supersampling, then 1x with 4x multisampling where
the budget allows, then 1x alone. A photo fails only if even that does not
fit. The draw runs in a wgpu error scope, so an out-of-memory error is
caught and not the player's panic.

Measured (2026-11-17, RTX 2080 SUPER 8 GB, Vulkan, daytona, `res=4k`; the
desktop held 6.3 GB of the 8 GB before the player started): before, a 4K
photo drew at 7680x4320 with 4x MSAA (16 samples a pixel; the multisampled
colour and depth alone 1 GB, 1.8 GB or more with the rest) and failed with
"Out of Memory" with the card's memory at 7.4 GB; now it is drawn at
7680x4320 with 935 MB of the renderer's allocations and card use peaking
about 1 GB over its starting point; forced back to the old 2x with 4x MSAA
the draw runs out of memory and falls back to 2x without. With the game
running in a visible window the card's use was 7.5 GB (130 MB of it the
player's, by `nvidia-smi`; the rest the compositor's and others', which
grew when the window opened), leaving 0.7 GB: a 4K photo then fell back
from 2x to 1x with 4x MSAA to 1x alone, and was taken. That is why the
fallback is needed. With the HUD, motion blur, depth of field and bloom
on, the 2x picture needed more than the 1.4 GB tried, and the budget now
sends 4K at 1x with 4x multisampling (about 0.6 GB) from the start.

## What works, what doesn't

*Confirmed* (2026-11-14, scripted runs on one NVIDIA GPU, Vulkan, the
pictures looked at):

- `make photo-check` passes on `topskatr` (2C), `lastbrnx` (2B), `srallyc`
  (2A), `vf2` (2A) and `daytona` (the original Model 2): the same state
  hash at frame 2400 with and without a session at frame 2200 that scrubs
  100 frames back, orbits, turns on depth of field and 8-image motion blur
  and takes a 1080p photo; and the studio, run again from the history,
  has the game's own state hash at the frame it opened on. Also with
  `--run-ahead 1` (`stcc`) and with `--photo-history 0` (`doa`), where the
  frame shown is not the game's own, the state hashes match.
- Free camera pictures that make sense (orbits, moves, turns) in
  `topskatr`, `srallyc`, `lastbrnx`, `daytona`, `vf2`, `stcc`, `doa`:
  lighting stays on the scene, the geometrizer clips for the new view,
  the near clip shows what's beyond a wall the camera flies into.
- Depth of field with the focus found on the fighter or skater, the HUD laid
  sharp over a blurred picture, vignette, grain, chromatic aberration and
  letterbox; 1080p, 4K and 8K photos.
- The dock's layout, pad and mouse paths, hide, thirds, and its height and
  text-size bounds, in the player's unit tests (`photo/dock/tests.rs`).
- The bench state hashes (`make bench`: `srallyc`, `lastbrnx`, `topskatr`,
  `vf2`, `daytona`) are unchanged.

*Not done or unconfirmed:*

- Tiling: a photo is drawn in one piece, so its size is limited by the GPU's
  largest texture and memory (8K is drawn at 1x without multisampling).
- Not tried by hand: the mouse, keyboard and a real gamepad were not used on
  a visible window (the test machine's window was hidden, about a redraw a
  second, hence `--fast`); the window's preview cost while moving the
  camera wasn't measured. The controls' directions were checked by
  scripted moves only.
- Recompiled code (`RECOMP=1`): not built here (the recompiled program
  crates take hours to build on the shared machine). The studio is made
  like the game's machine (`Machine::configure_like`), so it runs the
  recompiled code when the game does, as run-ahead's machine does;
  `make photo-check RECOMP=1` is the check to run.
- 3D HUD parts a game doesn't list as HUD windows in its manifest (the
  speedometer needle in `srallyc`, `daytona`'s map) stay with the HUD off and
  move with the free camera; windows drawn with another camera (mirrors)
  move too.
- Things the game culled are missing from the free camera's view (above),
  and a sky drawn in tiles stays fixed (the **2D background** row).

## Costs

Measured on the development machine (16-core desktop, RTX 2080 SUPER):

- **History, while playing**: a save state every 60 frames, 2.4 to 3.9 ms
  on the game's thread (`topskatr`, `lastbrnx`, `stcc`; 10 to 12 ms on a
  loaded machine), compressed on another thread to 0.1 to 0.5 MB; 10
  seconds hold 1 to 6 MB. Printed at the end of a `--seconds`/`--frames`
  run (`photo history ...`). `--photo-history 0` turns it off.
- **Opening photo mode**: 35 to 330 ms (the studio loads the newest state
  and runs up to 61 frames); the first time also loads the ROMs and builds
  the studio, 250 to 780 ms, and keeps it (one more machine: 50 to 85 MB)
  for the session.
- **Taking a photo**: drawing and reading back 1080p (drawn at 7680x4320)
  60 to 420 ms (motion blur's images add most of it); 4K 340 ms; 8K 410
  ms. Writing the PNG on its thread: 0.2 to 0.35 s at 1080p, 1.3 s at 4K,
  3 s at 8K.
- **Nothing** while photo mode isn't open, apart from the history.

## What was tried and dropped

- Saving the history from a spare machine (copying the game's machine into
  it, as run-ahead does, and serialising the copy on another thread) to
  take the save state off the game's thread: the copy's states loaded into
  the studio gave a different state hash on `topskatr` and `lastbrnx` (the
  copy carries less than a save state), and waiting for the spare's
  compression cost more than it saved. Dropped for the plain save state.
- Depth of field from the renderer's depth buffer: it leaves out what is
  drawn in the game's order (skies, but also large floors and roads), which
  would blur the road under a car as if far away; photo mode draws its own
  1/z of every polygon instead.

## The browser

Not there. Everything the photo needs is in crates the browser build shares
(`model2::photo`, `gpu::Renderer::photo`); what is native here is the
player's side (`crates/play/src/photo.rs` and `photo/dock.rs`: the dock, the controls, the
studio's ROM load, PNG writing on a thread). A browser photo mode would
keep the studio in the emulation worker, take states there (WebAssembly
memory permitting: about 1.2 MB a second compressed), draw the photo with
the same renderer call, and hand the PNG to a download (`Blob`); the
readback of a large texture is asynchronous there.

## A true camera (design)

The free camera can't show what the game culled. A *true camera* would let
the game itself cull and draw for a new viewpoint: in a copy of the
machine, write the camera the player chose into the game's own camera
variables and run one frame, so the game's own visibility, level of detail
and the geometrizer all work for that view. Design:

- Per game (data in `crates/romset/src/games/<set>.rs`, a `camera` field
  of `Game`): where the game keeps the camera it draws with (the address of
  its view matrix or position and angles, and their format), and the point
  in the frame at which to write it: after the game has worked out its
  camera for the frame (from the player's car, a replay path ...) and
  before it sends the objects' transforms to the geometry processor; an
  instruction address to break at, as the idle-loop table names addresses.
- Generic mechanism (`model2::photo`): run the studio to the frame before,
  then the frame itself with a one-shot hook at that address that
  overwrites the camera with the chosen one (composed with the game's,
  so the photo camera's controls stay relative), and draw that frame. The
  game's logic runs one frame in the studio, so the scene moves on by a
  frame from the one frozen; running the frame once more from the same
  state with another camera gives the same scene from elsewhere.
- Finding the addresses: the matrices the list sends for static scenery
  are the camera's rotation times a fixed model matrix, so the camera's
  rotation is the one 3x3 orthonormal matrix in RAM that, applied with a
  fixed matrix, gives them frame after frame; the instruction that last
  writes it before the list is built is the hook (`REMODEL_WATCH`). Where
  the copro (2A TGP, 2B copro SHARC) multiplies camera and object, the
  camera is uploaded to the copro each frame and can be found in the copro
  command stream instead.

**Spike** (`make camera-scan`, `remodel camera-scan FRAME GAP`; generic,
no per-game data yet): run a coin-start run to FRAME and GAP frames on,
take the turn most scenery objects' matrices share between the two frames
as the camera's, and list the 3x3 rotations in the i960's work RAM
(`0x200000`-`0x23FFFF`, `0x500000`-`0x5FFFFF`) that turned by exactly that.
Results (2026-11-14, *unconfirmed*: candidates, nothing written to them yet):

- `topskatr`: two candidates at frames 3600+30 and again at 4200+40:
  `0x583040` (by columns, world to view) and `0x583010` (the same turn
  transposed: likely the inverse kept beside it), each followed by three
  words that change like a position. The best lead for a first true camera.
- `lastbrnx` (2B): the scenery turned 67 degrees between frames 3000 and
  3120 and no rotation in RAM did: the camera is kept as angles or another
  form, or only multiplied in on the copro SHARC; the copro's command
  stream is where to look next.
- `srallyc`, `daytona`, `stcc`: the coin-start script drives straight, so
  the camera hardly turns between the frames tried; they need frames in a
  corner (an `--input` with steering).

Not built beyond the spike: the hook (the instruction that writes the
camera before the list is built, `REMODEL_WATCH` on the address found) and
the manifest field are the next steps.
