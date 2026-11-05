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

## The menu

- **Save states**: a quick slot (F5/F9) and four more. A save holds the
  whole machine, both sound boards included, so loading one resumes the
  sound exactly where it was. The one thing not saved is a replacement
  soundtrack voice that's playing (`docs/mods.md`).
- **Sound**: the effects (sound board) and music (DSB2 or DSB1) levels in dB; the
  default 8:-8 was set by ear (`docs/hw/sound.md`).
- **Display**: where the HUD goes on wide windows, and exact arcade speed
  (57.524 frames per second) against the default of one game frame per
  display refresh at 60 or 120 Hz, which is 4.3% fast but smooth.
- **Game**: the ROM folder and the supported sets found in it. Sets are
  read as MAME keeps them: `<set>.zip` or a `<set>/` folder. With no ROM
  folder set up, or the last game missing, the player starts in the menu.
  The ROM folder defaults to `rom/` in the source tree (or wherever the
  current directory has a `rom/` folder), else to `roms/` beside the
  settings (`~/.local/share/remodel/roms`), created on first run.

Later, the browser build's game list (`docs/web.md`: a card per game with
an attract mode screenshot, its year and controls, grouped by board) could
become the player's start screen in place of the Game page of the menu.
The cards come from `remodel web-index` and `remodel shot`, which work from
the manifests alone.

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
about 6 MB: the stripped release binary and a `README.txt` (bring your own
ROMs, keys, where files go). The version is the workspace's from Cargo plus
`git describe`. It runs the i960 on the interpreter only: recompiled code is
derived from the game's ROMs, so the target builds without
`REMODEL_RECOMP_DIR` (in its own `target/dist`), checks that the recompiled
registry came out empty, and refuses `RECOMP=1`.

Unpacked anywhere outside the source tree, it keeps its files in
`$XDG_DATA_HOME/remodel` and looks for sets in `roms/` there; with none
found it opens in the menu, where another ROM folder can be typed in.
