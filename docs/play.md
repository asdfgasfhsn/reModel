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
built into the binary and decoded with `remodel-png`.

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
chip it is off, and the menu and launcher say so), and it replaces
multisampling: `aa` is kept but off while temporal is on (the menu's
anti-aliasing choices are greyed out, the launcher's row says why). A GPU
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

**Texture packs** (settings `pack` and `tex_filter`, below; `--pack on|off`,
`--texture-pack DIR` and `--tex-filter off|mip|2|4|8|16` for one run; both
off by default). With `pack` on, replacement textures from the game's pack
(`docs/texture-packs.md`) are drawn on the GPU in place of the originals. The
pack is the folder `<data dir>/<set>/texpack/` (`out/<set>/texpack` in a
source tree; `make upscale-textures` makes one), and a clone with none uses
its parent's (*unconfirmed*: not tested). `--texture-pack DIR` names one for
every game that run and turns `pack` on; `--pack` alone means on. PNGs are
decoded on background threads when their texture is first drawn and the
original shows until then. The launcher's previews never use packs. The Esc
menu's line under the checkbox shows the folder, how many images it holds and
how many of the last frame's textured polygons were replaced, or why there
is no pack (a missing folder also gives a notice). `tex_filter` filters pack
images only: `off` (as they are), `mip` (mipmaps, no shimmer on distant
ground) or `2`, `4`, `8`, `16` (mipmaps plus anisotropic filtering up to that
many times, for ground at grazing angles); the menu offers only what the GPU
has. Neither changes what the game computes. Checked on `topskatr` with a
scripted run. *Not verified:* smooth motion's in-betweens and F5/F9 save
slots with a pack, and decode stutter was not measured.

**Smooth motion** (a setting, below; `--smooth` for one run; off by
default). On a
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

**Run-ahead** (a setting, below: off, 1, 2 or 3 frames; `--run-ahead N`
for one run; off by default). Every game takes some frames from reading
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

| Game | Control | Input lag (frames) | Run-ahead worth setting |
|---|---|---|---|
| `topskatr`, `skisuprg`, `segawski`, `waverunr` | curving, swing, lean, handlebar | 0 | off |
| `overrev`, `overrevb` | wheel | 0 | off |
| `vstriker` | stick, shoot | 0 | off |
| `hotd` | trigger | 0 (once 1) | off |
| `schamp` | stick 0, punch 1 | 0 | off (1 for punches) |
| `lastbrnx` | stick 0, punch 1 | 0 | off (1 for punches) |
| `stcc`, `indy500` | wheel | 1 | 1 |
| `srallyc` | wheel | 1 | 1, if the machine has 2x the game's speed (7 ms a frame interpreted) to spare |
| `doa` | punch, stick | 1 | 1 |
| `fvipers` | punch | 1 (8 once, mid-move) | 1 |
| `dynabb97` | bat, stick | 1 to 6, by the moment | 1 |
| `bel` | trigger | 3 (one measurement) | up to 3 |
| `sgt24h` | wheel | 4 | 3 |

Input lag here counts the frames after the first one that reads the
control, until the picture first differs from a run without it (`remodel
latency`, which `make input-lag` runs for every game: two machines from
the same frame of a run in play, one with the control held, their
renders compared pixel by pixel). 0 means that first frame already shows
it, which in the player is one frame from input to screen (the display
adds its own). The fighters were measured with no button already under
way; a move in progress can hold a control off for many frames (above,
the "once" values), which no run-ahead hides. Each game's least value is
in its manifest (`Game::input_lag`); the menu says it beside the setting.

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
they change as they change: aspect (fit the window or 4:3), bezel, HUD at
the edges or centred, speed (one frame a refresh or exact arcade speed),
smooth motion, anti-aliasing (off, 4x, 8x), temporal anti-aliasing (off,
TAA, 75%, 67%, 50%) and its checkerboard see-through (blend or keep), the depth buffer, the texture pack and its filtering (off, mip, 2x to 16x), the effects and music levels in dB, the geometrizer (native
or the game's chip; the preview restarts on it) and the Controls page
(remapping, below). "Save for" says where changes go: the defaults for all
games, or this game only (a game's own setting is tagged "this game", and
"Use the settings for all games" forgets them). The same settings are in
the Esc menu while playing. They come in layers, lowest first, as in the
browser build: the built-in defaults, the defaults for all games, the
game's own, then the command line (`--mix`, `--hud`, `--aspect`, `--bezel`,
`--exact`, `--smooth`, `--aa`, `--temporal`, `--stipple`, `--depth`, `--pack`, `--texture-pack`, `--tex-filter`, `--geo`), which applies to that run only and is
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

From a gamepad: the d-pad or left stick moves between the menu's buttons,
checkboxes, choices and sliders as they're laid out (repeating while held,
as in the launcher), A presses or ticks the one that glows cyan, left and
right move a slider or step through a choice, B or Select closes the menu
(or leaves the Controls page), and Start resumes. It opens on Resume. Its
foot shows the pad's buttons while the pad is what's being used.
`crates/play/src/focus.rs` does it for any egui page; the menu's tests
(`menu.rs`) drive it with `Nav`s headless. *Unconfirmed:* tested with
synthetic and scripted (`--nav`) events only, not yet with a real pad.

- **Save states**: a quick slot (F5/F9) and four more. A save holds the
  whole machine, both sound boards included, so loading one resumes the
  sound exactly where it was. The one thing not saved is a replacement
  soundtrack voice that's playing (`docs/mods.md`).
- **Sound**: the effects (sound board) and music (DSB2 or DSB1) levels in dB; the
  default 8:-8 was set by ear (`docs/hw/sound.md`).
- **Display**: the aspect (fit the window, the default, drawing the game
  wider than its frame from 4:3 to 32:9; or 4:3, `--aspect 4:3`), the
  bezel (below), where the HUD goes on wide windows, smooth motion,
  anti-aliasing, temporal anti-aliasing and the checkerboard pattern, and
  the depth buffer (above), the texture pack and its
  filtering (above), run-ahead (above), the geometrizer (on the next start), "Save changes
  for" this game or all games (as in the launcher), and exact arcade speed
  (57.524 frames per second) against the default of one game frame per
  display refresh at 60 or 120 Hz, which is 4.3% fast but smooth.
  **Below full speed** (a game frame taking longer than a refresh, as
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
  (`model2::audio::Pace`, described in `docs/web.md`, "Audio rate"): once
  the game has been below full speed for 0.15 s (three times the 50 ms
  queue) it slows with it, lower in pitch, instead of running dry; while
  it keeps up only the queue's drift correction (at most 1%, inaudible)
  moves the pitch, so a frame dropped now and then is absorbed by the
  queue rather than heard. Before,
  only a 0.5% drift correction steered the 50 ms queue, so `sgt24h`'s race
  start (its geometrizer at 18.7 ms a frame natively then, before the
  SHARC blocks; 6.8 ms now) would run it dry
  every few seconds, modelled from its frame costs; the title bar counts
  underruns.
- **Controls** (the button beside Resume): remap the game's keys and
  gamepad bindings, below.
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

The menu's Controls page lists every input of the game being played with
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
own keys (Esc, F5, F9, F11) can't be bound: pressing one while it waits
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
- `<set>/eeprom.bin`: the game's EEPROM (settings, records), written on quit or on switching games.
- `<set>/saves/quick.save`, `slot1.save` ...: save states, zlib-compressed,
  about 1.2 MB each. They only load into the same program ROMs.
  `resume.save` is the one Continue loads.
- `<set>/preview-native.save` (`-chip` with the chip geometrizer): the
  launcher's attract mode preview state.
- `<set>/texpack/`: the game's texture pack, if you make one (PNGs named
  as a dump's).

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
