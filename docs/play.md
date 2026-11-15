# The native player

`make play` (or `make play RECOMP=1` with recompiled code from `make recomp`)
builds and runs `remodel-play`: the game in a window, rendered on the GPU
(`docs/gpu.md`), with both sound boards running live. `remodel-play --help`
lists its options.

With no game given (`make play`, or `remodel-play` alone) it opens the
launcher (below); `--game ID` (`make play GAME=ID`) starts that game
straight away. At startup the reModel logo shows for 1.5 s, then fades out
over 0.5 s over the launcher (or the game booting underneath). Any
key, click or gamepad button skips it; `--no-splash` leaves it out. The
logo (`assets/reModel-logo-1086.png`, half the size of the original) is
built into the binary and decoded with `remodel-png`. `--fullscreen`
starts fullscreen (remembered, as `--rom` is), and `--interp` runs the
i960 on the interpreter in a build with recompiled code. `--no-idle-skip`
runs the games' idle loops instruction by instruction instead of
fast-forwarding through them (`docs/idle-skip.md`): the game is the same
either way, only slower.

**Geometrizer.** The geometrizer runs on the native implementation
(`docs/hw/geometrizer-2b.md` for Model 2B, `docs/hw/geometrizer-2c.md`
for 2C) instead of the game's uploaded program on its chip (the 2B SHARC,
the 2C TGP), where it implements that program (every 2B and 2C game so
far); the stream it draws is the same, word for word, and it takes under
a tenth of the interpreted chip's time (a quarter of the recompiled SHARC's,
a twentieth of the recompiled TGP's), so 2B games race at full speed
without recompiled code. The Geometrizer setting (or `--geo chip`, also
`sharc` or `tgp`, for one run) runs the game's own program instead; it
applies when the game starts. The `remodel` CLI keeps the chip
as its default, since its oracles (`lockstep`, `geo-check`) test that
program.

**Anti-aliasing and the depth buffer** (settings, below; `--aa off|4|8`
and `--depth on|off` for one run; both off by default, the original
look). Anti-aliasing multisamples the 3D layer (4x, or 8x where the GPU
has it, else 4x) and blends polygon edges by coverage; the depth buffer
draws polygons by their depth rather than the game's far-to-near order,
so crossing and mis-sorted ones come out right, keeping the game's order
for what depth can't settle (skies, shadows, translucent layers, large
floors). Neither changes what the game computes: state hashes are the
same. Costs 0.5-1 and 1.5-3 ms a frame at 1080p on the development GPU
(`docs/gpu.md`); what the depth buffer does is in `docs/enhancements.md`.

**Bloom** (setting `bloom`, below; `--bloom off|N` for one run, N a strength
from 1 to 100 percent; off by default). The bright parts of the 3D layer
glow: blurred through a chain of halvings and added back over the picture
before the HUD, so text and the HUD stay sharp and the tile layers don't
feed it. Works with anti-aliasing, temporal anti-aliasing and upscaling,
needs the same 16-bit float targets as temporal (a GPU without them has no
row), GPU only. The menu offers Off, Light (25), Medium (50) and Strong
(100). It is not part of any preset: it is taste, not fidelity. Costs about
0.25 ms a frame at 1080p and 0.7 ms at 4K (`docs/enhancements.md`, "Bloom").
There is no fog setting: the games send no fog to smooth (same place,
"Fog").

**Temporal anti-aliasing** (settings `temporal` and `stipple`, below;
`--temporal off|taa|75|67|50` and `--stipple blend|keep` for one run; off
and blend by default). Each frame's 3D layer is drawn at a slightly
different sub-pixel offset and blended into the picture so far, moved
along each polygon's motion; `taa` draws it at full size, a percentage
draws it that much smaller and upscales (50 to 100; the menu offers 75,
67 and 50). The motion comes from smooth motion's recording, so turning
temporal on turns that recording on (0.05-0.5 ms a game frame, plus one
geometrizer replay: 0.2-0.4 ms on 2C, 1.4-1.8 ms on 2B) whether or not
smooth motion's in-betweens are on; with them on, each in-between is
blended with its own motion. It needs the native geometrizer (with the
chip it is off: the launcher says so and the menu leaves its row out), and it replaces
multisampling: `aa` is kept but off while temporal is on (the menu's
anti-aliasing row is left out, the launcher's row says why). A GPU
without 16-bit float render targets (the OpenGL fallback, say) can't, and
says so. The games' checkerboard (screen door) see-through polygons are
drawn as an even 50% blend with `stipple = blend`, which stays steady at
any scale; `keep` keeps the pattern, where the 3D layer has at least 1.5
pixels a game pixel (else it blends anyway). The accumulation starts
again on a camera cut, a save state load, a game start, a resize, a
change of quality settings and a switch between run-ahead's machine and
the game's. The launcher's previews never use it. Nothing the game
computes changes. How it works and what was measured are in
`docs/enhancements.md`; the browser build doesn't have it.

What to expect. At `taa`, polygon edges are smooth whether the picture
moves or not, a still picture is cleaner than with 4x multisampling,
textures look about as sharp as a supersampled picture (the hard edges of
magnified texels, such as a skater's cut-out shadow, are kept), and it
costs about 1 ms a frame more than off at 1080p (4 at 4K) on the
development GPU. It is not perfect: objects animated without motion data
(waving flags; `vstriker`'s players, whose pose models change) look soft,
nearer the plain picture, a car bouncing with the camera loses some of its
smoothing, and on `vstriker`'s crowd a still picture shimmers slightly.
Fast driving softened distant scenery in earlier tests; not checked since
the fixes to the history's clip and resampling. At 67% and 50% the 3D
layer costs about the same as off, or less, and looks better than simply
enlarging a smaller picture. Thin things (overhead wires, rails, distant
poles) stay whole lines, a little softer than at `taa`, where they used
to break into dotted, speckled lines; at 50% close-set wires still show
some grain. Edges in fast motion look as coarse as the smaller
picture; 50% shows it more. Shadows and other checkerboard
polygons look like smooth see-through with `blend`, not the original
screen door. Something the game flickers on purpose (`topskatr`'s FINISH
sign) flickers the same, not averaged. The first frame after a camera cut,
a load or a resize is the plain picture, sharpening over the next few
frames. On the CPU it adds smooth motion's recording and a replay a game
frame (above), which an interpreted 2B game running ahead may not have to
spare (`sgt24h` with run-ahead 1 fell behind and the player dropped
run-ahead). Prefer 4x multisampling where the original's screen door
matters, with `--geo chip`, or when the CPU has no time to spare.
*Unconfirmed:* tried on one NVIDIA GPU (Vulkan) on a 60 Hz display;
in-betweens with it on were only forced there, not seen on a faster
display.

**Smooth shading** (opt-in; settings `shading`, `shading_fine` and
`shading_balance`, below, and `crease` in the settings file only;
`--shading flat|vertex|pixel`, `--crease auto|N`, `--shading-fine
on|off` and `--shading-balance on|off` for one run; `flat`, `auto`, fine
off and balance on by default; docs/enhancements.md, "Smooth
shading"). The games
light each polygon with one brightness, so curved surfaces show their
facets. `vertex` lights each polygon corner with the game's own lighting
for the mean of the normals of the faces around it that belong to the
same smooth surface, and blends it across the polygon; `pixel` works the
game's lighting out for every pixel, for the normal blended across the
polygon, so highlights (the games that use specular light: `topskatr`,
`srallyc`, `stcc`, `bel`, `lastbrnx`, `fvipers`, `schamp`) sit where the
surface faces them instead of being spread between corners. Faces
meeting at more than the crease angle keep a hard edge (`auto` is the
game's own angle: 40 degrees, 25 for `schamp`; 0 smooths nothing, the
original picture). The normals and lighting come from what the
native geometrizer recorded about each polygon (its side channel, on
whenever shading is); with the chip (`--geo chip`, not turned off by it)
only block facts are known, so `vertex` blends the polygons' own
brightnesses instead and `pixel` is drawn as `vertex` (the menu and
launcher say so, and the window title says what was drawn: `shading
pixel` or `shading vertex`). It works with temporal anti-aliasing (its
first frame after a reset is the shaded frame), multisampling, the depth
buffer and texture packs (a pack image's lighting follows the shaded
brightness). A pixel never changes hue, only brightness: where a game's
colour tables turn a colour into another one at other brightnesses
(`topskatr`'s ramp boards, `dynabb97`'s players, `lastbrnx`), the shading
stays within the brightnesses of the polygon's own colour. HUD windows,
shadows and polygons whose corners all keep their own normal are left as
the game drew them, pixel for pixel. Smooth motion's in-betweens are
replayed with their own side channel, frames drawn with the last frame's
polygons when the geometrizer is late use that frame's, and run-ahead's
machine records it too, so every image is shaded at the same level and
shading doesn't alternate between images; a frame drawn again reuses its
shading. The launcher's previews are always flat. Nothing the game
computes changes. On the CPU it costs about 1-2 ms each time a new frame
is drawn (up to 6 ms on busy frames on a loaded machine), and building
the lit polygons 0.1-2 ms more; repeats cost nothing. On the GPU it is
within the noise at 1080p and a little at 4K with `pixel`; with
`flat` (or a crease of 0) none of it runs.

What to expect (from the review pictures): clearly better on
fighters and curved bodies (`lastbrnx`'s and `fvipers`' fighters,
`doa`'s, `topskatr`'s skaters, `stcc`'s cars, `segawski`'s life vest);
little or no change on mostly flat cars and scenery (`sgt24h`, `srallyc`,
roads, buildings; `indy500` lights its cars evenly); worse on art drawn
to be faceted: `schamp`'s Sonic quills and Knuckles' dreadlocks melted
together at 40 degrees, so `schamp` uses 25, which keeps their ridges;
`vstriker`'s small low-poly players look a little rounder, not worse, at
40. Each limb of a fighter is shaded on its own, so joints keep a seam.
Gradients step through the game's 64 brightness levels, so large smooth
surfaces show bands; **Blend brightness levels** (`shading_fine`, the
menu's checkbox under Shading, the launcher's "Brightness levels") blends
the two of the game's colours around each lit pixel's brightness
instead, which removes the bands and never leaves the polygon's colour
(only lit pixels; the rest stay the game's; pure black, such as ink
lines, stays black). **Balance brightness** (`shading_balance`, on by
default; the menu's checkbox under Shading, the launcher's "Brightness
balance") keeps each smooth surface as bright overall as the game drew
it; off, the game's lighting is worked out at the smoothed normals with
no correction, so bumpy surfaces (`skisuprg`'s snow) come out brighter
(on screen a small change). The window title says `unbalanced` when it
is off. *Unconfirmed:* tried on one NVIDIA
GPU (Vulkan) on a 60 Hz display (in-betweens forced, not seen on a
faster display).

**Texture packs** (settings `pack` and `tex_filter`, below; `--pack on|off`,
`--texture-pack DIR` and `--tex-filter off|mip|2|4|8|16` for one run; both
off by default). With `pack` on, replacement textures from the game's pack
(`docs/texture-packs.md`) are drawn on the GPU in place of the originals: 3D
textures, and tile patterns (HUD, text, 2D backgrounds) at the window's
resolution when the pack has tile images. The
pack is the folder `<data dir>/<set>/texpack/` (`out/<set>/texpack` in a
source tree; `make upscale-textures` makes one), and a clone with none uses
its parent's (checked: `overrevb` with only `overrev/texpack`). `--texture-pack DIR` names one for
every game that run and turns `pack` on; `--pack` alone means on. PNGs are
decoded on background threads when their texture is first drawn and the
original shows until then. The launcher's previews never use packs. The Esc
menu's line under the checkbox shows the folder, how many images it holds and
how many of the last frame's textured polygons (and tile patterns) were
replaced, or why there is no pack (a missing folder also gives a notice). `tex_filter` filters pack
images only: `off` (as they are), `mip` (mipmaps, no shimmer on distant
ground) or `2`, `4`, `8`, `16` (mipmaps plus anisotropic filtering up to that
many times, for ground at grazing angles); the menu offers only what the GPU
has (3D textures only; tiles are drawn bilinear). Neither changes what the
game computes. Checked on `topskatr` with a scripted run (tiles too:
`--shot` in a race, the HUD drawn from realesrgan-x4plus tiles). *Not verified:* smooth motion's in-betweens and F5/F9 save
slots with a pack, and decode stutter was not measured.

**Material maps** (settings `materials` and `materials_relief`, below;
`--materials on|off` and `--materials-relief off|S` for one run; off, and
relief 2, by default; docs/texture-packs.md, "Trying material maps"). With
`materials` on and a texture pack whose `pack.txt` says format 2 and that
holds normal maps (`make pack-materials` makes them), the maps tilt each
pixel's normal and the game's own lights shade the bumps: grooves in wood
and brick, rougher road. It needs the texture pack on and works with any
`shading` (with `flat` the polygons stay flat but carry the maps' detail;
the player then records the geometrizer's lighting side channel, as
shading does, for the lamps). `materials_relief` is the rule: relief at a
strength (the menu and launcher offer 1, 2, 4 and 8; the settings file and
command line 0.1 to 8; such a strength shows in the menu as one more
choice, "custom (1.5)", and in the launcher as "Custom (1.5)", kept until
another is picked), how strongly the bumps' light change scales the
surface's brightness, or `off`, the faithful rule (the map's normal in the
game's own lighting sum: subtler, and nothing where the game lights a
surface to full brightness). The menu's line under the checkbox (and the
window title) says what the last frame did, e.g. `relief 2: maps on 594 of 633
lit textured polygons (74 materials drawn, 194 held, 145 MB; 234 map
files)`, or why they have no effect: no texture pack, a pack without
maps, the chip geometrizer (`--geo chip`: no exact lighting layer, the
picture is as with them off), or no polygon in the frame lit by the exact
layer. The launcher's previews draw without them. Maps share the pack's
GPU memory budget with its images. Nothing the game computes changes.
Checked on `topskatr` (234 generated maps on the ESRGAN pack,
`--frames 3300` shots at 1920x1080): the player's frame with maps on,
relief or faithful, flat or pixel shading, changes exactly the pixels
`gpu-check` changes for the same captured frame (538,345 of 2,073,600 for
relief 2, pixel shading; the two pictures otherwise differ only where the
player draws the pack's tile images); turning them on in the menu gives
the same picture as `--materials` and is saved; with `--geo chip` the
picture is identical to materials off and the title says why; temporal's
plain frame changes the same pixels. *Not verified:* in-betweens on a
display over 60 Hz, run-ahead with maps.

**Smooth motion** (a setting, below; `--smooth on|off` for one run, a
bare `--smooth` meaning on; off by default). On a
display faster than the game's 57.5 Hz, the refreshes between two game
frames show the 3D geometry part way from the frame before to the newest
one, by the time since the newest was first shown, instead of repeating
it: at 120 Hz every other refresh is an in-between at the halfway point,
at 144 Hz they fall where the clock puts them. Each in-between runs the
newest frame's display list again through the native geometrizer with
every object's matrix interpolated (rotation on the arc, scale and
position in a straight line) between its value in the two frames;
`docs/hw/geometrizer.md`, "Smooth motion", has how objects are matched
and when a frame counts as a camera cut and isn't blended. The 2D layers
(HUD, text) stay on the game's frames, and the game itself runs exactly
as without. The cost is one game frame (about 17 ms) of latency, since
what shows is always between the last two frames, and an extra
geometrizer pass per in-between on the main thread: 0.2-0.4 ms for the
2C games tried, 1.4-1.8 ms for the 2B ones (`remodel --smooth`);
the title bar counts the in-betweens and their time. It needs the native
geometrizer: with `--geo chip` it is off. *Unconfirmed* in the player
itself: it builds and the in-betweens it draws are the ones checked with
`make smooth-frames`, but nobody has played with it on a fast display yet.
At 60 Hz (one game frame per refresh) there is nothing to draw between,
and none are drawn below 69 Hz or when the game is behind (paced by the
clock).

**Run-ahead** (a setting, below: off up to the game's useful maximum, 3 at
most; `--run-ahead N` for one run; off by default). Every game takes some frames from reading
a control to showing what it does: its own input lag, on top of the
display's. Run-ahead hides up to N frames of it: after each game frame a
second machine takes a copy of the game's state and runs N more frames
with the controls as they are, and that machine's last frame is what's
drawn (`model2::runahead`; RetroArch's idea, done here from scratch). The
game itself never runs those frames: its sound, save states, EEPROM and
state hash are exactly as without (`make run-ahead-check`), and nothing
the frames ahead do leaves the second machine (their sound commands are
dropped, its EEPROM and drive board are never read). How much there is
to hide depends on the game (`make input-lag`, each game's `docs/games/<set>.md`):

| Game | Control | Input lag (frames) | Offered by the menu (`run_ahead_max`) |
|---|---|---|---|
| `topskatr`, `skisuprg`, `segawski`, `waverunr` | curving, swing, lean, handlebar | 0 | none: no row |
| `overrev`, `overrevb` | wheel | 0 | none |
| `vstriker` | stick, shoot | 0 | none |
| `hotd` | trigger | 0 (once 1) | none |
| `schamp` | stick 0, punch 1 | 0 | off or 1 (helps punches, the stick judders) |
| `lastbrnx` | stick 0, punch 1 | 0 | off or 1 (helps punches, the stick judders) |
| `stcc`, `indy500` | wheel | 1 | off or 1 |
| `srallyc` | wheel | 1 | off or 1, if the machine has 2x the game's speed (7 ms a frame interpreted) to spare |
| `doa` | punch, stick | 1 | off or 1 |
| `fvipers` | punch | 1 (8 once, mid-move) | off or 1 |
| `dynabb97` | bat, stick | 1 to 6, by the moment | off or 1 |
| `bel` | trigger | 3 (one measurement) | up to 3 |
| `sgt24h` | wheel | 4 | up to 3 (the most the player offers) |

Input lag here counts the frames after the first one that reads the
control, until the picture first differs from a run without it (`remodel
latency`, which `make input-lag` runs for every game: two machines from
the same frame of a run in play, one with the control held, their
renders compared pixel by pixel). 0 means that first frame already shows
it, which in the player is one frame from input to screen (the display
adds its own). The fighters were measured with no button already under
way; a move in progress can hold a control off for many frames (above,
the "once" values), which no run-ahead hides. Each game's least value is
in its manifest (`Game::input_lag`), and the most the menu offers is
`Game::run_ahead_max`: that lag, at most 3, or 1 for `schamp` and
`lastbrnx`, whose punches lag a frame (the help line says it helps them and
makes the stick judder). A game with nothing to hide (`run_ahead_max` 0)
has no run-ahead row; `--run-ahead N` still takes 0 to 3 for any game but
warns above the game's maximum. The help line says what the row does for
the game, and, when the player switched run-ahead off because the machine
could not keep up (below), that it did.

Once the game has run for a couple of seconds the help line also says how
much run-ahead this computer has time for: the player times the game's own
frames, the main thread's hand-over of each to drawing and the drawing
(without run-ahead, so the figure doesn't change when it is turned on),
and run-ahead N costs about N+1 emulated frames for each one shown, so it
counts how many fit in 85% of the display's frame time beside the
hand-over, with the drawing on its own thread (below, "The frame loop")
fitting in that time by itself (with `--no-overlap`, all of it on one
thread, the drawing is counted in with the frames), up to 3 ("this
computer has time for about 2", or "this computer is
too slow for run-ahead"). A choice above that is warned of ("3 is more
than it can keep up with, and is switched off if the game falls behind";
the launcher's hint turns amber): it still works, and the 3 s rule below
stays as the safety net. Before the game has been measured (just started,
or the launcher showing another game) the line says nothing extra. The
figure is an estimate: the first seconds of a game, a heavy scene or the
launcher's own drawing (it measures the preview there) can move it.

A game whose picture changes in the first frame that reads the control
(0 above) has nothing to hide: run-ahead then only shows the game N
frames further on than it is, and whenever a control changes, what's
shown jumps from the guessed future to the real one (a judder of up to N
frames). So run-ahead suits the games with lag above, by at most that
many frames; it stays off unless set, and is a per-game setting like the
others.

The cost is a copy of the game's state per frame shown and N more frames
of emulation, so a machine needs about N + 1 times the speed the game
takes alone (the title bar says what run-ahead takes of the emulate time,
and its copy). The copy moves only the memory either machine wrote since
the last one (`Model2Bus::copy_from`; tile RAM and buffer RAM most
frames, texture RAM when the game uploads) and the chips' state directly,
no save state in between, keeping what the copro and geometrizer cores
have decoded:

| Game | Board | A game frame | Geometrizer wait + copy | Run-ahead 1 | Run-ahead 2 | Save + load instead |
|---|---|---|---|---|---|---|
| `topskatr` | 2C | 6.1 ms | 0.1 + 0.8 ms | 7.4 ms | 13.0 ms | 3.6 + 4.2 ms |
| `skisuprg` | 2C | 6.0 ms | 0.2 + 0.7 ms | 7.4 ms | 12.8 ms | 5.3 + 1.8 ms |
| `stcc` | 2C | 6.5 ms | 0.2 + 0.8 ms | 7.9 ms | 13.9 ms | 3.8 + 1.7 ms |
| `segawski` | 2C | 5.8 ms | 0.2 + 0.8 ms | 7.2 ms | 12.5 ms | 3.6 + 2.2 ms |
| `waverunr` | 2C | 6.8 ms | 0.2 + 0.8 ms | 8.4 ms | 14.3 ms | 4.8 + 2.2 ms |
| `overrev` | 2C | 6.7 ms | 0.2 + 0.7 ms | 8.1 ms | 13.8 ms | 4.9 + 1.8 ms |
| `hotd` | 2C | 6.7 ms | 0.2 + 0.8 ms | 8.4 ms | 13.9 ms | 3.8 + 2.1 ms |
| `bel` | 2C | 5.3 ms | 0.2 + 0.8 ms | 6.7 ms | 11.6 ms | 3.9 + 1.9 ms |
| `overrevb` | 2B | 8.6 ms | 0.8 + 1.2 ms | 12.9 ms | 16.4 ms | 5.0 + 3.0 ms |
| `sgt24h` | 2B | 7.0 ms | 1.0 + 1.3 ms | 11.7 ms | 14.3 ms | 3.7 + 3.1 ms |
| `dynabb97` | 2B | 12.1 ms | 0.3 + 1.2 ms | 15.7 ms | 23.2 ms | 3.5 + 3.5 ms |
| `doa` | 2B | 8.2 ms | 0.9 + 1.2 ms | 12.6 ms | 16.6 ms | 4.3 + 3.7 ms |
| `indy500` | 2B | 6.1 ms | 0.4 + 1.0 ms | 7.8 ms | 14.1 ms | 4.7 + 3.3 ms |
| `fvipers` | 2B | 8.6 ms | 0.7 + 1.1 ms | 10.9 ms | 19.9 ms | 5.4 + 3.0 ms |
| `schamp` | 2B | 9.4 ms | 1.2 + 1.1 ms | 12.1 ms | 22.5 ms | 4.4 + 3.5 ms |
| `vstriker` | 2B | 9.1 ms | 1.0 + 0.9 ms | 11.1 ms | 21.1 ms | 3.1 + 2.9 ms |
| `lastbrnx` | 2B | 10.3 ms | 1.0 + 0.9 ms | 10.5 ms | 21.7 ms | 4.0 + 4.4 ms |
| `srallyc` | 2A | 12.2 ms | 0.2 + 0.6 ms | 12.2 ms | 24.2 ms | 6.7 + 2.9 ms |

Per shown frame, natively, the i960 interpreted and the native
geometrizer (`make run-ahead-check`, 2,071 frames of a coin-start run,
four games at a time on a 16-core desktop; run-ahead 1 with smooth
motion on, 2 without): a game frame as the game runs it alone (the
geometrizer on its thread not counted), the wait for the game's
geometrizer frame and the copy, then everything run-ahead adds to each
frame. A whole save state and load instead of the copy would cost the
last column (16 MB each way). Recompiled builds weren't measured:
run-ahead's frames run the recompiled code too, so they cost what the
game's frames do there, plus the same copy.

With the chip geometrizer (`--geo chip`) the copy also waits for the
game's own geometrizer frame, which otherwise runs alongside the next
frame: about one interpreted SHARC frame on 2B, so run-ahead there
needs the native geometrizer (the default). If the game falls behind
(paced by the clock) for 3 s with run-ahead on, the player turns it off
and says so; setting it again tries again. The second machine takes as
much memory again as the game's machine (50 MB for `topskatr`, 85 MB
for `vstriker`, measured). The launcher's previews never run ahead.

With smooth motion on too, the in-betweens blend the frames that are
*shown*, one run ahead from the last game frame with the next: the
second machine notes the transforms of the last frame of each run only
(`Geometrizer::set_smooth_shown`) and keeps them across copies, and the
game's own machine records none (`docs/hw/geometrizer.md`, "Smooth
motion"). `make run-ahead-check` compares them with a plain run's.

### The frame loop

Each redraw the main thread reads the controls, runs the frames that are
due (and run-ahead's), and hands the frame shown over to drawing; the
draw thread (`crates/play/src/draw.rs`) then draws it, the overlay over
it, and presents it, while the main thread goes back to the event loop
and on to the next frame. So emulating frame N+1 (and its frames ahead)
runs alongside drawing frame N, and a frame costs the busier of the two
threads rather than both added up. `--no-overlap` draws on the main
thread straight after emulating, as before (for comparison; the same
code, run in place).

Following the display, the frames due are the refreshes that went by
since the last redraw (one game frame each at 60 Hz, one per two at
120 Hz; the fraction of a refresh carried over), not one per redraw: a
redraw that comes late, after the surface's wait ran past a vblank, runs
the missed refreshes' frames too (up to `MAX_CATCH_UP`), so the game
keeps its speed and the audio its pitch. A gap over 0.1 s is a stall,
counted as one refresh and left to the audio queue.

The hand-over copies what the renderer reads from the machine shown
(`gpu::BoardFrame`): the geometrizer's raster stream and texture point
RAM, its smooth motion frame and lighting side channel (shared, not
copied), tile RAM (1 MB) and the 3D offsets every frame, and palette RAM
(128 KB) and render RAM (the 7 MB of texture RAM, luma tables and
framebuffers) only when their write counts changed since the last copy
(`Model2Bus::video_ram_versions`; forgotten on a switch between the
game's and run-ahead's machines, a new game, a loaded state). Texture ROM
is shared with the machine. Most frames move 2.3 to 3.5 MB in 0.2 to
0.3 ms; frames that upload textures add the 7 MB. The geometrizer's wait
(a frame drawn with the frame before's polygons if it is late) is part of
the hand-over too, as it was of the drawing. The overlay is laid out on
the main thread (egui's context and input stay there) and painted on the
draw thread. Nothing the machine computes changes: only when it is
drawn.

When to start the next frame: as soon as the draw thread is done with the
last one, as on one thread, or, if that would leave too little of the
refresh for the next frame's emulation (the time recent redraws' took,
plus a tenth and a millisecond), at the last moment that still makes the
next refresh, counted from when the draw thread got the surface's
texture (after any wait for vsync). The main thread waits in the event
loop, so key presses keep coming in and the controls read are as fresh
as on one thread: on a machine fast enough for both in one refresh
nothing changes, and on a slower one the frame's controls are read as
late as the refresh allows. The draw thread wakes the event loop when it
starts drawing and when it is done. A draw that panics (wgpu, egui's painter,
the renderer) is caught on whichever thread drew it (`draw::run_guarded`):
that picture is lost, the error printed and shown once, and the player
carries on; after 120 failed draws in a row it quits cleanly.

Costs are timed per thread: the status line's `draw` is the draw
thread's (`on its thread`), with the main thread's hand-over and any wait
for the draw before (`waited`, 0 while drawing is the quicker part). The
"behind" test (paced by the clock, run-ahead dropped after 3 s) uses the
busier thread's time per frame. The launcher draws on the main thread
(its previews are light).

What it did (60 Hz display, 1280x720 window, RTX 2080 Super, interpreted
i960, native geometrizer, coin-start runs, mean of the status lines from
45 to 66 s, with a build that never drops run-ahead, for measuring; Original is the defaults,
heavy is `--temporal taa --shading pixel --shading-fine on --bloom 50`):

| Game | Settings | Run-ahead | Before: emulate + draw | fps | After: main thread (emulate + hand-over), draw thread | fps |
|---|---|---|---|---|---|---|
| `vf2` | Original | 0 | 3.8 + 4.5 = 8.3 | 60 | 3.8 + 0.8 = 4.6, 4.4 | 60 |
|  | Original | 1 | 9.0 + 4.5 = 13.4 | 60 (clock) | 8.9 + 0.8 = 9.7, 4.5 | 60 |
|  | Original | 2 | 13.3 + 4.7 = 18.0 | 49 (clock) | 13.8 + 0.5 = 14.3, 4.7 | 56 (clock) |
|  | Heavy | 0 | 3.8 + 9.4 = 13.2 | 60 | 3.9 + 1.6 = 5.5, 9.3 | 60 |
|  | Heavy | 1 | 9.2 + 10.6 = 19.8 | 44 (clock) | 9.6 + 1.5 = 11.1, 9.9 | 60 |
|  | Heavy | 2 | 11.3 + 9.9 = 21.2 | 36 (clock) | 14.1 + 0.5 = 14.6, 9.8 | 55 (clock) |
| `srallyc` | Original | 0 | 3.1 + 4.5 = 7.6 | 60 | 3.0 + 0.8 = 3.8, 4.5 | 60 |
|  | Original | 1 | 8.7 + 5.7 = 14.4 | 55 (clock) | 7.4 + 0.8 = 8.2, 4.4 | 60 |
|  | Original | 2 | 11.4 + 4.9 = 16.3 | 59 (clock) | 10.9 + 0.8 = 11.7, 4.5 | 60 |
|  | Heavy | 0 | 3.2 + 8.4 = 11.6 | 60 | 3.0 + 1.3 = 4.3, 8.4 | 60 |
|  | Heavy | 1 | 7.6 + 9.9 = 17.5 | 56 (clock) | 7.9 + 1.6 = 9.5, 8.5 | 60 |
|  | Heavy | 2 | 9.6 + 9.3 = 18.9 | 47 (clock) | 11.6 + 1.5 = 13.1, 8.8 | 60 |
| `lastbrnx` | Original | 0 | 8.7 + 3.3 = 12.0 | 60 | 8.7 + 2.0 = 10.7, 3.3 | 60 |
|  | Original | 1 | 15.2 + 3.3 = 18.5 | 37 (clock) | 17.0 + 0.2 = 17.2, 3.4 | 47 (clock) |
|  | Original | 2 | 16.1 + 3.4 = 19.5 | 22 (clock) | 17.3 + 0.2 = 17.5, 3.5 | 25 (clock) |
|  | Heavy | 0 | 8.3 + 11.6 = 19.9 | 44 (clock) | 8.4 + 2.6 = 11.0, 9.8 | 58 |
|  | Heavy | 1 | 13.1 + 9.4 = 22.5 | 26 (clock) | 17.0 + 0.4 = 17.4, 10.8 | 42 (clock) |
|  | Heavy | 2 | 16.0 + 9.9 = 25.9 | 18 (clock) | 17.2 + 0.3 = 17.5, 10.1 | 24 (clock) |

Milliseconds: `emulate` is the status line's per emulated frame, run-ahead
included; the draw is per picture; fps is pictures a second; "(clock)":
too slow to follow the display in at least one of the status lines, so
paced by the clock (where it catches up two frames to a redraw, the
emulate column is per frame, not per picture). Before is the build
without the draw thread. The hand-over is mostly the geometrizer's wait
(1.2 ms of `vf2`'s heavy 1.5) and 0.2 to 0.3 ms of copying; it shrinks
when the game is behind, since the late frame is drawn without waiting.

What the numbers say: everything whose emulation fits on its own now
keeps 60 fps. `vf2` run-ahead 1 heavy goes from 19.8 ms (44 fps, paced
by the clock, and the player would drop run-ahead) to 11.1 ms on the
main thread beside 9.9 on the draw thread; `srallyc` run-ahead 1 and 2
fit with heavy settings too, and `lastbrnx` with heavy settings and no
run-ahead goes from 44 to 58 fps. Run-ahead 2 on `vf2` (14.6 ms) is
close to the frame and runs a little behind; `lastbrnx` at run-ahead 1
or 2 doesn't fit by its emulation alone (17 ms: its copro SHARC, "Run-ahead
cost" below), which the draw thread can't help.

*Untested by eye*: smooth motion's in-betweens on a display over 60
Hz with the draw thread (they're computed on it, at the time it draws),
and how the hand-over feels in play; the frames presented were checked
headless (`make draw-overlap-check`: `--frame-log` reads every picture
back and hashes it; both ways must draw the same picture for every frame
both drew, and end on the same state hash). Checked on `vf2` (run-ahead
1, 8x multisampling, pixel shading, bloom; 840 frames compared),
`lastbrnx` (the same; 509) and `srallyc` (run-ahead 2, Original; 2,437):
the same pictures and state hashes. Pictures drawn late (the frame
before's polygons) are only reported: on run-ahead's machine which
polygons those are depends on timing whatever the drawing
(`Geometrizer::copy_from` copies the engine, not `last_frame`, so the
machine keeps its own from before the copy), and one boot frame of
`srallyc` differed that way.

### Slowdowns

`--perf-log` (`make play PLAY_ARGS=--perf-log`) prints a `slow` line for
every redraw whose emulation took longer than its frames' share of real
time (17.4 ms a frame), and a `frame times` line on leaving the game:

```
slow          topskatr frame 4211: 1 frames in 23.8 ms (23.8 ms each, 17.4 due); i960 JIT: +41 blocks (0.90 ms installing), 18233 instructions interpreted, 0 dropped, 0 thrown away; copro JITs: +3 blocks, 120 words interpreted, 0 thrown away; 12 jobs on the workers; audio 61 ms queued
frame times   topskatr: 21034 frames emulated, ms: median 4.10, p99 9.80, p99.9 18.20, worst 23.8; over 8 ms 310, over 16.7 ms 9; 9 slow redraws
```

It also times the redraws (`perf::spans`, off without it): one handed
over to drawing more than 1.5 refreshes after the one before prints a
`gap` line with where the time between the two went, and leaving the game
a `redraw gaps` line:

```
gap           topskatr frame 1335: redraw 23.8 ms after the last (2.9 refreshes); ms: surface 18.1, drawing 5.6, other 0.1; mostly surface
redraw gaps   topskatr: 22 of 4683 redraws came over 1.5 refreshes late, worst 80.0 ms; mostly: surface 16, drawing 3, event loop 2, emulation 1; ms in each: surface 8.5, drawing 4.7, event loop 4.1, emulation 1.9, geometrizer 0.1, other 0.3; QoS: game's thread user-interactive, draw thread default
```

The game's thread's time is split into: waiting for the draw thread
(still on the frame before, in `Drawer::finish` or deferred by
`next_start`, below "The frame loop"), as what the draw thread was doing
then: `surface` (`get_current_texture`, which waits for a free image,
so for the display), `present`, or `drawing`; `event loop` (idle until
it delivered the redraw); `emulation` (the frames due and run-ahead's);
`geometrizer` (waiting for its frame, at the hand-over and in emulation);
and `other` (the rest of the redraw: the overlay, the copy). With
`--no-overlap` the surface's wait and the present are the game thread's
own. On macOS the summary gives each thread's quality-of-service class.
The first redraw's gap is the game's start. The `gpu` line at start-up
gives the surface's present mode (vsync: FIFO relaxed where offered, else
FIFO), the modes it offers and its frame latency, for a report.

(The numbers are an example.) A slowdown that is the JIT's shows as
blocks arriving and code interpreted while they are made, jobs queued on
the workers, or translations dropped (the game rewrote code it had) or
thrown away; one with none of those is the game, the geometrizer or the
machine. The sound slows with the game only when the game stays behind
for a while (`model2::pacing::Pace`); a single long redraw is taken up by
the audio queue.

`--sound-log FILE` writes every byte the game sends its sound boards, a
line each (`topskatr 3144 1234567890 ae`: set, game frame, bus cycle,
byte; `ctl` for control-register writes). When a sound or a music track
is missing, it shows whether the game sent the command (a DSB2 music
board's track is `ae 10 nn`, docs/hw/sound.md)
or the board lost it.

The translation workers run below the game's threads: a nice of 10 on
Linux, the "utility" quality-of-service class on macOS (which keeps them
off the performance cores the game wants), below-normal priority on
Windows (`jit::lower_priority`); so does `--jit-warm`'s background run.

### Run-ahead cost

Measured with `make run-ahead-profile GAME=g RUN_AHEAD_FRAMES=n` (the
CLI's `run-ahead-profile`: the player's loop headless, 1,000 frames after
3,000 of a coin-start run, interpreted i960, native geometrizer on its
thread, 16-core desktop, each figure the mean of two runs; run-to-run
noise is about 10%, more for `lastbrnx`). Milliseconds per shown
frame:

| Game | Run-ahead | Real frame | Geometrizer wait | Copy | Frames ahead | Total |
|---|---|---|---|---|---|---|
| `srallyc` (2A) | 0 | 8.2 | 0 | 0 | 0 | 8.2 |
| | 1 | 8.4 | 0.5 | 0.9 | 8.6 | 18.4 |
| | 2 | 8.4 | 0.5 | 0.9 | 17.3 | 27.1 |
| `lastbrnx` (2B) | 0 | 14 | 0 | 0 | 0 | 14 |
| | 1 | 14.2 | 1.7 | 1.0 | 14.5 | 31.4 |
| | 2 | 14.3 | 1.7 | 1.0 | 30.7 | 47.7 |
| `vf2` (2A) | 0 | 6.8 | 0 | 0 | 0 | 6.8 |
| | 1 | 7.0 | 0.5 | 0.8 | 7.2 | 15.5 |
| | 2 | 6.8 | 0.5 | 0.8 | 14.3 | 22.4 |

Drawing is extra, once per shown frame whatever the run-ahead: 2 to 4.6
ms (Original, the three games) and 8 to 10.5 ms (4 to 8 sample
anti-aliasing 8, temporal anti-aliasing and fine pixel shading), on the
same thread as the emulation (a `--run-ahead 0` player run on an RTX
2080 Super, default window; the figure includes waiting for
the GPU). With the heavy settings `srallyc` needs 7.7 + 8 ms before
run-ahead adds anything, which is why the player drops run-ahead there.
(That was before the draw thread: now the drawing runs alongside the
emulation, above, "The frame loop".)

What the numbers say:

- **Run-ahead N costs N + 1 game frames plus the copy.** Copy and waits
  are 1.3 to 2.7 ms of it (3 to 9% at run-ahead 1); the frames dominate.
- **Nothing is thrown away except the frames themselves.** The shadow's
  frames are never drawn (only the last is), its sound commands are
  dropped before any board runs (the sound boards belong to the player and
  are fed from the real machine only) and the real machine's frames are
  not drawn. What the shadow does repeat is the geometrizer (worker busy
  0.4 ms `srallyc`/`vf2`, 1.6 ms `lastbrnx` per frame, on its thread; it
  shows as the wait columns).
- **The copy is already dirty-tracked** (per memory, `CopyMarks`): it
  moves 3.4 to 3.6 MB of a 24.3 MB (2A) or 16.5 MB (2B) save state, the
  same every frame (the buffer and tile RAMs the game rewrites each
  frame), in 0.8 to 1.0 ms. A save and load would be 6 to 9 ms and 2 to
  4 ms.
- **Most of a game frame is the i960 waiting.** The poll loops the games
  wait for the next frame in (`srallyc` 0x5A379C, five instructions;
  `lastbrnx` 0x73F4, four; `vf2` 0x10F98, two) were 105 to 126 thousand of
  each frame's 145 thousand i960 instructions over the 3,500 frames run
  (72 to 87%; a hot-spot sample of one frame says the same) and, timed
  per pass of the loop, 6.1 ms of `srallyc`'s 8.2 ms frame, 3.6 of
  `vf2`'s 6.7 and 4.6 of `lastbrnx`'s 13 (the 2B runs its SHARC per
  instruction in the loop too, so less of it is saved there).
  `docs/games/srallyc.md` says 18%: that was not what this build does in
  attract mode or a scripted race (frames 4,000 to 5,000).
- **Inputs change rarely, for digital controls.** The coin-start fighter
  script (a punch and kick pulse every 30 frames) changes the inputs on
  13.4% of frames (`lastbrnx`, `vf2`). A person at a fighter presses
  and releases a few buttons a second, so 7 to 15% is a fair estimate (not
  measured). The keyboard's wheel is a full-lock switch, so it changes
  only on presses; a gamepad's analogue stick changes on most frames while
  steering (estimate), where an unchanged-input path would not hold.

Ranking, by expected saving:

1. **Idle skip** (option 4): 35 to 75% of every frame, run-ahead's or
   not, for all three games, and it multiplies with everything below (the
   shadow's N + 1 frames shrink too). It must keep the instruction count,
   bus time, timers, the TGP and, on 2B, the SHARC running through the skipped
   passes to keep the state hash, so it is per loop, to the poll's own
   exit (vblank status). Expect `srallyc` run-ahead 1 from 18 ms to about
   7 and `vf2` from 15.5 to about 8; `lastbrnx` from 31 to about 20.
   **Done** (`docs/idle-skip.md`), and it came in as expected; below.
2. **Unchanged-input fast path on two threads** (option 1): with the
   inputs as the last frame's the shadow runs one frame and the real
   machine runs its own frame at the same time, so a frame costs about
   one game frame of wall time (and no copy) instead of N + 1. At 13%
   changes, run-ahead 1 on a fighter goes from 2.1 frames to about 1.2,
   run-ahead 2 from 3.1 to about 1.4. On analogue steering it saves
   nothing. Worth it for the fighters (the games that offer run-ahead 1
   with typically digital controls); needs two free cores, and a
   determinism test.
3. **Overlap drawing with the next emulated frame** (not on the list):
   2 to 10 ms a frame on the main thread, which the heavier presets make
   as large as the emulation; the GPU work of frame n can run alongside
   frame n + 1's emulation. Saves up to the draw time. **Done**: the
   drawing is on its own thread, above, "The frame loop".
4. **Skip work in the ahead frames** (option 2): drawing and sound are
   already skipped. The geometrizer can't be skipped without diverging
   the shadow (its output feeds polygon RAM and the lists), so what's left
   is the wait columns, 0.5 ms (1.7 on `lastbrnx`), by starting the copy
   earlier or keeping the geometrizer's frame off the copy's path: 3 to
   6% of run-ahead 1.
5. **Cheaper save and restore** (option 3): the copy is 0.8 to 1.0 ms of
   15 to 31, 3 to 5%, and already moves a seventh of the state. Not worth
   more.

**With the idle skip** (2026-11-13, the same runs, same machine;
milliseconds per shown frame, the total with the waits and the copy,
which didn't change; before: the build before the skip):

| Game | Run-ahead | Real frame before | after | Total before | after | Skipped |
|---|---|---|---|---|---|---|
| `srallyc` | 0 | 8.0 | 2.7 | 8.0 | 2.7 | 80% |
| | 1 | 8.3 | 2.8 | 18.1 | 6.9 | |
| | 2 | 8.3 | 2.8 | 26.7 | 10.1 | |
| `lastbrnx` | 0 | 13.8 | 8.6 | 13.8 | 8.6 | 78% |
| | 1 | 13.6 | 8.7 | 29.7 | 20.2 | |
| | 2 | 13.5 | 8.8 | 44.8 | 30.5 | |
| `vf2` | 0 | 6.7 | 3.5 | 6.7 | 3.5 | 72% |
| | 1 | 6.9 | 3.5 | 15.2 | 8.4 | |
| | 2 | 6.9 | 3.5 | 22.8 | 12.3 | |

Run-ahead 1 now fits a 17.4 ms frame on all three, and run-ahead 2 on
`srallyc` and `vf2`. Less is saved on `lastbrnx` (2B): its copro SHARC
still runs through the skipped passes, five steps for each i960
instruction credited.

## The launcher

The start screen, when no game is given, and from the menu's "Back to
games". The games found in the ROM folder are a row of cards along the
bottom (each game's poster, `assets/posters`, and title), grouped under
headings: the recently played first, then each board (Model 2B, 2C); the
groups are also tabs along the top. The selected card's game fills the
rest: its live attract mode on the left, in the window's shape (what the
game will look like, bezel included with 4:3), and on the right its board,
year, title, set name, controls (the manifest's default keys) and buttons:
Play, or Continue and New game when it was left part way, and Settings.
Behind everything a neon wireframe grid floor scrolls slowly towards a
horizon of wireframe mountains (a shader, `crates/play/src/backdrop.wgsl`,
in the logo's black, cyan and blue), with the selected game's bezel art
faded in over it. The row slides to keep the selected card in view and
scales it up; at the ends it stops at the margins. Everything is laid out
on a 1600x900 design scaled to the window (on top of the display's own
scaling), so it reads the same at 1080p, a 3440x1440 ultrawide and 4K.

| | Keyboard | Gamepad | Mouse |
|---|---|---|---|
| choose a game | Left, Right | d-pad, left stick (repeat while held) | click a card, wheel |
| choose a button | Up, Down | d-pad, stick | |
| play (the button chosen) | Enter, Space | A, Start | click the button, or the selected card |
| settings | Tab | Y | Settings |
| previous, next group | Page Up, Page Down | shoulder buttons | the tabs |
| back (close the settings) | Esc, Backspace | B, Back | |

F11 toggles fullscreen, as in a game. *Unconfirmed:* the gamepad
navigation is tested with synthetic events only, not yet with a pad.

**Live previews.** Once a card has been selected for half a second, its
game starts in the background and its attract mode shows in place of the
poster, fading in, its sound fading in 9 dB under the game's mix. It starts
from a save state at the manifest's `attract_frame` (the frame `remodel
shot` takes for the browser's game list), so it's live almost at once:
the chips and the state load on a thread of their own, then the machine is
built and the state loaded on the main thread (about 100 ms, the one
pause). The state is made the first time a game is previewed, by running
it headless from power-on to that frame with its sound boards and a fresh
EEPROM (never the player's), while the card shows the poster and how far
it has got; it's kept as `<data dir>/<set>/preview-native.save` (or
`preview-chip.save` with the chip geometrizer), in the save state format,
and made again if it no longer loads. Making it takes as long as running
the game that far as fast as it goes: recompiled, 2-4 s for `stcc`, `sgt24h`
and `topskatr`, 12 s for `hotd`, 19 s for `fvipers`; interpreted, 12 s for `topskatr` and 15 s for `doa`. Once cached, a preview shows about 0.6 s after its card is selected (the half second, 20-40 ms loading, 75-115 ms starting). One state
is made at a time, and it carries on if the selection moves to a game
whose state is ready; selecting another game that needs one stops it. Only
one preview runs: moving on fades it out over 0.2 s and drops it. A game
that can't start (chips missing from the ROM folder) keeps its poster and
says why. The preview never takes input, never saves its EEPROM, and is
emulated like the game would be (it costs what the game costs).

**Settings** slide out from the right over the preview, which shows what
they change as they change (except anti-aliasing, temporal anti-aliasing,
the depth buffer, bloom, shading, the texture pack and its filtering and
material maps, which the preview draws without; they show in the game): aspect (fit the window
or 4:3), bezel, HUD at the edges or centred, speed (one frame a refresh or
exact arcade speed), smooth motion, run-ahead (off, 1, 2, 3), anti-aliasing (off, 4x, 8x), temporal anti-aliasing (off,
TAA, 75%, 67%, 50%) and its checkerboard see-through (blend or keep), the depth buffer, shading (flat,
vertex, pixel; with the chip geometrizer pixel says it draws as vertex), its brightness levels (the game's or blended) and its brightness balance (on or off), the texture pack and its filtering (off, mip, 2x to 16x), material maps (off or on; no effect without the pack or with the chip, said in place of the hint) and their relief (1, 2, 4, 8 or faithful; another strength from the settings file or the command line as "Custom (S)"), bloom (off, light, medium, strong; another percentage from a flag or file as "N%"; not offered on a GPU without 16-bit float targets), the effects and music levels in dB, the geometrizer (native
or the game's chip; the preview restarts on it) and the Controls page
(remapping, below). "Save for" says where changes go: the defaults for all
games, or this game only (a game's own setting is tagged "this game", and
"Use the settings for all games" forgets them). The same settings are in
the Esc menu while playing, in tabs and behind a picture preset (below,
"The menu"). The launcher lists them as one scrolling list in the menu's
order (display, picture, motion, sound, more) and with the menu's rules,
sharing its code (`hints.rs`): a **Preset** row (Original, Sharp,
Enhanced, Custom, with the menu's help line for the game) comes first of
the picture rows, which only Custom lists; rows that do nothing as set
(anti-aliasing under temporal, the lighting rows with flat shading, the
chip's, the bezel in a wide window or for a game without art, texture
filtering without the pack) are left out; the run-ahead row stops at the
game's maximum and is gone for a game with no lag; and its hint says what
this computer has time for (above). Choosing a preset sets its rows and a
picture row changed afterwards makes it Custom, the same as in the menu, and
what is saved from either place is identical (`preset = enhanced` and so
on). The crease angle is only in the menu. They come in layers, lowest first, as in the
browser build: the built-in defaults, the defaults for all games, the
game's own, then the command line (`--mix`, `--hud`, `--aspect`, `--bezel`,
`--exact`, `--smooth`, `--run-ahead`, `--aa`, `--temporal`, `--stipple`, `--depth`, `--shading`, `--crease`, `--shading-fine`, `--shading-balance`, `--pack`, `--texture-pack`, `--tex-filter`, `--materials`, `--materials-relief`, `--bloom`, `--geo`), which applies to that run only and is
never saved, unless a setting it gave is changed in the launcher or menu
(that change is then kept as any other). A layer keeps only what it sets.

With the HUD at the edges, 3D parts of the HUD move with it: a game's manifest lists the
3D windows that hold HUD (`hud_windows`: each polygon moves by its third of the
screen, `topskatr`'s speed gauge) or HUD among other 3D (`hud_element_windows`: a
polygon moves with the 2D HUD element whose box contains its centre and stays
put otherwise, `srallyc`'s rev counter needle, which is a polygon of its own,
with its dial a tile element, while the same window holds the cars at car
select).

**Continue.** Leaving a game (quitting, "Back to games", or starting
another) keeps a resume state, `<data dir>/<set>/saves/resume.save` (the
F5/F9 save format, sound boards included), unless it was a scripted
`--input` run. Its card then offers Continue (with how long ago) beside New
game, which starts from power-on as before; the state is replaced the next
time the game is left.

`--shot FILE --seconds N` draws the launcher too when it's what shows (and
the menu, when it's open over a game), and `--nav` scripts gamepad input
for that: `--nav next@1,settings@2,right@3` names a pad button and the
second it's pressed (left, right, up, down for the d-pad; accept, back,
settings for A, B, Y; prev, next for the shoulder buttons; start, select,
guide). It's released 0.08 s later, or after `:H` seconds
(`down@3.5:2` holds it, repeating); `start+select@3` presses two at once.
E.g.
`remodel-play --no-sound --seconds 6 --shot-size 3840x2160@2 --nav settings@3 --shot out/launcher.png`,
or the menu over a game, left from the pad:
`remodel-play --game topskatr --no-splash --no-sound --seconds 10 --nav start+select@3,down@3.5:3,left@7,accept@7.5 --shot out/back.png`.
`--frames N` ends the run when the game reaches frame N (it stops there),
so `--shot` pictures that frame: with `--input coin-start` the same frame
the CLI's `--capture` saves, for `gpu-check` to draw beside it.
It's drawn with egui (`crates/play/src/launcher.rs`), its layout and
navigation tested headless (`cargo test -p remodel-play`).

**In the app menu.** `make install-desktop` (`RECOMP=1` for the recompiled
build) builds the player and adds reModel to the desktop's app menu: a
`remodel.desktop` entry in `~/.local/share/applications` and the icon
(`assets/reModel-icon.png`, the logo on a square) in the hicolor theme.
The entry runs this checkout's player from the checkout's directory, so it
finds `rom/` and keeps its files in `out/` as `make play` does; rebuild
with `make play` (or rerun `install-desktop`) and the menu runs the new
build. The window's app id (Wayland) and class (X11) are `remodel`, which
the entry names, so the window gets the icon. `make uninstall-desktop`
removes both.

## Keys

The player's own keys are the same for every game:

| Key | |
|---|---|
| Esc | menu (the game pauses while it's open) |
| F5, F9 | quick save, quick load |
| F8 | photo mode (docs/photo-mode.md) |
| F11 | fullscreen |

A gamepad opens and closes the menu too: its Guide button (the logo
button, often taken by Steam or the system), or Start and Select (Back)
pressed together, or held together for half a second. Neither reaches
the game then, and buttons pressed in the menu stay away from the game
until they're released, so the Start that resumed doesn't also start a
game. Start or Select alone still goes to the game (the cabinet's start
button, and the coin on most games' default bindings), 0.1 s late: a
press waits that long to see whether it's the pair (a tap shorter than
that reaches the game for 0.1 s afterwards). While one is held, the other
waits until it's released or the two have been held long enough to open
the menu. The pad is `crates/play/src/pad.rs` (`Gate`), tested with
synthetic events.

Each game's controls, and their default keys and gamepad bindings, are in
its manifest (`crates/romset/src/games/<set>.rs`, `Controls`); the native
player, the browser build and scripted `--input` all read them.
`make inputs GAME=<set>` lists them. Gamepads use the standard layout
(A/B/X/Y named as on an Xbox pad); the one used last drives the game.
Players can remap them per game (below, "Remapping"); the tables here are
the defaults.

### Digital steering

Keys and pad buttons on an analog input (Left and Right on a wheel, the
d-pad, the browser's touch arrows) hold it at full deflection while
they're down: a switch. That suits most games, whose own code eases the
wheel in. A game that steers straight from the wheel's position gets a
`KeyRamp` on that input in its manifest (`romset::AnalogInput`): the keys
then turn it out to full deflection over `attack` frames and let it back
to the centre over `release` (also on the way through to the other side),
as a hand on a wheel would. A stick or trigger is never ramped: its
position is the player's. `crates/model2/src/input.rs` (`KeyRamps`) does
it, for the native player and the browser alike; scripted `--input`
values are raw and not ramped. `daytona` has one (12 frames out, 6 back:
`docs/games/daytona.md`, "the handling"); the other games' keys are
switches.

### Top Skater (`topskatr`)

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| start | Enter | Start |
| steer (`curving`) | Left, Right | left stick, d-pad |
| slide | Down, Space | right trigger, B |
| jump front, jump tail | Z (or Up), X | A, X |
| select left, right | Q, E | shoulder buttons |

### Sega Ski Super G (`skisuprg`)

One credit is two coins (the game's default setting). The skis' two angle
inputs (`edge`, `swing`) both follow the left stick, which is how the
game's own demo drives them; full deflection on the keyboard turns hard.

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| blue panel button: starts a game, left choice | Enter, A | Start, X |
| red, green panel buttons: middle, right choice | S, D | B, A |
| turn (`edge` and `swing`) | Left, Right | left stick, d-pad |
| `push` (unconfirmed what it does) | Up, Down | right trigger minus left trigger |
| select left, right, decide | Q, E, W | shoulder buttons, Y |

The game needs its drive board (the platform motor); reModel answers its
boot check and commands as an idle board at rest (`crates/model2/src/drive.rs`).

### Sega Water Ski (`segawski`)

One credit is two coins. Slide the platform to steer; SET starts a game
and confirms on the select screens, the select buttons move the choice.

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| set: start, confirm | Enter, S | Start, A |
| select up, down | W, X | d-pad up, down |
| steer (`slide`) | Left, Right | left stick, d-pad |
| pitch left, right (unconfirmed what they do) | Q, E | left, right shoulder |

### Sega Touring Car Championship (`stcc`)

Sound effects and music (the DSB1 music board, `docs/hw/dsb1.md`). One coin
is one credit. On the menus the wheel moves the selection and the gas
pedal confirms; hold the brake while confirming the car to get the manual
gearbox (MT), which the shift buttons step through.

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| start | Enter | Start |
| steer (`wheel`) | Left, Right | left stick, d-pad |
| gas, brake | Up, Down | right trigger, left trigger |
| shift up, shift down | A, Z | right shoulder or B, left shoulder or X |
| view 1, view 2 | V, B | Y, A |

The wheel's force feedback board is commanded every frame but the game
never checks its reply; reModel keeps the commands
(`crates/model2/src/drive.rs`) for a future gamepad rumble.

### Wave Runner (`waverunr`)

All its sound comes from the SCSP sound board (no music board). Three coins are
one credit. On the course select screen the handlebar moves the selection
and the throttle confirms. The ride-on cabinet's body (roll and pitch
sensors, compressor and valves) isn't needed: the game plays as if no
cabinet board were attached.

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| start | Enter | Start |
| steer (`handlebar`) | Left, Right | left stick, d-pad |
| throttle | Up, Z | right trigger, A |
| view (unconfirmed what it changes) | V | Y |
| body roll (`roll`, no effect seen yet) | A, D | right stick |

### Over Rev (`overrev`)

All its sound, music included, comes from the SCSP sound board, run by
Jaleco's own driver (no music board). One coin is one credit. On the
menus the wheel moves the selection and the gas pedal confirms: mode,
car, then the gearbox (AT or MT; the shift buttons step through MT's
gears).

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| start | Enter | Start |
| steer (`wheel`) | Left, Right | left stick, d-pad |
| gas, brake | Up, Down | right trigger, left trigger |
| shift up, shift down | A, Z | right shoulder or B, left shoulder or X |
| view (chase, cockpit, bumper) | V | Y |

### Super GT 24h (`sgt24h`)

Model 2B, Jaleco, like `overrevb`: its sound is the SCSP's, run by
Jaleco's driver. One coin is one credit. On the menus the wheel moves the
selection and the gas pedal confirms: course, car, then the gearbox (AT
or MT; the shift buttons step through MT's gears). It needs the
recompiled build (`RECOMP=1`) for full speed in races, as `overrevb` does.

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| start | Enter | Start |
| steer (`wheel`) | Left, Right | left stick, d-pad |
| gas, brake | Up, Down | right trigger, left trigger |
| shift up, shift down | A, Z | right shoulder or B, left shoulder or X |
| view | V | Y |

### Dynamite Baseball '97 (`dynabb97`)

Model 2B, Sega's own: its sound is the SCSP's, run by CRX_drv. One coin
is one credit. Button 1 confirms on the menus (team, then starting
pitcher). At bat, the stick moves the hitting point and the bat lever
swings: hold Space (or the right trigger) to pull it back and let go to
swing. It needs the recompiled build (`RECOMP=1`) for full speed, as the
other 2B games do. Player 2's controls are in the manifest but have no
default keys.

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| start (player 1, 2) | Enter or 1, 2 | Start |
| stick | arrow keys | d-pad |
| button 1, button 2 (`sw1`, `sw2`; what 2 does is unconfirmed) | Z, X | A, B |
| bat (pull back, let go to swing) | Space | right trigger |

### Dead or Alive (`doa`)

Model 2B, Tecmo's: its sound is the SCSP's. One coin is one credit. The
stick picks the character and punch confirms. In a fight the three
buttons are hold, punch and kick. Its protection device is
answered by the board model, so nothing needs setting up. Like the other
2B games it needs the recompiled build (`RECOMP=1`) for full speed: a
headless coin-start run manages 2.7x real time recompiled, but only about
0.8x interpreted (the player then paces it by the clock, see "Display"
below). Player 2's controls are in the manifest but have no default keys.

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| start (player 1, 2) | Enter or 1, 2 | Start |
| stick | arrow keys | d-pad |
| hold, punch, kick | Z, X, V | X, A, B |

### Indianapolis 500 (`indy500`)

Model 2B, Sega's own, the Twin cabinet: its sound is the SCSP's. Two
coins are one credit. On the menus the wheel moves the selection and the
gas pedal confirms: course (the oval, the street course or the road
course), then the gearbox (AT or MT; the shift buttons step through MT's
gears). The two view buttons step the camera in and out through its four
views. It runs stand-alone (NETWORK TYPE in the test menu's game
assignments). Like the other 2B games it needs the recompiled build
(`RECOMP=1`) for full speed; interpreted, a headless coin-start run
manages about 1.1x real time with slow frames in races.

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| start | Enter | Start |
| steer (`wheel`) | Left, Right | left stick, d-pad |
| gas, brake | Up, Down | right trigger, left trigger |
| shift up, shift down | A, Z | right shoulder or B, left shoulder or X |
| view in, view out (`view1`, `view2`) | V, B | Y, A |

### Fighting Vipers (`fvipers`)

Model 2B, Sega's own: its sound is the SCSP's. One coin is one credit.
The stick picks the fighter and punch confirms. In a fight the three
buttons are punch, kick and guard. Like the other 2B games it needs the
recompiled build (`RECOMP=1`) for full speed: a
headless coin-start run manages 2.8x real time recompiled, but only about
0.8x interpreted (the player then paces it by the clock, see "Display"
below). Player 2's
controls are in the manifest but have no default keys.

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| start (player 1, 2) | Enter or 1, 2 | Start |
| stick | arrow keys | d-pad |
| guard, punch, kick | Z, X, V | X, A, B |

### Sonic Championship (`schamp`)

Model 2B, Sega's own: its sound is the SCSP's. One coin is one credit.
The stick picks the fighter and punch confirms. In a fight the three
buttons are punch, kick and barrier. Like the other 2B games it needs the
recompiled build (`RECOMP=1`) for full speed: a
headless coin-start run manages 2.7x real time recompiled, but only about
0.8x interpreted. Player 2's
controls are in the manifest but have no default keys.

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| start (player 1, 2) | Enter or 1, 2 | Start |
| stick | arrow keys | d-pad |
| barrier, punch, kick | Z, X, V | X, A, B |

### Virtua Striker (`vstriker`)

Model 2B, Sega's own: its sound is the SCSP's. One coin is one credit.
The stick picks the team and short pass confirms. In a match the stick
runs, and the three buttons are short pass (also the sliding tackle when
defending), long pass and shoot (held, shoot charges a power bar). Like
the other 2B games it needs the recompiled build (`RECOMP=1`) for full
speed: a headless coin-start run manages 1.8x real time
recompiled, but only about 0.7x interpreted. Player 2's controls are in
the manifest but have no default keys.

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| start (player 1, 2) | Enter or 1, 2 | Start |
| stick | arrow keys | d-pad |
| short pass, long pass, shoot | Z, X, V | A, X, B |

### Last Bronx (`lastbrnx`)

Model 2B, Sega's own: its sound is the SCSP's. One coin is one credit.
The stick picks the fighter and punch confirms. In a fight the three
buttons are punch, kick and guard (the corner of the screen where a
second player would join offers "P+K+G start survival mode"). Like the
other 2B games it needs the
recompiled build (`RECOMP=1`) for full speed: a headless coin-start run
manages 2.9x real time recompiled, but only about 0.7x
interpreted. Player 2's controls are in the manifest but have no default
keys.

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| start (player 1, 2) | Enter or 1, 2 | Start |
| stick | arrow keys | d-pad |
| guard, punch, kick | Z, X, V | X, A, B |

### Sega Rally Championship (`srallyc`)

Model 2A, Sega's own: its sound is the SCSP's. Two coins are one credit
(the game's default setting; the scripted coin-start pays twice). On the
menus the wheel moves the selection and the gas pedal confirms; the car
select offers AT and MT, which the wheel picks. A manual (MT) car has an
H-gate shift lever, which the player **latches**: a gear key puts the
lever in that gear and leaves it there, and the shift keys step through
the gate (neutral, 1, 2, 3, 4; down from first is neutral, up from neutral
is first). The automatic needs none of them. The game runs at about 1.4x
real time interpreted, recompiled alike, so it needs no recompiled build.

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| start | Enter, 1 | Start |
| steer (`wheel`) | Left, Right | left stick, d-pad |
| gas, brake | Up, Down | right trigger, left trigger |
| gear 1, 2, 3, 4 (latched) | Q, W, E, R | |
| neutral | X | |
| shift up, shift down (latched) | A, Z | right shoulder or B, left shoulder or X |
| view | V | Y |

### The House of the Dead (`hotd`)

A light-gun game for two. Two coins are one credit. The mouse is player
1's gun: the window shows a crosshair pointer, the left button fires and
the right button reloads (the game reloads when you shoot off the screen;
moving the pointer off the picture and clicking does the same). With a
wide window (16:9 and up) the extra area at the sides counts as off the
screen. A gamepad's sticks move a crosshair drawn over the game: it's player
2's gun once the mouse has moved over the window, else player 1's.

| Control | Keys | Mouse | Gamepad |
|---|---|---|---|
| coin | C, 5 | | Back |
| start (player 1, 2) | Enter or 1, 2 | | Start |
| aim | | move | either stick |
| fire (`trigger1`) | Space | left button | right trigger, A |
| reload | R | right button, or fire off the picture | left trigger, B |

The game's own calibration (test menu, GUN SETTING) works on what
reModel reports; with a fresh EEPROM the shots land where the pointer is.

### Behind Enemy Lines (`bel`)

Two gunners on mounted guns while the vehicle drives itself. One coin is
one credit. The mouse is player 1's gun as in `hotd`, but the guns are
mounted (analog yaw and pitch), so they can't point off the screen: the
right button fires a missile instead of reloading, and leaving the
picture keeps the last aim. A gamepad's sticks aim as in `hotd`.

| Control | Keys | Mouse | Gamepad |
|---|---|---|---|
| coin | C, 5 | | Back |
| start (player 1, 2) | Enter or 1, 2 | | Start |
| aim | | move | either stick |
| fire (`trigger1`) | Space | left button | right trigger, A |
| missile (`missile1`) | X | right button | left trigger, B |

With a fresh EEPROM the crosshair follows the pointer; the game's GUN
CALIBRATION in the test menu changes that.

## The menu

Esc opens the pause menu (and closes it again). It is a short list:
**Resume**, **Save / Load**, **Settings**, **Controls**, **Photo mode** (as
F8: `docs/photo-mode.md`), **Back to games** and **Quit**. Nothing else sits beside Resume: the game's settings are one
level in. A small "About: reModel <version>" line sits at the foot.

From a gamepad: the d-pad or left stick moves between the buttons and rows
as they're laid out (repeating while held, as in the launcher), A presses
the one that glows cyan, left and right change a settings row, LB and RB
switch settings tabs, B or Select goes back one screen (from the pause menu
it resumes) and Start resumes from anywhere. The menu opens on Resume. The
keyboard does the same: the arrow keys, Enter or Space for A, Q and E
(or Tab and Shift+Tab) for the tabs, Backspace for B; Esc closes the whole
menu. The focus glows while the pad or those keys are in use, and the foot
shows the pad's buttons while the pad is. `crates/play/src/focus.rs` does
it for any egui page; the menu's tests (`menu.rs`) drive it with `Nav`s
headless. *Unconfirmed:* tested with synthetic and scripted (`--nav`)
events only, not yet with a real pad.

- **Save / Load**: a screen of its own with the quick slot (F5/F9) and four
  more. A save holds the whole machine, both sound boards included, so
  loading one resumes the sound exactly where it was. The one thing not
  saved is a replacement soundtrack voice that's playing (`docs/mods.md`).
- **Settings**, in tabs. Each row reads "label ◂ value ▸": left and right
  change it (A or a click on the name steps on, round to the first), and a
  single help line at the foot says what the row in focus, or under the
  pointer, does. A row that does nothing as things are set is left out, not
  greyed (anti-aliasing while temporal is on, temporal with the chip
  geometrizer or on a GPU that can't, the shading options with flat
  shading). Every tab ends with **Applies to: this game / all games**,
  where the changes are kept (as in the launcher).
  - **Display**: the aspect (fit the window, the default, drawing the game
    wider than its frame from 4:3 to 32:9; or 4:3, `--aspect 4:3`), where
    the HUD goes on wide windows (shown for fit the window), the bezel
    (below; shown for 4:3, where there is art), and fullscreen (as F11).
  - **Picture**: starts with the preset row, **Original**, **Sharp**,
    **Enhanced** or **Custom** (below). Only Custom lists the individual
    rows: anti-aliasing, temporal anti-aliasing and the checkerboard
    pattern, the depth buffer, shading with its crease angle, blended
    brightness levels and brightness balance, the texture pack and, with
    it, material maps and their relief (the help line says what they did or
    why they have no effect: above). The crease angle is "the game's" or
    20 to 60 degrees; other values come from the settings file or
    `--crease`.
  - **Motion**: exact arcade speed (57.524 frames per second) against the
    default of one game frame per display refresh at 60 or 120 Hz, which is
    4.3% fast but smooth; smooth motion (not with the chip); run-ahead
    (above; only up to the game's useful maximum, and not at all for a game
    with no lag to hide).
  - **Sound**: the effects (sound board) and music (DSB2 or DSB1) levels in
    dB; the default 8:-8 was set by ear (`docs/hw/sound.md`).
  - **More**: the geometrizer (native or the game's chip, on the next
    start) and texture filtering (of the texture pack's images, with the
    pack on).

  **Picture presets.** *Original* is the arcade look and every default:
  nothing that changes the picture (and no smooth motion or run-ahead).
  *Sharp* is 4x multisampling and mipmapped texture filtering, nothing
  else. *Enhanced* is per game (`romset::Recommended` in the manifest, the
  evidence in each game's `docs/games/<set>.md`): temporal anti-aliasing
  at full size (4x multisampling where temporal misbehaves, `vstriker`; and
  kept as the fallback with the chip geometrizer or a GPU without temporal),
  the depth buffer only for the games whose depth-check pictures it fixed
  and none it broke, smooth shading (vertex, the game's crease) for the
  games it flatters, smooth motion, mipmapped filtering and run-ahead at
  the game's useful maximum (below; 0 for `schamp` and `lastbrnx`, whose
  stick has no lag to hide). Choosing a preset sets those rows; changing
  any picture row afterwards makes the preset *Custom* and keeps the values.
  The preset is kept in the settings files as itself (`preset = enhanced`)
  and its rows are worked out per game, so Enhanced kept for all games is
  each game's own; a file with rows and no preset (from before presets)
  shows as Custom.
- **Below full speed** (a game frame taking longer than a refresh, as
  `overrevb`'s interpreted SHARC geometrizer does in races) the player
  doesn't let the game drop to half speed with the refresh rate: it paces
  by the clock instead, running frames as fast as they come (the title
  bar says "too slow to sync, paced by the clock"), and goes back to the
  display once frames fit again. When the geometrizer isn't done in time
  for a refresh, that frame is drawn with the previous frame's polygons
  (counted in the title bar), so the geometrizer runs alongside the next
  frame instead of after it: polygons then lag the 2D layers and inputs
  by a frame, as in the browser.
  The audio follows the frames the game actually runs
  (`model2::pacing::Pace`, described in `docs/web.md`, "Audio rate"): once
  the game has been below full speed for 0.15 s (three times the 50 ms
  queue) it slows with it, lower in pitch, instead of running dry; while
  it keeps up only the queue's drift correction (at most 1%, inaudible)
  moves the pitch, so a frame dropped now and then is absorbed by the
  queue rather than heard. Two things keep a game that sits near its
  frame budget from bending the pitch (found on `vf2` with the Enhanced
  picture settings, whose draw alone takes 9-12 ms in a fight on top of
  7 ms of emulation, `docs/games/vf2.md`): the queue's target grows from
  50 ms to at most 150 ms (1.25 times the longest gap between redraws in
  the last 8 s) when redraws come unevenly, and the rate control's
  patience grows with it; and a gap longer than 0.1 s between redraws is
  a hitch, a pause rather than a slow game, so it isn't counted as
  frames missed (a 0.2 s hitch used to count as 3 frames run of 11 due
  and pulled the pitch down by up to 40% for seconds). Before,
  only a 0.5% drift correction steered the 50 ms queue, so `sgt24h`'s race
  start (its geometrizer at 18.7 ms a frame natively then, before the
  SHARC blocks; 6.8 ms now) would run it dry
  every few seconds, modelled from its frame costs; the title bar counts
  underruns.
- **Controls**: remap the game's keys and gamepad bindings, below.
- **Back to games**: leave the game for the launcher (keeping a resume
  state for Continue). The launcher lists the supported sets in the ROM
  folder, read as MAME keeps them: `<set>.zip` or a `<set>/` folder, and a
  supported clone is listed beside its parent when its chips are in the
  parent's (`overrevb` in `overrev.zip`). With none found, it says where
  it looked and a ROM folder can be typed in. The ROM folder defaults to
  `rom/` in the source tree (or wherever the current directory has a
  `rom/` folder), else to `roms/` beside the settings
  (`~/.local/share/remodel/roms`), created on first run; `--rom` changes
  it (and is remembered).

## Remapping

The pause menu's Controls page lists every input of the game being played with
its keys and gamepad bindings. Click a binding, then press the key or
gamepad button that should replace it; **+** adds one (an input can have
several), **×** removes one, **reset** puts one input back, and "Reset to
defaults" all of them. Esc cancels waiting for a key, as does the pad's
Guide button, or waiting 10 seconds (a pad's other buttons all bind).
From the pad, A on a binding waits for the button to put there. Analog inputs (a
wheel, pedals, a ski angle) have three rows: the axis (click it, then move
a stick the way that should read as `+`, which binds it inverted if
that's left or up, or press a trigger; "invert" flips it, × leaves it
with none), and the keys or buttons that hold it at full `−` and `+`
(Left and Right for a wheel, as before). Light-gun games keep the mouse as
the gun; their trigger and reload keys are rows like any other, and so are
the buttons that fire and reload the gamepad's crosshair (pad buttons
only). Inputs the manifest leaves unbound (the second coin slot, test,
service, player 2 in `doa`) are behind "Show unbound inputs".

A key or button bound to two inputs is allowed (both get it) but marked
⚠ with the other input's name, except where the manifest itself binds it
so (`skisuprg`'s two ski angles share the arrows on purpose). The player's
own keys (Esc, F5, F8, F9, F11) can't be bound: pressing one while it waits
says so and waits for another.

Changes apply at once and are kept per game in
`<data dir>/<set>/controls.txt` (`out/<set>/` in the source tree, else
`~/.local/share/remodel/<set>/`), beside its EEPROM and saves: only what
differs from the manifest, one line per changed input (`jumpfront =
KeyX pad:South`, `curving.axis = -RightStickX`, `slide+ =` for none;
`romset::bindings` describes the format). The file is removed when
everything is back to the defaults; a line that doesn't parse (an unknown
input, a bad or reserved key) is ignored. The browser build remaps through
the same code and keeps the same text in its settings. Scripted `--input`
ignores bindings: it names the cabinet's inputs, so runs, oracles and
state hashes don't change.

## Bezels

With the 4:3 aspect in a wider window, the game's bezel fills the sides:
original cabinet-style artwork, one per game (`assets/bezels/README.md`;
a clone uses its parent's), as tall as the picture and centred on it. It's
on by default (Display, "Bezel", or `--bezel off`). A wide view draws the
game there instead, so it shows no bezel, nor does a window 4:3 or taller.
Every bezel is built into the binary (`crates/play/build.rs` embeds each
`assets/bezels/<set>.webp`, decoded with `image-webp`) and drawn with egui
under the menu (`crates/play/src/bezel.rs`). It's only drawn: emulation,
the HUD and the light guns don't see it, and the area beside the picture
still counts as off the screen.

`--shot FILE` (with `--seconds`) draws the last frame as a 1920x1080 window
would show it, bezel included, to a PNG: for checking without looking at
the window, e.g.
`remodel-play --game hotd --aspect 4:3 --no-sound --seconds 20 --input coin-start --shot out/hotd.png`.

## Files

Run from the reModel source tree (an `out/` directory in the current
directory), the player keeps its files in `out/`; anywhere else in
`$XDG_DATA_HOME/remodel` (normally `~/.local/share/remodel`):

- `play-settings.txt`: the settings, as `key = value` lines: the ROM
  folder, the games played (most recent first), fullscreen, where the
  settings panels save, and the game settings set for all games.
- `<set>/settings.txt`: the game's own settings, only where they differ
  from those for all games.

  A game setting whose value doesn't parse, in either file, is ignored
  with a warning on the terminal (the setting falls back to the next
  layer down, or its default).
- `<set>/controls.txt`: the game's remapped controls, only what differs
  from its manifest (above, "Remapping").
- `<set>/eeprom.bin`: the game's EEPROM (settings, records), written on quit or on switching games.
- `<set>/saves/quick.save`, `slot1.save` ...: save states, zlib-compressed,
  about 1.2 MB each. They only load into the same program ROMs, and carry
  a format number and the reModel version that wrote them (docs/usage.md,
  "Versions and releases"): a state from a newer format is refused with a
  message in the menu.
  `resume.save` is the one Continue loads.
- `<set>/preview-native.save` (`-chip` with the chip geometrizer): the
  launcher's attract mode preview state.
- `<set>/texpack/`: the game's texture pack, if you make one (PNGs named
  as a dump's).
- `<set>/jit/hot.txt` (on the JITs, the default; outside a source tree in the
  cache folder instead, `~/.cache/remodel/jit/<set>/hot.txt`): the JITs'
  hot-block list, the addresses of the blocks they translated (no game
  code), handed to the translation workers at the next boot so the game
  runs translated from its first seconds. Written on leaving the game and
  every 30 s; safe to delete. `--jit-warm` makes one for a game that
  has none from a quiet coin-start run in the background (no window or
  sound), for the game's next boot; `make jit-warm GAME=<set>` does the
  same from the command line (docs/jit.md, "Hot-block list" and
  "Pre-flight"). The JIT never makes the game wait: blocks are
  translated on background threads and interpreted until ready.
- Photo mode's photos aren't here: they go to `reModel/<set>/` in your
  pictures folder (`docs/photo-mode.md`; `--photo-dir` puts them
  elsewhere).

## A copy to hand out

`make dist` builds the player for the machine it runs on (here
`x86_64-unknown-linux-gnu`) as `out/dist/remodel-<version>-<target>.tar.gz`,
about 6 MB: the stripped release binary, a `README.txt` (bring your own
ROMs, keys, where files go), `LICENSE`, and `THIRD-PARTY-NOTICES.txt`, which
`tools/notices.py` writes from `cargo metadata`: every crate linked in, its
licence, and the licence texts it ships (bundled fonts' included). The version is the workspace's from Cargo plus
`git describe`. It runs the i960 on the interpreter only: recompiled code is
derived from the game's ROMs, so the target builds from the checkout's own
workspace (in its own `target/dist`), never the generated one in
`out/recomp/`, checks that it linked the empty `crates/recomp` stub, and
refuses `RECOMP=1`.

Unpacked anywhere outside the source tree, it keeps its files in
`$XDG_DATA_HOME/remodel` and looks for sets in `roms/` there; with none
found it opens the launcher, where another ROM folder can be typed in.
