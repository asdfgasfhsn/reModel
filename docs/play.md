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

| Key | |
|---|---|
| C, Enter | coin, start |
| arrows | steer (Down slides) |
| Z, X | jump front, jump tail |
| Q, E | select |
| Esc | menu (the game pauses while it's open) |
| F5, F9 | quick save, quick load |
| F11 | fullscreen |

Gamepads work too; the one used last drives the game.

## The menu

- **Save states**: a quick slot (F5/F9) and four more. A save holds the
  whole machine, both sound boards included, so loading one resumes the
  sound exactly where it was. The one thing not saved is a replacement
  soundtrack voice that's playing (`docs/mods.md`).
- **Sound**: the effects (sound board) and music (DSB2) levels in dB; the
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
