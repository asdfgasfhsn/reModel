<p align="center"><img src="assets/reModel-logo.png" alt="reModel" width="640"></p>

# reModel

**A static-recompilation and emulation toolkit for Sega Model 2 arcade games.**

reModel runs Model 2 arcade games on a modern PC. It turns each game's own
program code into native code ahead of time (static recompilation) and
replaces the custom hardware, the geometry processors, the renderer and the
sound boards, with reimplementations, so the games run at full speed with the
original behaviour intact. There is a native player (`make play`) and a
browser build (WebAssembly and WebGPU, `make web`). On top of the faithful
picture it can add what the hardware never had: any resolution and widescreen
views with the HUD moved to the edges, smooth motion for fast displays,
run-ahead to cut input lag, anti-aliasing and an optional depth buffer, texture
packs, and custom soundtracks. Everything that is not specific to one game is
built to be reused across the Model 2 family.

> **Bring your own ROMs.** reModel contains no game code or data. See
> [Provenance and legal](#provenance-and-legal).

## Games

Eighteen games run, on the 2A, 2B and 2C boards. "Plays" means playable with
sound in the native player; the details, and what each game needed, are in
[docs/games/](docs/games/README.md).

| Game | Set | Board | Status |
|---|---|---|---|
| Top Skater | `topskatr` | 2C | plays; the first target, with its DSB2 music board |
| Sega Ski Super G | `skisuprg` | 2C | plays (its drive board answered) |
| Sega Touring Car Championship | `stcc` | 2C | plays, sound and DSB1 music |
| Sega Water Ski | `segawski` | 2C | plays |
| Wave Runner | `waverunr` | 2C | plays |
| Over Rev | `overrev` | 2C | plays |
| The House of the Dead | `hotd` | 2C | plays, with light guns |
| Behind Enemy Lines | `bel` | 2C | plays, with mounted guns |
| Over Rev (2B version) | `overrevb` | 2B | plays |
| Super GT 24h | `sgt24h` | 2B | plays |
| Dynamite Baseball '97 | `dynabb97` | 2B | plays |
| Dead or Alive | `doa` | 2B | plays, protection device answered from the ROM |
| Indianapolis 500 | `indy500` | 2B | plays |
| Fighting Vipers | `fvipers` | 2B | plays |
| Sonic Championship | `schamp` | 2B | plays |
| Virtua Striker | `vstriker` | 2B | plays |
| Last Bronx | `lastbrnx` | 2B | plays |
| Sega Rally Championship | `srallyc` | 2A | plays; its program runs on an MB86234 interpreter |

## Highlights

- **Whole games, not demos:** attract mode, coin-up, menus and full play
  through to game over, with sound and music, controls (keyboard, gamepad,
  touch, light guns), and save states.
- **Fast:** most games run at 2x to 4x real time natively, recompiled;
  native geometrizers replace the geometry programs (see [docs/gpu.md](docs/gpu.md),
  [docs/recompiler.md](docs/recompiler.md)).
- **Enhancements:** widescreen views, smooth motion, run-ahead, anti-aliasing
  (multisampled, or temporal with upscaling in the native player), bezels, texture packs and soundtrack mods ([docs/enhancements.md](docs/enhancements.md),
  [docs/mods.md](docs/mods.md)).
- **In the browser:** the same games on WebGPU, with a settings panel, gamepad
  and touch controls ([docs/web.md](docs/web.md)).
- **Documented hardware:** what the games' own code shows about the boards,
  chips and protocols is written down in [docs/hw/](docs/hw/), with
  confirmed and unconfirmed findings marked apart.

## Quick start

Requires a Rust toolchain (stable) and `make`.

```sh
make help                          # every target, always current
make check                         # format, clippy, unit tests (no ROMs needed)
# put your ROM sets in ./rom, as MAME keeps them (rom/topskatr.zip or rom/topskatr/)
make verify                        # check the chips against known-good hashes
make scan                          # list the sets found in rom/
make play RECOMP=1                 # the launcher: pick a game, then play
make play GAME=hotd RECOMP=1       # straight into a game
make web RECOMP=1                  # browser build -> out/web (needs unzipped rom/<set>/ folders; serve over HTTPS)
```

`make recomp` builds the recompiled code once a game has been run; `RECOMP=1`
then uses it. Every target takes `GAME=<set>`. `make help` is the authority;
the worked examples (runs, snapshots, sound, benchmarks, capture) are in
[docs/usage.md](docs/usage.md).

## Documentation

Guides:

- [docs/usage.md](docs/usage.md): the full command catalogue with examples.
- [docs/play.md](docs/play.md): the native player, its launcher and settings.
- [docs/web.md](docs/web.md): the browser build.
- [docs/roms.md](docs/roms.md): the ROM sets, their boards and what each game needs.
- [docs/status.md](docs/status.md): the status table; what is done, in progress and planned.
- [docs/architecture.md](docs/architecture.md): the crate layout and what is shared across Model 2.
- [docs/recompiler.md](docs/recompiler.md): the static recompiler.
- [docs/gpu.md](docs/gpu.md): the GPU renderer.
- [docs/enhancements.md](docs/enhancements.md): visual enhancements, done and planned.
- [docs/texture-packs.md](docs/texture-packs.md): texture dumps and packs.
- [docs/mods.md](docs/mods.md): custom soundtracks and other mods.
- [docs/mame-defects.md](docs/mame-defects.md): where MAME differs from the hardware, written up for upstream.

Games: [docs/games/](docs/games/README.md) has one page per game with its findings.

Hardware notes ([docs/hw/](docs/hw/)):

- [model2a.md](docs/hw/model2a.md), [model2b.md](docs/hw/model2b.md), [model2c.md](docs/hw/model2c.md): the three boards.
- [peripherals.md](docs/hw/peripherals.md): the 2C board's I/O and peripherals.
- [geometrizer.md](docs/hw/geometrizer.md): the geometrizer and 3D rasteriser; [geometrizer-2c.md](docs/hw/geometrizer-2c.md) and [geometrizer-2b.md](docs/hw/geometrizer-2b.md): the native geometrizers.
- [mb86235.md](docs/hw/mb86235.md) (2C geometry chip), [mb86234.md](docs/hw/mb86234.md) (2A), [adsp2106x.md](docs/hw/adsp2106x.md) (2B SHARC).
- [tilemap.md](docs/hw/tilemap.md): the 2D tile layer.
- [sound.md](docs/hw/sound.md): 68000 + SCSP and the DSB2 music board; [dsb1.md](docs/hw/dsb1.md): the DSB1; [z80.md](docs/hw/z80.md): the Z80 core.

Asset conventions: [posters](assets/posters/README.md), [bezels](assets/bezels/README.md).

## Provenance and legal

- **No ROMs are included.** reModel contains no game code or data. You need
  your own dumps of games you own (bring your own ROMs); it checks them
  against known-good hashes. Never commit ROM files or generated listings:
  `rom/` and `out/` are git-ignored.
- **Not affiliated.** reModel is not affiliated with, endorsed by or
  connected to Sega or any game's other publishers (Tecmo, Jaleco, EPL and
  others). Game titles are used only to identify what it runs.
- **How the hardware notes were made.** They started from public information,
  including reading MAME's and Supermodel's source code for facts (some early
  notes in `docs/hw/tilemap.md`, `docs/hw/peripherals.md` and
  `docs/hw/mb86235.md` cite them). No emulator code was copied. Most of those
  facts have since been confirmed from the games themselves, and the notes
  mark what is still unconfirmed. Later work (Model 2B, Model 2A, the MB86234,
  the native geometrizers and the rest) is strictly clean-room: written from
  the games' own code and data and public chip manuals.

## License

MIT for reModel's own code ([LICENSE](LICENSE)). `make dist` and `make web`
also ship `THIRD-PARTY-NOTICES.txt` for the crates they link
(`tools/notices.py`). Game software and data remain the property of their
respective owners.
