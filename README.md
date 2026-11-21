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
run-ahead to cut input lag, anti-aliasing and an optional depth buffer, smooth
shading, texture packs with material maps, and custom soundtracks. Everything that is not specific to one game is
built to be reused across the Model 2 family.

> **Bring your own ROMs.** reModel contains no game code or data. See
> [Provenance and legal](#provenance-and-legal).

## Games

Thirty-four games run, on the original Model 2 and the 2A, 2B and 2C boards. "Plays" means playable with
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
| Sonic The Fighters (Sonic Championship, Japan) | `sfight` | 2B | plays |
| Virtua Striker | `vstriker` | 2B | plays |
| Last Bronx | `lastbrnx` | 2B | plays |
| Sega Rally Championship | `srallyc` | 2A | plays; its program runs on an MB86234 interpreter |
| Virtua Fighter 2 | `vf2` | 2A | plays; its program runs on an MB86234 interpreter |
| Daytona USA | `daytona` | Model 2 | plays; the first original Model 2 game, its program runs on an MB86234 interpreter |
| Virtua Cop | `vcop` | Model 2 | plays, with light guns; stages 1 to 3 start and play, whole stages not yet shot through to their bosses |
| Desert Tank | `desert` | Model 2 | plays; the desert mission and the canyon (EXPERT) mission, steered by handle and pedal |
| Virtua Cop 2 | `vcop2` | 2A | plays, with light guns; all three stages start and play, no stage shot through to its end |
| Sky Target | `skytargt` | 2A | plays, with a flight stick; mission 1 to its boss, no mission finished |
| Manx TT Superbike | `manxtt` | 2A | plays; both courses, automatic or manual gearbox, to the end of the clock; no race steered to a finished lap |
| Manx TT Superbike (twin cabinet) | `manxttc` | 2A | plays; the twin-cabinet release, a clone of `manxtt` |
| Motor Raid | `motoraid` | 2A | plays; a Championship rides stage 1 to the finish and into stage 2 |
| Gunblade NY | `gunblade` | 2B | plays, with mounted guns; the first mission to game over, none finished |
| Dynamite Cop | `dynamcop` | 2A | plays; mission 1 on the ship's deck to game over, standing and punching; no mission finished |
| Zero Gunner | `zerogun` | 2B | plays; the first mission (Asia) to game over, none finished |
| Pilot Kids | `pltkids` | 2B | plays; stage 1 to its boss and game over, none finished |
| Rail Chase 2 | `rchase2` | 2B | plays, with the riders' sticks; the first stage, none finished |
| Cyber Troopers Virtual-On | `von` | 2B | plays, with twin sticks; its link board answered; the first fight to a draw, none won |

## Highlights

- **Whole games, not demos:** attract mode, coin-up, menus and full play
  through to game over, with sound and music, controls (keyboard, gamepad,
  touch, light guns), and save states.
- **Fast:** most games run at 2x to 4x real time natively, recompiled;
  native geometrizers replace the geometry programs (see [docs/gpu.md](docs/gpu.md),
  [docs/recompiler.md](docs/recompiler.md)).
- **Enhancements:** widescreen views, smooth motion, run-ahead, anti-aliasing
  (multisampled, or temporal with upscaling in the native player), a depth
  buffer, smooth shading, bezels, texture packs and soundtrack mods
  ([docs/enhancements.md](docs/enhancements.md),
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
make play                          # the launcher: pick a game, then play (on the JITs, docs/jit.md)
make play GAME=hotd                # straight into a game (Esc menu, F8 photo mode)
make play GAME=srallyc PLAYERS=2   # two to four linked machines on one split screen, a controller each (docs/play.md)
make play GAME=srallyc PLAY_ARGS=--host   # linked play over a LAN, a machine each: host here, `--join IP:PORT` there (docs/play.md)
make play GAME=srallyc JIT=0       # without the JITs: the interpreters (RECOMP=1 JIT=0: recompiled code)
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
- [docs/photo-mode.md](docs/photo-mode.md): the player's photo mode (free camera, time scrub, depth of field, captures).
- [docs/web.md](docs/web.md): the browser build.
- [docs/roms.md](docs/roms.md): the ROM sets, their boards and what each game needs.
- [docs/variants.md](docs/variants.md): a survey of every clone (revision, region, hack) of the supported games, what each needs, and how they could show as versions of a title.
- [docs/status.md](docs/status.md): the status table; what is done, in progress and planned.
- [CHANGELOG.md](CHANGELOG.md): the tagged versions and what each one brought.
- [docs/architecture.md](docs/architecture.md): the crate layout and what is shared across Model 2.
- [docs/recompiler.md](docs/recompiler.md): the static recompiler.
- [docs/jit.md](docs/jit.md): the JITs (a spike): the i960's and the copro programs' code translated at run time, opt-in.
- [docs/idle-skip.md](docs/idle-skip.md): fast-forwarding exactly through the games' idle loops.
- [docs/gpu.md](docs/gpu.md): the GPU renderer.
- [docs/enhancements.md](docs/enhancements.md): visual enhancements, done and planned (smooth shading, material maps, temporal anti-aliasing, ...).
- [docs/texture-packs.md](docs/texture-packs.md): texture dumps and packs, the pack format and material maps.
- [docs/uplift-studio.md](docs/uplift-studio.md): the uplift studio: restyling a game's art through texture packs (concept, texture map, select, restyle, check, show); `make texmap` says which texture is where, and the `make studio-*` targets (`remodel studio`) select, restyle, generate, check and compare a project of textures. `make catalogue` (and `remodel-play --catalogue`) build a catalogue of every texture, tile, layout and object a game shows, and the textures it loads, by observing it; `make tour-run` and `make coverage` run a game's tours and say how much of its polygon and texture ROM they have read.
- [docs/uplift-vision.md](docs/uplift-vision.md): where uplift is going: the vision, a map of capabilities and a ranked roadmap, with one spec per capability in [docs/uplift/](docs/uplift/README.md) (a design; not built).
- [docs/mods.md](docs/mods.md): custom soundtracks and other mods.
- [docs/mame-defects.md](docs/mame-defects.md): where MAME differs from the hardware, written up for upstream.

Games: [docs/games/](docs/games/README.md) has one page per game with its findings.

Hardware notes ([docs/hw/](docs/hw/)):

- [model2.md](docs/hw/model2.md), [model2a.md](docs/hw/model2a.md), [model2b.md](docs/hw/model2b.md), [model2c.md](docs/hw/model2c.md): the original board and its three successors.
- [peripherals.md](docs/hw/peripherals.md): the 2C board's I/O and peripherals.
- [link-board.md](docs/hw/link-board.md): the link (network) board as the games use it, `srallyc` in detail, a design sketch for linked play, and linked play between machines over a LAN; [eeprom-records.md](docs/hw/eeprom-records.md): the Sega settings records in the EEPROM and their checksums.
- [geometrizer.md](docs/hw/geometrizer.md): the geometrizer and 3D rasteriser; [geometrizer-2c.md](docs/hw/geometrizer-2c.md) and [geometrizer-2b.md](docs/hw/geometrizer-2b.md): the native geometrizers.
- [texture-lod.md](docs/hw/texture-lod.md): mipmaps, the log table and microtextures, as the games send them.
- [mb86235.md](docs/hw/mb86235.md) (2C geometry chip), [mb86234.md](docs/hw/mb86234.md) (2A), [adsp2106x.md](docs/hw/adsp2106x.md) (2B SHARC).
- [tilemap.md](docs/hw/tilemap.md): the 2D tile layer.
- [sound.md](docs/hw/sound.md): 68000 + SCSP and the DSB2 music board; [dsb1.md](docs/hw/dsb1.md): the DSB1; [multipcm.md](docs/hw/multipcm.md): the original Model 2's MultiPCM sample chips; [z80.md](docs/hw/z80.md): the Z80 core.

Asset conventions: [logo and app icon](assets/README.md), [posters](assets/posters/README.md), [bezels](assets/bezels/README.md).

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
also ship `THIRD-PARTY-NOTICES.txt` for the crates they link and the
player's bundled fonts, Inter and Exo 2 under the SIL OFL (`tools/notices.py`). Game software and data remain the property of their
respective owners.
