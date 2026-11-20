# The native player

`make play` (or `make play RECOMP=1` with recompiled code from `make recomp`)
builds and runs `remodel-play`: the game in a window, rendered on the GPU
(`docs/gpu.md`), with both sound boards running live. `remodel-play --help`
lists its options.

With no game given (`make play`, or `remodel-play` alone) it opens the
launcher (below); `--game ID` (`make play GAME=ID`) starts that game
straight away; `--players N` (`make play GAME=srallyc PLAYERS=2`) starts it
as linked machines on one split screen ("Link play (same screen)", below); `--host` and `--join HOST:PORT` start it as one machine of a linked game whose others are on other machines ("Network link play", below).
At startup the reModel logo shows for 1.5 s, then fades out
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
chip it is off: the menu leaves its row out), and it replaces
multisampling: `aa` is kept but off while temporal is on (the menu's
anti-aliasing row is left out). A GPU
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
brightnesses instead and `pixel` is drawn as `vertex` (the menu
says so, and the window title says what was drawn: `shading
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
menu's checkbox under Shading) blends
the two of the game's colours around each lit pixel's brightness
instead, which removes the bands and never leaves the polygon's colour
(only lit pixels; the rest stay the game's; pure black, such as ink
lines, stays black). **Balance brightness** (`shading_balance`, on by
default; the menu's checkbox under Shading) keeps each smooth surface as bright overall as the game drew
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
decoded on background threads from the start and uploaded ahead of
their first draw (a stand-in shows for a texture not ready; docs/texture-packs.md,
"Flicker", with `--pack-log FILE` and `--pack-fixes none|all|surface,hold,preload`
to find and compare what makes replacements flicker). The launcher's previews never use packs. The Esc
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
strength (the menu offers 1, 2, 4 and 8; the settings file and
command line 0.1 to 8; such a strength shows in the menu as one more
choice, "custom (1.5)", kept until another is picked), how strongly the bumps' light change scales the
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
the menu's hint turns amber): it still works, and the 3 s rule below
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

The test has margins and patience both ways (`crates/play/src/sync.rs`),
because a game's frame cost wanders by several milliseconds with what is
on screen and a cost near the budget used to flip the player between
the display and the clock every few seconds (`vstriker`'s preview did it
at 16.0 ms against 16.7, and back at 13 ms, each flip a change of
rhythm and of in-betweens on and off): the clock paces only once the
smoothed cost has stayed over 103% of the frame time for 90 game
frames (1.5 s), and the display again only after 240 frames (4 s) under
80%; in between, whichever is on stays. A redraw that comes late runs at
most 2 frames to catch up (`CATCH_UP`), the rest of the lateness is
dropped, rather than sprinting several frames in one redraw.

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
`srallyc` differed that way. The check pins smooth motion and temporal
anti-aliasing off whatever the game's saved settings say: both make a
picture depend on when and how often frames were drawn, not only on the
frame (an in-between on the clock; a temporal image on the images before
it, a repeat adding a jittered sample), so the two ways legitimately
differ with them. A saved `temporal = taa` (it turns multisampling off
too) once made the check fail on `srallyc`: run-ahead pictures that
differed and frames drawn twice two ways, wherever the 3D layer showed.

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

### The renderer's time and memory

With `--perf-log` the draw thread says where a draw's host time goes and
what the renderer holds on the GPU:

```
draw phases   300 draws, ms each: draw 6.66 (shading 0.83, build 1.15, place 1.14, layers 0.41, 3d prep 1.69, 3d buffers 0.23, 3d pass 0.02, rest of the frame 0.28, submit 0.54); 2D layers reused in 132
gpu memory    357.1 MB allocated, 371.6 MB reserved; 3d aux a 63.8 MB; 3d aux b 63.8 MB; history a 63.8 MB; history b 63.8 MB; 3d depth t 31.9 MB; 3d layer t 31.9 MB; bloom level 23.6 MB (x5); ...
```

`draw phases` (every 300 draws; the example is Sega Rally at 4K with
`--temporal taa --bloom 100 --depth on --shading pixel`): the whole draw,
then `shading` (smooth shading's lighting of the frame; nothing for a
repeat), `build` (the display list into polygons), `place` (the HUD's
polygons moved, the polygons sorted, clipped and placed on the screen; with
the HUD at the edges also the tile planes of a new frame, see below),
`layers` (the 2D tile layers on the CPU, or found the same as the draw
before's, then only compared; the count says how often), `3d prep` (vertices and the polygon table), `3d
buffers` (their buffers and bind group, pipelines), `3d pass` (encoding the
render pass: draws are batched, so it is small), the rest of the frame
(temporal resolve, bloom, 2D uploads, the compose pass) and `submit`. They
are CPU time of the draw thread: the GPU's own time shows as `submit` or
the surface wait growing when it falls behind. `gpu memory` comes from the
driver allocator's own accounting (wgpu's `generate_allocator_report`, so
Vulkan and DX12; not the browser): bytes in live allocations by label, and
`reserved`, the memory blocks they sit in, which is what the process holds
(add some 50 MB of the driver's own: `nvidia-smi --query-compute-apps=pid,used_memory
--format=csv`). It is printed at the start and whenever either changes by a
MB, so after each change of settings.

A hidden window gets a redraw a second under vsync (on Wayland with the
NVIDIA driver the compositor does not hand its swapchain images back at
all, so the acquire times out; the player skips those draws and configures
the surface again at most every 15 seconds, where it used to do so at each
miss, which left a swapchain's worth of shared memory behind each time:
about 100 MB a second until the kernel killed the player). A bounded run
(`--frames`) is for checks and measurements, not for watching, so it
presents in mailbox mode and does not wait for the compositor. To time
drawing without a visible window otherwise, `REMODEL_PRESENT=immediate` (or
`mailbox`) sets that present mode where the surface offers it: the game then runs at its
pace and redraws as fast as it can (most of those are repeats of a game
frame; a repeat does everything but the emulation). The draw times
are then those of a draw with warm caches on an idle GPU: not what a
vsynced 4K window pays when the GPU is the slower side.

What it costs (Top Skater in a race, an 1800x1350 window, Ryzen 7 3700X and
RTX 2080 Super under `REMODEL_PRESENT=immediate`; ms of the draw thread per
draw, medians of a minute's `draw phases` lines, the machine shared with
other work, so a few tenths either way). About 87% of those draws repeat a
game frame, which is what a window drawing far faster than the game does,
and what smooth motion's in-betweens share with them: the same tiles and
palette, so the 2D layers are reused (the 2D layers changed on every new
game frame of the race: those draws cost what they did).

| settings | before | after |
|---|---|---|
| default | 2.9 (layers 1.4, build 0.7, 3d prep 0.3, rest 0.5) | 1.0 (layers 0.2, build 0.4, 3d prep 0.15) |
| `--temporal taa` | 3.5 (3d prep 0.9) | 1.5 (3d prep 0.5) |
| `--temporal 50` | 3.9 (3d prep 1.1) | 1.2 (3d prep 0.4) |
| `--shading pixel` | 4.5 (build 1.9) | about 2.9 (build 1.5) |
| Sega Rally, 4K (3840x2160), `--temporal taa --bloom 100 --depth on --shading pixel` | 7.7 (build and place 3.3, 3d prep 1.7, layers 1.55) | 6.7 (shading 0.8, build 1.15, place 1.15, 3d prep 1.7, layers 0.4) |

`layers` is `video::layers_2d`, a CPU rasteriser of the tile planes at the
game's 496x384 whatever the window: 1.4 ms a call (the planes 0.95, HUD
placement and shifts 0.3, widening 0.1, RGBA 0.15). `3d prep` is the vertex
and polygon table; temporal's is dearer for the previous positions and the
thin-polygon test. `3d pass` is nothing (the polygons are batched into a
few draws): the encoding is not where the time goes, and no draw waited
for the GPU at these sizes (`submit` 0.1 ms; `device.poll`, buffer maps and
read-backs aren't used while playing). Bind groups and buffers are
made each draw (`3d buffers`, 0.1 ms): not worth pooling.

What the process holds on the GPU (nvidia-smi; the allocator's reserved
memory in brackets), same window and race:

| settings | before | after |
|---|---|---|
| default | 249 MiB (192 MB) | 91 MiB (34 MB) |
| `--aa 8` | 505 MiB (448 MB) | 169 MiB (111 MB) |
| `--aa 4 --depth on` | 505 MiB (448 MB) | 169 MiB (111 MB) |
| `--temporal taa` | 507 MiB (448 MB) | 170 MiB (111 MB) |
| `--temporal 50` | 251 MiB (192 MB) | 138 MiB (79 MB) |
| Sega Rally, 4K, `--temporal taa --bloom 100 --depth on --shading pixel` | 555 MiB (448 MB) | 483 MiB (372 MB) |

(About 55 MB of each is the driver's own, which none of this reaches. The
rest scales with the window, as below: these windows are 2.4 million
pixels, a 4K one has 3.4 times that.)

Where memory goes, for a window of `W x H` pixels:

- the 3D layer: 4 bytes a pixel; with `--aa 4|8` also the multisampled
  image (16 or 32 bytes a pixel; cleared, resolved, never read back) and
  with `--depth on` the depth buffer (4 bytes a sample: 16 or 32 with
  multisampling);
- temporal anti-aliasing: the 3D layer and the aux target at the internal
  size (`--temporal 67` is 0.45 of the pixels) and two of each of aux (8
  bytes a pixel, internal size) and the history (8 bytes, output size):
  about 36 bytes a pixel at `taa` (4K: 300 MB); the multisampled targets
  are not allocated;
- bloom: its chain of half, quarter, ... sizes at 8 bytes a pixel: about
  2.7 bytes per pixel of the 3D layer;
- the game's render RAM, 7 MB, and the 2D layers (1 MB each);
- texture packs: images and material maps up to the budget
  (docs/texture-packs.md), tile atlas.

What was wasted, and is fixed:

- the allocator's blocks: wgpu's defaults hold 128 MB of device and 64 MB
  of host memory before the first texture and grow in blocks up to 256 MB;
  the player asks for blocks of 4 to 8 MB and a block of its own for a
  texture larger than that (192 MB reserved became 34 at the default
  settings, and the 448 MB of an 8x multisampled window 111);
- targets kept after the setting that wanted them was changed (the 3D layer
  and multisampled and depth targets after temporal anti-aliasing replaced
  them, temporal's after it was turned off, bloom's levels): a kept texture
  unused for 120 frames is dropped;
- the multisampled colour and depth targets are cleared at the start of
  their pass and discarded at its end: they are marked transient, which on
  Apple GPUs makes them memoryless (the 8x multisampled target of a 4K
  window is 265 MB there, and none now; elsewhere it changes nothing);
- the 2D layers (and their two uploads): the tiles and palette of the draw
  before's, the view and the 3D the same (a repeat, a smooth motion
  in-between, a HUD that didn't change) are not rasterised and uploaded
  again (`Renderer::layers_cached`). An in-between cost 1.4 ms of that on
  this machine, of the roughly 4 ms a draw costs on an M3 Max (not measured
  there);
- the tile planes were worked out twice a draw with the HUD at the edges in
  a game whose HUD has 3D parts (Sega Rally): once for the layout that moves
  the HUD's polygons (`HudLayout`), once for the 2D layers; they are shared
  now (`Planes`), and kept while the tiles and palette are the same. That
  was 0.95 ms of every draw of such a game at any window size.

Ideas not done:

- the history in a 4-byte format (R11G11B10 float) would halve temporal's
  history (4K: 66 MB) but rounds the colour, so the picture changes;
- aux and history ping-pong in place where the resolve reads each pixel
  once before writing it (it reads neighbours: needs a copy of a tile);
- `layers_2d` itself, 1.4 ms of the 2.9 of a draw of a new game frame
  (the planes 0.95), is a CPU rasteriser of the tile planes at the game's
  496x384; as a compute or fragment pass over the tile RAM on the GPU it
  would leave the draw thread (about half a core at 120 Hz with smooth
  motion, where every other draw is an in-between, which now reuse the
  layers), and cost the GPU a few microseconds; a change that can't alter
  the picture but needs the tile planes' rules (modes, splits, priorities)
  written in WGSL and checked against the CPU's (`tile_planes_check`);
- the vertex and polygon table of a repeated game frame kept (the stream,
  the RAM version and the view the same): `build` and `3d prep` (0.6 ms)
  would go for repeats; they are few at the display's rate with smooth
  motion off, and an in-between has geometry of its own;
- smooth motion's in-between is `geometrizer replay` plus `build` and
  `3d prep` of a whole frame: the polygons interpolated on the GPU from
  both frames' (a vertex shader, the vertices uploaded once a game frame)
  would take the in-between's CPU cost to about nothing, and is the
  largest saving there is for 120 Hz (`build` 0.7 + `3d prep` 0.3 + the
  replay);
- the hand-over copy (1 MB of tile RAM every frame, render RAM only when
  its version changed) is on the game's thread.

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
games". It lists the games found in the ROM folder (never the sets that
aren't there) in one of two views, switched by the Carousel | Grid switch
in the top bar or LB and RB (Q and E), and kept in `play-settings.txt`
(`view = grid`) for next time.

The **carousel**: at the right the selected game's poster large, the next
games' beside it (the strip wraps round), at the left its name, what kind of
game it is (`genre` in the game's manifest, `crates/romset/src/games/`),
its board and year, a line on whether it was left part way, and the buttons
Play, Saves and Settings. The game's live attract mode (below) plays dimmed
behind everything, its poster blurred until it does. The neon wireframe
floor (`crates/play/src/backdrop.wgsl`) lies under the posters.

The **grid**: every game as a poster with its name and genre; above them
filter chips (All; Recent, once something was played; and Racing, Fighting,
Shooting or Sports for each kind there is a game of: the manifest's `kind`),
a sort (Recently played, which puts the games played first, latest first,
then the rest A to Z; or A to Z), and at the foot "N of M games". The grid
doesn't run the preview. The carousel isn't filtered.

Both have the same top bar and hint bar. The top bar is the logo, the
**quick resume** pill (Y: the last game played, at its latest state:
the resume state, the quick save or a slot, whichever was written last, or
from the start if it has none; the pill says which and how long ago; a card
in the carousel, one line beside the logo in the grid) and the view switch.
The hint bar along the foot shows what each button does, the pad's chips
(Choose, A Play, X Saves, a menu button Settings, Y Continue, LB RB View,
and in the grid LT RT Filter) or, once the keyboard was used, its keys
(arrows, Enter, S, Tab, C, Q E, [ ]). Each entry can be clicked.

Play (A) starts the selected game from power-on; a game left part way is
picked up from **Saves** (X): a side panel of that game's states (where you
left off, the quick save, slots 1 to 4, each with its thumbnail when it has
one), and choosing one starts the game from it.

| | Keyboard | Gamepad | Mouse |
|---|---|---|---|
| choose a game, move over the controls | arrows | d-pad, left stick (repeat while held) | click a poster, wheel |
| play the selected game | Enter, Space | A | click it again, or Play |
| saves | S | X | Saves |
| settings | Tab | Start (the menu button) | Settings |
| quick resume | C | Y | the pill |
| carousel or grid | Q, E, Page Up, Page Down | LB, RB | the switch |
| previous, next filter (grid) | [ and ] | LT, RT | the chips |
| back (close a panel) | Esc, Backspace | B | the panel's back arrow |

Every control, the top bar's, the buttons, the chips, the sort and the hint
bar's entries, is reachable from the d-pad, which moves between them as
they're laid out; the focus ring (a cyan outline) shows while the pad or the
keyboard is in use, and the chips show the pad's buttons until the keyboard
or the mouse is used. Left and right on the carousel's selected poster step
through the games; down goes to the buttons. F11 toggles fullscreen, as in a
game. *Unconfirmed:* the gamepad navigation is tested with synthetic events
only, not yet with a pad.

Everything is drawn in the theme's look (`theme.rs`: Inter and Exo 2) on a
1280x720 design scaled to the window (and to the display's own scaling), so
it reads the same at 1080p, a 3440x1440 ultrawide and 4K; no text is under
14 points at 720p.

**Live previews.** Once a game has been selected in the carousel for half a
second, it starts in the background and its attract mode plays dimmed behind
the carousel, in place of the blurred poster, fading in, its sound fading in
9 dB under the game's mix (the texture is the window's shape at half its
size). The grid doesn't run it. It starts
from a save state at the manifest's `attract_frame` (the frame `remodel
shot` takes for the browser's game list), so it's live almost at once:
the chips and the state load on a thread of their own, then the machine is
built and the state loaded on the main thread (about 100 ms, the one
pause). The state is made the first time a game is previewed, by running
it headless from power-on to that frame with its sound boards and a fresh
EEPROM (never the player's), while the poster stays and says how far it has
got; it's kept as `<data dir>/<set>/preview-native.save` (or
`preview-chip.save` with the chip geometrizer), in the save state format,
and made again if it no longer loads. Previews run the i960 on the JIT as games do (with the game's hot-block
list, if it has one; interpreted, `vstriker`'s preview with smooth motion
took 21 to 44 ms a frame while the game's own is 3 to 8). Making it takes as long as running
the game that far as fast as it goes: recompiled, 2-4 s for `stcc`, `sgt24h`
and `topskatr`, 12 s for `hotd`, 19 s for `fvipers`; interpreted, 12 s for `topskatr` and 15 s for `doa`. Once cached, a preview shows about 0.6 s after its game is selected (the half second, 20-40 ms loading, 75-115 ms starting). One state
is made at a time, and it carries on if the selection moves to a game
whose state is ready; selecting another game that needs one stops it. Only
one preview runs: moving on fades it out over 0.2 s and drops it. A game
that can't start (chips missing from the ROM folder) keeps its poster and
says why. The preview never takes input, never saves its EEPROM, and is
emulated like the game would be (it costs what the game costs).

**Settings** (the menu button, Tab) open the Esc menu's side panel (below,
"The menu") over the launcher, for the selected game: Picture, Controls
(remap its keys and pad), Sound, Speed and latency, "Use the settings for all
games" for a game that has its own, and Quit reModel. It is the same panel,
code and settings as in a game (the picture styles, and the rows that apply
as set); changes are kept for all games or this game by its scope switch,
and what is saved from either place is identical. B, Start or Esc close it.
The preview doesn't show the picture settings (it draws flat, without
anti-aliasing and the rest); they show in the game.

Settings come in layers, lowest first, as in the
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
`--input` run. The carousel's line for the game says when it was left
(and Saves lists it first, "Where you left off"); the quick resume pill
(Y) picks the last game played up at its latest state: this one, the quick
save or a slot, whichever was written last. Play always starts from
power-on. The state is replaced the next time the game is left.

`--shot FILE --seconds N` draws the launcher too when it's what shows (and
the menu, when it's open over a game), and `--nav` scripts gamepad input
for that: `--nav next@1,settings@2,right@3` names a pad button and the
second it's pressed (left, right, up, down for the d-pad; accept, back,
continue, saves for A, B, Y, X; settings (the menu button, as start), prev,
next for the shoulder buttons, lt, rt for the triggers; start, select,
guide). It's released 0.08 s later, or after `:H` seconds
(`down@3.5:2` holds it, repeating); `start+select@3` presses two at once.
E.g.
`remodel-play --no-sound --seconds 6 --shot-size 3840x2160@2 --nav next@3 --shot out/launcher.png`
(the grid),
or the menu over a game, left from the pad:
`remodel-play --game topskatr --no-splash --no-sound --seconds 10 --nav start+select@3,down@3.5:3,left@7,accept@7.5 --shot out/back.png`.
`--frames N` ends the run when the game reaches frame N (it stops there),
so `--shot` pictures that frame: with `--input coin-start` the same frame
the CLI's `--capture` saves, for `gpu-check` to draw beside it.
It's drawn with egui (`crates/play/src/launcher.rs`; what it lists, the
filters, the sort and the quick resume are in `library.rs`), its layout,
navigation, filters and hints tested headless (`cargo test -p remodel-play`).

**In the app menu.** `make install-desktop` (`RECOMP=1` for the recompiled
build) builds the player and adds reModel to the desktop's app menu: a
`remodel.desktop` entry in `~/.local/share/applications` and the icon
(`assets/reModel-icon.png`, `assets/README.md`) in the hicolor theme.
The entry runs this checkout's player from the checkout's directory, so it
finds `rom/` and keeps its files in `out/` as `make play` does; rebuild
with `make play` (or rerun `install-desktop`) and the menu runs the new
build. The window's app id (Wayland) and class (X11) are `remodel`, which
the entry names, so the window gets the icon. `make uninstall-desktop`
removes both.

## Link play (same screen)

Two to four linked machines of one game in one window, split-screen, one
controller each, like a kart racer's split screen: each is a whole machine
with its own sound board, joined by the link board model
(`model2::link::LinkHub`, `docs/hw/link-board.md` section 9) the way
the cabinets' own network joined them. Only games with an entry in `crates/romset/src/games/link.rs`
(`romset::LinkPlay`: how many cabinets, where the game keeps the setting that
makes a cabinet CAR 1 to CAR N and what its checksum is, the link RAM layout)
can, and the player splits the screen for the ones it lists
(`link::SPLIT_GAMES`); today that is Sega Rally (`srallyc`) and Daytona USA (`daytona`), up to 4 on one screen (`remodel link-run` runs Daytona with up to 8). Daytona keeps its settings on the I/O board, so its linked machines start from the manifest's default record rather than the player's own settings.

**Starting it.** `make play GAME=srallyc PLAYERS=2` (2 to 4), or
`remodel-play --game srallyc --players 2`. In the launcher a game that can
has a **Link play** button (key L) beside Play, Saves and Settings: a panel
asks for 2, 3 or 4 players. Each machine boots, does the board check,
passes the link barriers and shows the link attract ("3人まで対戦可能!!")
by itself; coin up and start per player as at a row of cabinets, and the
cars race each other (`POSITION 1/2`, the others' cars on the road).

**Machines and EEPROMs.** Each machine has its own EEPROM with LINK TYPE
set in memory (CAR 1 for player 1, which is the master, up to CAR N); the
settings the master sends to the others during the first handshake go into
their own images. They are kept in the linked set's own files, `out/srallyc-link2/eeprom-0.bin`
(`eeprom-1.bin`, ...; in the data folder anywhere else), so the single game's
`out/srallyc/eeprom.bin` is only read, to start the set from the player's
settings the first time, and is never written. The linked set's saves and
resume state are likewise its own (`out/srallyc-link2/saves/`).

**The screen.** Every machine draws into its own view of the window with
its own renderer on the one GPU device (`gpu::Renderer::from_device`), each
view 4:3 and letterboxed in its cell, so the picture styles (anti-aliasing,
depth, shading, bloom, the CRT and ink looks) apply to each view as they do
to a single game. Two players are side by side or stacked, as the
`link_layout` setting (the menu's Display tab, `--link-layout auto|side|stacked`)
says: Auto picks whichever gives each picture more room, which is side by side
in a 16:9 window (two pictures of about 957x717 in 1920x1080, against 717x537 stacked)
and stacked in a tall one. Three and four players are quadrants (957x537 cells
in 1080p, a 716x537 picture); the empty fourth quadrant of three shows a dim
panel with the game's name. A small tag by each view says who plays it and
on what ("P2 Controller name", yellow and "Press a button on a controller to
join" while nobody does), above the picture where the letterbox has room.
The render resolution is the view's own, so it follows the window.

What is off in link play: smooth motion and run-ahead (each would need
every machine to run its in-betweens or frames ahead, and the exchange
makes a machine's frame depend on the others), temporal anti-aliasing
(it needs smooth motion's recording; the setting is ignored, multisampling
as set applies), texture packs, the bezel, Compare and photo mode (its menu
row is left out; F8 says so).

**Controls.** Player 1 is the keyboard. Gamepads take the other seats in
the order their first button press comes (that press is the join, not the
game's): the first pad plays P2, and so on up to the player count; a pad
that unplugs frees its seat. Each pad has its own gate (the Start and
Select menu chord, buttons held in the menu), the bindings are the game's
(the Controls page edits them for everyone: every machine is "player 1" of
its own cabinet, so coin, start, pedals and wheel are all per player), and
the menu follows the pad used last. `--link-keyboard off` has the
keyboard play nothing, so pads take the seats from P1 (four pads for four
players, where P1 would otherwise be the keyboard). A seat nobody holds has
neutral controls. Scripted runs: `--input` is every player's script unless
`--input2` to `--input4` say another for that player.

**Sound.** Each machine's sound board runs on its own thread and its own audio
stream, mixed by the system. The `link_audio` setting (the menu's Sound tab,
`--link-audio`): **Player 1 loudest** (the default: P1 full, the others
at 30%, so a pass is heard and engines are not a wall of sound), **All equal**,
**Spread** (left to right in player order) and **Player 1 only** (the
others' sound boards don't run, the cheapest). Changing mode changes
levels at once and stops boards the new mode leaves out; one it adds is
there from the next start.

**Menu and states.** The Esc menu pauses every machine. A save, quick or in a
slot, holds the whole set: every machine's state and sound board, the
hub's and the board flags (`started`, `master`, which the machines' own state doesn't
carry), as a normal save file whose tail is the other machines' (so
`remodel link-run --save-at` and the player agree about what the set is);
loading it resumes the race exactly: the same inputs from there give the same
combined hash as an unbroken run. Reset game and Back to games keep the
player count.

**Reproducible.** `--frames N` prints each machine's state hash and the
combined one (an FNV of the machines' hashes and the hub's state), which is
what `remodel link-run` prints too; the same scripted inputs give the same
value in both (4,400 frames, two machines: `bd3b0b5768e8d6ad`; it was `a7d468fcdccbfe99` before the hub's saved state changed for network play, `docs/hw/link-board.md` section 12: the machines' own hashes are unchanged).

### Free play

`srallyc` and `daytona` have a FREE PLAY choice among the coin settings in
their own test menu (COIN ASSIGNMENTS, setting #26; COIN ASSIGNMENT,
setting #27: `docs/games/srallyc.md`, `docs/games/daytona.md`). The player
uses it, so a start press begins a game with no coins (srallyc asks for two per
credit otherwise, and a linked race needs one credit a machine). The attract
screens say FREE PLAY, as on the cabinet.

The `free_play` setting (the menu's More tab and the launcher's game
settings, shown only for games that have the choice), `--free-play` and
`--no-free-play`: **Link play only** (the default: on in split-screen and
network link play, off in a single game), **Always** or **Never**. It
applies when the game starts. The game's settings are patched in memory
(for srallyc, with its EEPROM checksum fixed): the player's own settings
file is never given free play, and a link set's own files that an
earlier session left on free play are put back when it is off. In a network game
the host's setting goes to every machine with the rest of the master's
settings (the settings hash the session agrees on covers it), whatever the
others have chosen. From the command line:
`make link-run GAME=srallyc LINK_OPTS=--free-play INPUT=start@1560-1564`
and `make link-net GAME=daytona NET="--host 47890 --nodes 2 --free-play"`
(`remodel link-run --free-play`, `link-net --free-play` on the host); `make
link-free-play-test` runs a two-node race of each game from start presses
only, in one process and over loopback.

### What link play costs

Measured on the development machine (16 threads, RTX 2080 Super; the machine
was shared with other work, load average 3 to 10, so read the milliseconds as
within about 1 ms), Sega Rally, JIT, coin-start script into a race, status
lines of the last 15 seconds of a 4,300 frame run (frames 3,100 on are the
race), presentation unpaced (`--frames` runs use mailbox, so the player
draws as fast as it can: the redraws a second below are the most it could
draw, against the 60 a real display wants), sound off:

| | Emulate, main thread (ms/frame) | Hand-over (ms) | Draw thread, 1280x720 window (ms/draw) | Most draws a second | GPU memory held: allocator, process |
|---|---|---|---|---|---|
| single game | 2.2 | 0.37 | 2.8 | 268 | 34 MB, 91 MiB |
| 2 players | 1.5 | 0.70 | 2.6 | 278 | 48 MB, 105 MiB |
| 3 players | 1.8 | 1.15 | 3.8 | 178 | 48 MB, 109 MiB |
| 4 players | 1.9 | 1.39 | 4.1 | 158 | 64 MB, 125 MiB |
| 4 players, 3840x2160 fullscreen | 1.9 | 1.42 | 4.1 | 159 | 80 MB, 189 MiB |
| 2 players, 3840x2160 fullscreen | 1.4 | 0.67 | 2.4 | 297 | 55 MB, 167 MiB |

Every count holds 60 Hz, with room: four machines take 1.9 ms of the
16.7 ms refresh to run (the machines run alongside each other, a thread
each, so it is the slowest one plus the hub) and the draw thread 4 ms, about
2.7 times as many draws as the display asks for; the machines' own cost does
not grow with the player count the way a single thread running them in turn
would (one machine is about 1.5 ms of a frame; four in turn would be 6).
`--fast` (no drawing; 2,700 frames in the race, two runs each) gave the
emulation alone, every thread together, at 0.2 to 0.6 CPU cores at 60 Hz
for 2 to 4 players (a single game 0.3 to 0.4; these differ run to run more
than the players do, the JIT's translation of the race's code and the
other work on the machine); the full emulation speed is 900 frames a second
for two machines and 500 for four, 8 times real time. The sound boards cost
about 0.08 of a core each (a single game 0.4 against 0.3 cores without
sound; four players 0.9 with Player 1 loudest and 1.0 with All equal against
0.6 without; Player 1 only 0.7: one board), so **Player 1 loudest** costs the
four boards (0.3 core) and Player 1 only costs one. At 4 players
the whole player is about 1.3 cores of the 16 (emulation 0.6, the four
sound boards 0.3, drawing 0.3, the rest). Audio ran with four streams
without an underrun.

**GPU.** All the views share one device and queue; each machine has its
renderer (`gpu::Renderer::from_device`), its frame copy and its own
textures (the RAM of its game, its 3D layer, the 2D layers), so the memory
is per machine (`render ram` 7 MB and the 3D layer at the view's size, 10.7
MB at 480x360 views in a 1280x720 window and 25 MB at 1440x1080 views in
3840x2160): four machines hold about 60 to 80 MB where a single game holds
35 to 55. The draw thread draws the views on a thread each (one thread for
the view whose renderer it holds, scoped threads for the others), which
brought four views from 6 to 12 ms (drawn one after another) to about 4 (the
CPU work of each view, polygons, layers and encoding, is the cost, not the GPU). The render
resolution is the view's size, so a smaller cell draws fewer pixels.

What the picture styles cost with four views (1280x720 window, the same
race, draw ms a draw / GPU memory held by the allocator): original 4.1 / 64
MB; anti-aliasing 4x 4.3 / 111 MB; 8x 4.2 / 157 MB; bloom 50 4.4 / 68 MB;
the CRT look 3.7 to 5 / 80 MB; 4x, bloom, CRT and depth together 5.2 / 166
MB (158 MB, 293 MiB in the process, at 3840x2160 with 4x anti-aliasing).
The costs are the same per view as a single game's (`docs/gpu.md`)
and the memory is the same times the players, so the multisampling
targets are what to watch on a small GPU: 8x with four views holds
about 90 MB more than none. None of them came near a refresh.

**Degrading.** Four machines hold 60 Hz here with room, so nothing degrades
by itself: the render size follows the window (a smaller window or fewer
players draws fewer pixels), and the options for a slower machine are the
sound mode (Player 1 only), anti-aliasing off, and a smaller window. A
machine that cannot keep up is paced by the clock as a single game is
(`sync`); there is no automatic reduction of the internal resolution, which
none of the measured runs needed. *Unmeasured:* a slower GPU or CPU; four
machines at 120 Hz; the pad's menu in link play beyond the checks above.


## Network link play (a LAN)

The same linked cabinets, each on its **own machine**: every player runs
reModel, one **hosts**, the others **join**, and each window shows its own
cabinet. The link board's blocks travel between the machines over UDP, in
lockstep (`remodel-linknet`, `docs/hw/link-board.md` section 12). Sega Rally
(`srallyc`, 2 to 4 machines) and Daytona USA (`daytona`, 2 to 8) can.

**Hosting.** `make play GAME=srallyc PLAY_ARGS="--host"` (or `remodel-play
--game srallyc --host [PORT]`; the port defaults to 47890), or in the launcher
**Link play** (key L), then **Host a network game**. The panel shows this
machine's LAN address and port ("192.168.1.20:47890"), lists the players as
they join with their round trip, and has **Start the game** (at least two
players). `--net-players N` starts by itself once N players are in. The host
is player 1: CAR 1, the master, and the one whose settings everyone runs
(srallyc's EEPROM settings; daytona starts from its default record).

**Joining.** `remodel-play --game srallyc --join 192.168.1.20:47890`, or the
launcher's **Link play**, **Join a network game**: type the host's address
(the port may be left out) and press Enter, or pick a host the panel found on
the network. The list merges two ways of finding hosts (see **Finding hosts
and names** below) and shows each host's name, game and players ("Alex   Sega Rally
1/4"); a host you cannot join is greyed out with the reason (another reModel
version, other ROMs, a game you don't have, full, already starting). Each joiner
is the next player. The window then shows the
game's boot, board check and link attract by itself, like the same-screen
set; coin up and start as at a row of cabinets.

**Finding hosts and names.** A host in its lobby advertises itself two ways, and the
join list merges them (one entry per host, found either way):

* **mDNS / DNS-SD**: the service `_remodel._udp.local.` on the host's real
  port (any port, not only 47890), with a TXT record of the game set, the
  reModel version, a hash of the program ROMs (what the session agreement
  compares), players and seats free, the delay D it would start with, the lobby
  state and the host's name. The record is updated as players join and leave;
  the advert is withdrawn when the lobby is full or the start is under way, put
  back if a seat frees up, and withdrawn with a goodbye when the host exits or
  closes the lobby. It uses the shared multicast port 5353 as other mDNS
  responders do, so Avahi or Bonjour on the same machine is fine.
* **Broadcast**: a datagram to 255.255.255.255:47890 that hosts on the default
  port answer, kept as the fallback. If mDNS cannot start (no multicast on the
  machine) reModel says so once on the console and carries on with the
  broadcast alone. Typing an address always works.

Each player has a **user name**, kept with the player's settings
(`player_name` in the settings file), edited on the launcher's **Link play**
page, `--name NAME` for one run. It defaults to the OS login name, else
"Player"; it is trimmed to printable characters, at most 16. The host's lobby
lists the players by name, the join list shows the host's, and the status line
and the end messages use them ("Sam left", "Sam has been silent for 5.0 s"). Two
players with one name are shown "Sam" and "Sam (2)". Names are only shown: they
are not part of any machine's state or hash, so the same inputs still give the
same state hashes.

**What must match.** The same game, the same ROM files (a hash of the program
ROMs), the same reModel version (the crate version) and protocol. A joiner
that does not match is refused with the reason. The host picks D, the link
delay, from the measured round trip (about one frame per 17 ms of one-way
delay, plus one): on a wired LAN 1 or 2, over Wi-Fi 3 to 5. `remodel link-net`
(below) has `--delay`.

**Playing.** Controls are a single player's: the keyboard and the first
gamepad, the game's own bindings. A small status line at the top of the window
gives the players, D, the round trip and how often the picture was held to wait
for another machine ("held 12x (0.4 s)"; "WAITING for the other player" when
it has been over a quarter second). Lockstep never guesses, so a late packet
holds *every* machine's frame (the picture freezes, the sound drops) and a slow
machine slows everyone; the cure is a larger D or a better network.

**What is off.** Smooth motion and run-ahead (as in same-screen link play),
texture packs' temporal parts, the bezel and photo mode as there, and
**save states, quick save/load, Continue and Reset game**: the machines
cannot be saved or reset alone (the menu says so). The session's settings
are the master's and nothing of your own (EEPROM, resume state) is written.
**Esc** opens the menu over the game, but the game does not pause for the
others: lockstep holds every machine's frame while one is in its menu (the
link stays alive; the others see "WAITING"), so keep it short.

**When it ends.** If a player leaves, quits the game or their machine goes
silent for 5 seconds, every other machine drops back to the launcher with the
reason ("Sam left", "Sam has been silent for 5.0 s: connection
lost"). If the machines' state checks disagree (every 60 frames each host
hashes the records it feeds its link; a damaged or misdelivered record
shows) the session stops with the frame and both hashes. There is no mid-race
recovery or rejoin: what a game would do with a missing cabinet is not
known, so the session ends instead.

**Firewall and ports.** The host listens on UDP, default port 47890
(`--host PORT`), on all interfaces; its firewall must let that port in, and
for the mDNS list UDP 5353 as well (multicast 224.0.0.251; most desktop
firewalls allow it on a trusted network). Some Wi-Fi access points and
guest networks filter multicast or isolate clients: the host then does not
show up in the list (and the broadcast may be filtered too); type its address. The
joiner needs nothing opened. With three or more machines each machine's port
must be reachable by the others (the joiners' ports are random). Nothing is
opened or changed by reModel; there is no internet traversal: for play across
the internet use a VPN that makes the machines look like one LAN, or see
`docs/hw/link-board.md` section 13 for what it would take.

**Headless: `remodel link-net`.** The same session without a window, for
tests and tuning, scripted inputs and all:

```
remodel --game srallyc --input "coin1@1500-1504,coin1@1560-1564,gas=0xff@3430-99999" \
    link-net --host 47890 --nodes 2 --delay 3 --frames 4400          # on one machine
remodel --game srallyc --input "..." link-net --join 192.168.1.20:47890 --frames 4400
```

`--name NAME` sets the player name, `--no-mdns` turns mDNS off. `remodel link-net --browse [SECS]`
(`make link-browse`) lists the hosts found on the network over mDNS and broadcast
for SECS seconds (default 3) and exits: one line each with the name, game,
players, address, version, ROM hash, D, how it was found, and whether this
machine could join it.

Each prints its node's state hash (the same line `remodel link-run` prints for
that node with the same inputs and delay: they must be equal), the stalls and
the bandwidth. Fault injection for tests: `--latency MS --jitter MS --loss PCT
--dup PCT --seed N` (on what that host sends), `--corrupt-rx-at F` (damage a
received record: the state check must report it), `--tamper-byte OFF:V` (a
settings byte changed after agreeing: the host refuses to start), `--leave-at F`; `--realtime` holds the machine to 60 frames a second, so the stall counts are what a
player would see (otherwise the hosts run as fast as they can and wait for each other).
`make link-net-test` runs the whole set over loopback for both games
(clean, 5% loss with 50 ms jitter, a polite leave, a kill, a settings
mismatch, a damaged record) and checks every hash against `link-run`; it needs
the ROMs.

## Keys

The player's own keys are the same for every game:

| Key | |
|---|---|
| Esc | menu (the game pauses while it's open) |
| F5, F9 | quick save, quick load |
| F8 | photo mode (docs/photo-mode.md): a dock along the bottom edge (tabs Camera, Time, Focus, Look, Effects, Frame; LB and RB switch them, Y hides it, the red shutter takes the photo, B leaves), never a panel over the picture |
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

### Sticks as levers

A game's digital stick or lever (the fighters, the shooters, `von`'s two
levers) can be played with an analog stick. Each stick reads as an 8-way
lever, as four more pad "buttons" (`romset::PadButton`: `LeftStickUp`,
`LeftStickDown`, `LeftStickLeft`, `LeftStickRight` and the right stick's;
`romset::StickDirs` does the reading, for the native player and the browser
alike). It takes a real push, half of full travel or the dead zone if
that's more, and holds the direction its angle is in: within 22.5 degrees
of an axis it's that direction alone, between them both (a diagonal), as a
lever's two switches. A held direction lets go a tenth of travel lower and
keeps to a wider angle, so a stick resting on an edge doesn't chatter.

Every game whose pad bindings use the d-pad for a button gets the left
stick's same direction as well, unless the game itself reads that stick's
axis (a wheel on the left stick's X, an analog bound to its Y) or has a gun
the sticks aim: the d-pad and the left stick are one lever, with no
per-game edit. The driving games, the light-gun games and the like keep
their stick as an axis. A manifest can name the directions itself
(`von` binds the right stick to its right lever), and they can be
remapped like any button: on the Controls page, move a stick to bind it
(the chips read "L stick ↑"). Older bindings files still load; a button
the player has remapped keeps exactly their list.

### Virtual-On (`von`)

| Control | Keys | Gamepad |
|---|---|---|
| coin | C, 5 | Back |
| start | Enter, 1 | Start |
| left lever | W, A, S, D | left stick, d-pad |
| left shot, left dash | Q, E | LB, LT (also X, A) |
| right lever | arrow keys | right stick |
| right shot, right dash | `.`, `/` | RB, RT (also B, Y) |

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

Esc opens the pause menu (and closes it again). The game stays on screen,
paused, with a small "Paused" pill at its top left, and the menu is a panel
docked on the right edge (380 points at 720p, scaled with the window like
the rest of the look), full height, over a soft shadow. Its header has the
game's name and the page's heading; its foot the button hints (A Select, B
Back, Esc Resume).

The first page lists what to do: **Resume**, **Quick save** (F5), **Quick
load** (F9), **Save slots** (how many are used), **Photo mode** (F8:
`docs/photo-mode.md`), **Reset game** (select it twice within four seconds:
the game starts again from power-on, keeping its EEPROM) and **Back to
games**. Under a small "SETTINGS" label come the settings, one level in,
each showing its value: **Picture** (the style), **Controls**, **Sound**
(the music level) and **Speed and latency** (run-ahead, or exact speed or
smooth). With no game running (the menu from the launcher's Settings, for the
game selected there) there is no game to resume or save, **Controls** is that
game's, a game with settings of its own has **Use the settings for all games**,
and there is a **Quit reModel** row at the foot. The version is at the foot of
the list.

From a gamepad: the d-pad or left stick moves between the buttons and rows
as they're laid out (repeating while held, as in the launcher), A presses
the one that glows cyan, left and right change a settings row, LB and RB
switch settings tabs, B or Select goes back one screen (from the pause menu
it resumes) and Start resumes from anywhere. The menu opens on Resume. The
keyboard does the same: the arrow keys, Enter or Space for A, Q and E
(or Tab and Shift+Tab) for the tabs, Backspace for B; Esc closes the whole
menu. The focus ring shows while the pad or those keys are in use, and the foot
shows the pad's buttons while the pad is. `crates/play/src/focus.rs` does
it for any egui page; the menu's tests (`menu.rs`) drive it with `Nav`s
headless. *Unconfirmed:* tested with synthetic and scripted (`--nav`)
events only, not yet with a real pad.

- **Save slots**: a page of its own with a card for the quick slot (F5/F9)
  and four more: a thumbnail of the game as it was (or "Empty"), the name,
  when it was saved ("Yesterday 21:08 · 14 h ago", in the computer's time
  zone where the platform tells it) and which reModel saved it and whether
  this version loads it. A click, or A, selects a card (the pad's focus
  then goes on to Load, or to Save here for an empty slot); **Load**,
  **Save here** and **Back** are at the foot. Quick save and Quick load on
  the first page, F5 and F9 do the same as the quick card. Saves and loads
  say so in a toast at the screen's bottom left. A save holds the whole
  machine, both sound boards included, so loading one resumes the sound
  exactly where it was. The one thing not saved is a replacement soundtrack
  voice that's playing (`docs/mods.md`). The thumbnails are described under
  "Files" below.
- **Settings**, in tabs under the page's heading (the Motion tab's is
  "Speed and latency"; Picture, Sound and Speed and latency are the pause
  page's own rows, Display and More are one tab away). Each row reads
  label over value, "◂ ▸" at its right: left and right
  change it (A or a click on the name steps on, round to the first), and a
  help line at the foot says what the row in focus, or under the
  pointer, does. A row that does nothing as things are set is left out, not
  greyed (anti-aliasing while temporal is on, temporal with the chip
  geometrizer or on a GPU that can't, the shading options with flat
  shading). Every tab ends with **Applies to: this game / all games**,
  where the changes are kept (the same from the launcher's Settings).
  - **Display**: the aspect (fit the window, the default, drawing the game
    wider than its frame from 4:3 to 32:9; or 4:3, `--aspect 4:3`), where
    the HUD goes on wide windows (shown for fit the window), the bezel
    (below; shown for 4:3, where there is art), and fullscreen (as F11).
  - **Picture** is a page of its own, not a tab (below, "Picture styles").
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

  **Picture styles.** The Picture page (the menu's Picture row) lists
  six cards, each a name, a small tag and one plain sentence: *Arcade
  original*, *Sharp*, *Smooth modern* (recommended), *CRT*, *Comic* and
  *Custom*.
  Choosing one applies it at once to the paused game beside the panel. A
  switch at the top, "<Game> only | All games", is the settings' scope (the
  same layers as the launcher's), and the last line says where it
  is saved and what other games keep ("Saved for Top Skater. Other games
  keep Sharp."). *Compare* shows the paused picture twice, as the arcade
  original left of a vertical divider and as chosen right of it, with
  "Before" and "After" labels at the picture's top corners: drag the
  divider with the mouse, or move it with the pad's left and right (also
  from any card while Compare is on). The game is drawn twice (`draw::Compare`,
  `gpu::Renderer::with_quality` and `wipe`): the original into a kept
  picture, the chosen style into the window, and the original's part left
  of the divider copied over; the divider is limited to the part of the
  picture the panel doesn't cover. Cost doubles while paused only; the
  texture pack, if on, shows on both sides. *Custom* is whatever Advanced
  made; choosing another style and then Custom again restores the last
  custom values (kept for the session, not in the files). *Advanced ›*
  opens a row for each picture setting, each "◀ value ▶" with a yellow dot
  where it differs from the style (or the style the Custom came from:
  "Custom (from Smooth modern)"), and only the focused row says what it does:
  Resolution (the temporal render scale; shown with temporal smoothing on),
  Edge smoothing (multisampling; off while temporal smoothing is on),
  Temporal smoothing, Lighting (flat, smooth, per pixel), Glow (bloom),
  In-between frames (smooth motion; as before it does not make the style
  Custom), Wide screens (HUD placement, only a dot-free setting), Texture
  pack and CRT; under *More*: the depth buffer, see-through polygons,
  crease angle, brightness levels and balance, material maps and relief
  and texture filtering. Any picture row changed makes the style Custom;
  *Back to the style's values* returns to it. Some changes finish when the
  game runs again (smooth shading's exact lighting and temporal
  smoothing's motion data are recorded from frames the paused game does not
  run).

  **The Comic look** (`look` setting, off by default; the Comic style is
  Sharp with smooth lighting and the look on) draws ink outlines, the
  game's lighting in three flat bands and halftone dots in the shadows
  (docs/enhancements.md, "Looks"), with no pack on any game. With the pack
  on and a `look.txt` in it (or in the game's data folder) that file is the
  look: it is read again within half a second of an edit, so a person or an
  agent editing it sees the picture change in the running game. Advanced
  shows *Comic look* and, with it on, a row for each parameter; a change
  applies at once and is kept as `look_tuning` over the file's value
  (*Look tuning: Reset* goes back to the file). Needs the GPU path; the
  look is not in photo mode or the Compare "before" picture. For agents:
  `make look-render` draws captured frames with a look and parameters
  (docs/texture-packs.md, "Looks").

  **The CRT look** (`crt` setting, 0 to 100, off by default and in every
  other style; the CRT style is Sharp with 60) is a pass over the finished
  picture (`crates/gpu/src/crt.wgsl`), not the bezel or menu, and changes
  nothing the machine does. It draws one scanline for each of the game's
  384 lines at any output size (a line's brightness a gaussian across it,
  wider where brighter), a stripe mask staggered on alternate lines, a
  little glow from bright parts, a soft vignette and a very slight barrel
  curve, all scaling with the strength; the lines and mask fade out under
  about 2 output pixels a game line (a window under 770 pixels tall), where they
  would only beat against the pixel grid. `--crt off|N` sets it for a run.

  **Picture presets.** *Original* is the arcade look and every default:
  nothing that changes the picture (and no smooth motion or run-ahead).
  *Sharp* is 4x multisampling and mipmapped texture filtering, nothing
  else. *Enhanced* (shown as Smooth modern) is per game (`romset::Recommended` in the manifest, the
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
  (`~/.local/share/remodel/roms` on Linux), created on first run; `--rom` changes
  it (and is remembered).

### The look

`crates/play/src/theme.rs` is the player's shared look, laid out at 1280x720
logical points and scaled by the window's height (`theme::scale`: 1.0 at
720, a floor of 0.85, so a 4K screen and a small laptop window share one
layout). It holds the palette (background `#04060b`, cyan `#22e5ff`, blue,
red, yellow, green), the type (Inter for text, body 16 and never under 14
points at 720p; Exo 2 Black Italic in capitals for headings), egui's style,
and the pieces screens are built from: the panel, the menu row, the
segmented control, the button-hint chips (coloured face-button circles, a
"≡" chip for Start, grey chips for keys), the focus ring (a 4 point cyan
outline with a faint fill, and a left bar on menu rows) and the toasts.

Toasts are the corner notices (bottom left): a short queue, each a card with
a status dot (green done, yellow warning, red error) that fades after 2.5 s.
Errors (`Menu::error`) stay until a click on them or the menu opening;
`Menu::notify` and `Menu::warn` are the other two levels.

The fonts are in `crates/play/assets/fonts/` with their licences: Inter 4.1
Regular, SemiBold and Bold (rsms/inter) and Exo 2 Black Italic
(NDISCOVER/Exo-2.0), static TrueType because egui doesn't instance variable
fonts. Both are SIL OFL 1.1, and `make dist` copies the licences into
`THIRD-PARTY-NOTICES.txt`.

### Catalogue

`--catalogue` (or the **Catalogue** setting: Esc menu, More page; off by
default, applies when a game starts; `--catalogue off` overrides a saved
`on`) observes the game into its catalogue as you play: every distinct 3D
texture, 2D tile pattern, tile layout (logos, labels, number fields) and
object the game shows, each with how often, where and with what, and every
texture RAM upload (`out/GAME/catalogue`, or `--catalogue-dir DIR`; the
store, the report and the browser are in docs/uplift-studio.md,
"Catalogue"). Play every course, every character and the menus normally and
the catalogue fills; each run of the player is one session, and sessions
add up. `--catalogue-every N` sets the frames between samples (default 60,
about once a second; a sample is also taken, at most every 8 frames, when
the 2D layer is rewritten in bulk or textures are loaded).

What it costs: the game thread, once a frame, drains the texture watch (a
counter compare, 0.3 ms only on frames where the game wrote texture RAM), and
once a sample copies the frame's renderer inputs (about 4 MB) to a worker
thread, which identifies, draws and writes: the game thread never waits for
it (a sample the worker has no time for is dropped and counted). It turns the
geometrizer's lighting side channel on (for the models' addresses), which is
what `--shading` and `--materials` do too. `--frames N` with `--catalogue`
prints the cost at the end (`catalogue ... game thread X ms a frame`). The
emulation, its state hash and what is drawn are unchanged. A save state
loaded in the middle is handled (the watch starts from the loaded textures
and says so; the next sample is taken at once).

## Remapping

The Esc menu's **Controls** page (from the launcher's Settings too, for
the selected game) is a side panel with the game's actions, and over the
paused game beside it a picture of the device and, for games that read a
stick or pedals, the tuning. It is the Picture page's layout.

**The actions.** Each row is one input of the game with its bindings as
chips, for the device chosen at the top: **Pad**, **Keyboard**, or for a
driving game **Wheel** and for a light-gun game **Gun** (the game's own
controls: both devices' bindings). LB and RB (or Q and E, or a click)
switch device. A (or a click on the row) waits for a key, a pad button or,
for an axis, a stick or trigger, and puts it in place of the row's first
binding on that device; a click on one chip replaces that one; **X** (or the
**+** on a hovered row) adds another (an input can have several); **Y** (or
**×**) clears the device's bindings from the row, and on an empty row that
differs from the game's own, brings them back (as a hovered row's **Reset**
does). "Reset to the game's defaults" (it asks twice) puts every binding
and the tuning back. Esc cancels waiting for a key, as does the pad's Guide
button, or waiting 10 seconds (a pad's other buttons all bind). A dot
marks a row changed from the game's own. Analog inputs (a wheel, pedals, a
ski angle) have three rows: the axis (move a stick the way that should read
as `+`, which binds it inverted if that's left or up, or press a trigger;
X or **Flip** inverts it, Y leaves it with none), and the keys or buttons
that hold it at full `−` and `+` (Left and Right for a wheel, as before).
Light-gun games keep the mouse as the gun; their trigger and reload keys
are rows like any other, and so are the buttons that fire and reload the
gamepad's crosshair (pad buttons only). Inputs the manifest leaves unbound
(the second coin slot, test, service, player 2 in `doa`) are behind "Show
unbound inputs".

**The picture.** The pad (drawn with its sticks offset, or side by side
and with PlayStation button names when gilrs's name or vendor for the
connected pad looks like Sony's; a button on the picture's corner chooses
by hand), the wheel and pedals, the gun, or the keyboard, with each part
this game uses labelled beside it with a leader line, and the focused
action's parts glowing (its bindings are in the line under the picture).
The art is `crates/play/assets/controls/` (original, with its prompts in
its README); the parts' places are in `menu/figure.rs`. Actions the
wheel or gun has no part for are listed under it, and keys off the
picture's main block (the numeric pad's) aren't pictured but are in the
action rows.

**Conflicts.** A key or button bound to two inputs is allowed (both get
it) but marked: its chip turns yellow and the row says "also Jump tail",
the footer says some are doubled, and binding one raises a notice ("Z is
also Jump tail"), except where the manifest itself binds it so
(`skisuprg`'s two ski angles share the arrows on purpose). The player's own
keys (Esc, F5, F8, F9, F11) can't be bound: pressing one while it waits
says so and waits for another.

**Tuning.** For a game with an analog input bound to a stick, and one
bound to a trigger (pedals), a card gives each a **dead zone** (0 to 40%),
a **sensitivity** (50 to 150%) and a **curve** (Linear; Gentle, which
counts small movements for less; Quick, for more), with a graph of the
response and bars for what the stick reads raw and what the game gets,
live. The pad steps a row with left and right, the mouse drags the track.
The defaults (a stick's 15% dead zone, a trigger's none, Linear, 100%)
read exactly as before. Tuning follows the **This game only / All games**
switch, as the Picture page's settings do, and is kept with the settings
as `stick_dead`, `stick_sens`, `stick_curve` and `pedal_dead`,
`pedal_sens`, `pedal_curve` (the sticks' dead zone is a step from none to
its size at its edge, as it always has been: the curve bends what lies
beyond). Bindings are always the game's own: a different game has other
actions, so there is nothing for "all games" to share.

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
directory), the player keeps its files in `out/`; anywhere else in the
platform's data folder: `$XDG_DATA_HOME/remodel` (normally
`~/.local/share/remodel`) on Linux, `~/Library/Application Support/remodel`
on macOS, `%APPDATA%\remodel` on Windows:

- `play-settings.txt`: the settings, as `key = value` lines: the ROM
  folder, the games played (most recent first), fullscreen, where the
  settings panels save, and the game settings set for all games.
- `<set>/settings.txt`: the game's own settings, only where they differ
  from those for all games.

  A game setting whose value doesn't parse, in either file, is ignored
  with a warning on the terminal (the setting falls back to the next
  layer down, or its default).

  Starting a game prints a `settings` line on the terminal: every game
  setting that isn't the built-in default, with where it came from (all
  games, this game, the command line), and the two files read; changing
  one in the launcher or menu prints what changed and where it was saved:

  ```
  settings      srallyc: temporal taa (this game), aa 8 (all games); the rest default; files out/play-settings.txt, out/srallyc/settings.txt
  settings      srallyc: temporal taa -> off (saved for this game)
  ```
- `play.log`: this session's log, everything the player printed on the
  terminal (it goes to both), so a run started without a terminal can
  still be reported; the session before is kept as `play.prev.log`. A
  panic is written there with its backtrace. It starts with a `reModel`
  line (version, OS, CPU, JIT workers, the data folder), then `display`
  (monitor, refresh rate, scale, window) and `audio` (the output device,
  its rate, channels and buffer). Then, as they happen: each game's `game`
  and `settings` lines, `save` and `load` (the file, the frame, whether it
  worked), every `notice` the screen shows, `pacing` (synced to the
  display, or paced by the clock, and the frame costs that decided it),
  `underrun` (the audio ran dry: frame, queue; once a second at most),
  `controller` connects and disconnects, and the `--perf-log` lines if
  on. (On Windows only the panic and the player's own start-up lines
  reach the file for now.)
- `<set>/controls.txt`: the game's remapped controls, only what differs
  from its manifest (above, "Remapping").
- `<set>/eeprom.bin`: the game's EEPROM (settings, records), written on quit or on switching games.
- `<set>/saves/quick.save`, `slot1.save` ...: save states, zlib-compressed,
  about 1.2 MB each. They only load into the same program ROMs, and carry
  a format number and the reModel version that wrote them (docs/usage.md,
  "Versions and releases"): a state from a newer format is refused with a
  message in the menu.
  `resume.save` is the one Continue loads.
- `<set>/saves/quick.png`, `slot1.png` ...: the slot's thumbnail, beside its
  state: 224x168 (4:3), drawn from the machine as the state is saved (the
  game's own frame, without bezel or menu, anti-aliased by multisampling
  where temporal anti-aliasing is on), encoded on its own thread so a save
  doesn't wait for it. It is made only by the player, never read by the
  emulator: the state file and its hash are the same with or without it,
  and a state without one (saved by an older reModel, or a frame that
  couldn't be drawn) shows "No preview". The slots page reads them when it
  opens. Delete freely.
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
licence, and the licence texts it ships, plus the player's own bundled fonts
(Inter and Exo 2, SIL OFL 1.1). The version is the workspace's from Cargo plus
`git describe`. It runs the i960 on the interpreter only: recompiled code is
derived from the game's ROMs, so the target builds from the checkout's own
workspace (in its own `target/dist`), never the generated one in
`out/recomp/`, checks that it linked the empty `crates/recomp` stub, and
refuses `RECOMP=1`.

Unpacked anywhere outside the source tree, it keeps its files in the
data folder (above) and looks for sets in `roms/` there; with none
found it opens the launcher, where another ROM folder can be typed in.
