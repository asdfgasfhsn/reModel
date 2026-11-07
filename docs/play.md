# The native player

`make play` (or `make play RECOMP=1` with recompiled code from `make recomp`)
builds and runs `remodel-play`: the game in a window, rendered on the GPU
(`docs/gpu.md`), with both sound boards running live. `remodel-play --help`
lists its options.

At startup the Remodel logo shows for 1.5 s, then fades out over 0.5 s
while the game boots underneath (or the menu, with no game to start). Any
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
without recompiled code. `--geo chip` (or `sharc`, `tgp`) runs the game's
own program instead, for this run only. The `remodel` CLI keeps the chip
as its default, since its oracles (`lockstep`, `geo-check`) test that
program.

**Smooth motion** (`--smooth`, this run only; off by default). On a
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

## Keys

The player's own keys are the same for every game:

| Key | |
|---|---|
| Esc | menu (the game pauses while it's open) |
| F5, F9 | quick save, quick load |
| F11 | fullscreen |

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

The game needs its drive board (the platform motor); Remodel answers its
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
never checks its reply; Remodel keeps the commands
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
Remodel reports; with a fresh EEPROM the shots land where the pointer is.

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

- **Save states**: a quick slot (F5/F9) and four more. A save holds the
  whole machine, both sound boards included, so loading one resumes the
  sound exactly where it was. The one thing not saved is a replacement
  soundtrack voice that's playing (`docs/mods.md`).
- **Sound**: the effects (sound board) and music (DSB2 or DSB1) levels in dB; the
  default 8:-8 was set by ear (`docs/hw/sound.md`).
- **Display**: the aspect (fit the window, the default, drawing the game
  wider than its frame from 4:3 to 32:9; or 4:3, `--aspect 4:3`), the
  bezel (below), where the HUD goes on wide windows, and exact arcade speed
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
  (`model2::audio::Pace`, described in `docs/web.md`): below full speed
  it slows with the game, lower in pitch, instead of running dry. Before,
  only a 0.5% drift correction steered the 50 ms queue, so `sgt24h`'s race
  start (its geometrizer at 18.7 ms a frame natively then, before the
  SHARC blocks; 6.8 ms now) would run it dry
  every few seconds, modelled from its frame costs; the title bar counts
  underruns.
- **Controls** (the button beside Resume): remap the game's keys and
  gamepad bindings, below.
- **Game**: the ROM folder and the supported sets found in it. Sets are
  read as MAME keeps them: `<set>.zip` or a `<set>/` folder, and a
  supported clone is listed after its parent when its chips are in the
  parent's (`overrevb` in `overrev.zip`). With no ROM
  folder set up, or the last game missing, the player starts in the menu.
  The ROM folder defaults to `rom/` in the source tree (or wherever the
  current directory has a `rom/` folder), else to `roms/` beside the
  settings (`~/.local/share/remodel/roms`), created on first run.

## Remapping

The menu's Controls page lists every input of the game being played with
its keys and gamepad bindings. Click a binding, then press the key or
gamepad button that should replace it; **+** adds one (an input can have
several), **×** removes one, **reset** puts one input back, and "Reset to
defaults" all of them. Esc cancels waiting for a key. Analog inputs (a
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

Later, the browser build's game list (`docs/web.md`: a card per game with
an attract mode screenshot, its year and controls, grouped by board) could
become the player's start screen in place of the Game page of the menu.
The cards come from `remodel web-index` and `remodel shot`, which work from
the manifests alone.

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

Run from the Remodel source tree (an `out/` directory in the current
directory), the player keeps its files in `out/`; anywhere else in
`$XDG_DATA_HOME/remodel` (normally `~/.local/share/remodel`):

- `play-settings.txt`: the settings, as `key = value` lines. Options given
  on the command line are remembered too; `--game` once that game starts.
- `<set>/eeprom.bin`: the game's EEPROM (settings, records), written on quit or on switching games.
- `<set>/saves/quick.save`, `slot1.save` ...: save states, zlib-compressed,
  about 1.2 MB each. They only load into the same program ROMs.

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
found it opens in the menu, where another ROM folder can be typed in.
