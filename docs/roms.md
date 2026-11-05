# The ROM sets in `rom/`

What's in `rom/`, which board each game runs on, and which game to support
after `topskatr`. Nothing here quotes ROM contents; set names, chip names,
sizes and CRCs are metadata.

**Sources and how sure we are.** Each claim carries one of these tags:

- **ROM**: read from the zips themselves (member names, sizes, CRCs) or found
  by searching the chips (see "How this was checked" at the end). Firm.
- **dat**: the clrmamepro dat bundled in `m2emu1.1a.zip` (author "oak2003",
  for ElSemi's Model 2 Emulator 1.1a). It gives titles, years, parent/clone
  links, the full chip list of every set and, through its order and its
  descriptions ("Model 2A", "Model 2B", "Model 2C"), the board. It is a third
  party's catalogue: good for names and completeness, unconfirmed for boards.
- **gk**: general knowledge, mostly MAME's driver. Unconfirmed. MAME does
  not run `topskatr`, so its board facts are hypotheses (see the README).

## What's in the folder

- 32 set zips, each byte-identical to the "Model 2 Romset (Merged)" folder
  inside `m2emu1.1a.zip` (ROM: CRC of every zip compared). They are merged
  sets: a parent's zip also holds its clones' chips.
- `m2emu1.1a.zip` (642 MB) is not a game. It is the same 32 zips again, plus
  the dat, a configurator and ElSemi's Model 2 Emulator 1.1a (a closed-source
  Windows binary with a few Lua scripts). Remodel is clean-room: use the dat
  as a catalogue, don't use the emulator. `remodel scan` lists it as a set
  ("m2emu1.1a, not supported yet") because it treats every zip as one.
- `topskatr/` is the 20 chips of `topskatr.zip`'s parent set, unpacked; every
  file's CRC matches the zip (ROM). The zip also holds the clones
  `topskatrj` and `topskatru` (two program chips each, in subfolders).
- Completeness: every set in the dat, parents and clones, is present in full
  in its zip, by CRC; no zip has a chip the dat doesn't list (ROM + dat). The
  dat's sets mostly follow MAME's names, but this was not checked against
  MAME's own list, so a MAME-specific set or chip could still be missing.

## Board generations and what they mean for Remodel

Every board has the i960KB main CPU, the 68000 + SCSP sound board, the
same tilemap/2D layers and the same kind of polygon raster stream, so the
i960 recompiler, ROM loading, the sound board, the GPU renderer and both
players carry over (gk for the shared parts; the README's reuse table says
the same). The geometry hardware is what changes:

| Board | Geometry (gk unless marked) | What Remodel would need |
|---|---|---|
| Model 2 (1993-94) | Fujitsu TGP MB86234; its program is inside the chip and not dumped. | A TGP model (high-level, function by function, as MAME does), the original board's memory map and its sound board ROM layout. Large new work. |
| 2A-CRX (1995) | Same TGP as Model 2, on a cheaper board. | As Model 2, plus 2A's map. Once one 2A game runs, the other 13 2A sets are mostly manifests. |
| 2B-CRX (1995) | Analog Devices ADSP-21062 SHARC copro; the i960 uploads its program. | A SHARC core (interpreter, ideally a recompiler) and 2B's copro interface. The geometrizer side is unknown: 2B sets contain none of `topskatr`'s geometrizer code (ROM). |
| 2C-CRX (1996-98) | Fujitsu MB86235 (TGPx4) copro and geometrizer, both running programs the i960 uploads. **ROM:** every 2C program set contains MB86235 code shared with `topskatr`'s uploaded programs; no 2A or 2B set does. | Mostly a manifest per game. The MB86235 interpreter and recompiler, the geometrizer, the 2C memory map and the sound board are all there. New work is per game: inputs, any extra board, and whatever the game's code uses that `topskatr`'s doesn't. |

Two ROM clues sort the boards (the chips are ROM evidence; what they are is gk): every Model 2 and 2A
set carries the same nine shared chips, `opr-14742a.45` to `opr-14747.63`
(6 x 128 KB) and `mpr-16310.15`, `mpr-16311.16`, `mpr-16312.14`
(3 x 512 KB); no 2B or 2C set does. MAME treats the 512 KB three as the
TGP's function tables (gk). The 2B/2C zips that contain them (`doa`,
`dynamcop`, `pltkids`, `zerogun`) only do so because of a 2A clone merged
in.

**Sound.** All sets have the 68000 + SCSP board (gk). The sound programs of
12 sets carry the same Sega driver ID string as `topskatr`'s ("CRX_drv",
with a version): 3.2.8 like `topskatr` in `hotd`, `skisuprg`, `dynamcop`,
`dynabb97` and `motoraid`; others 3.1.1 to 3.4.9 (ROM). The rest (the
1994-95 Sega games, Jaleco's two, EPL's `bel`) use other drivers or other
ID strings. The SCSP model is generic,
so a different driver should mostly just work. Two sets have a music board
(ROM, from `.Ns` socket names and the program chips' first instructions):
`topskatr`, DSB2 (68000, `mpr-19760.2s`, MPEG ROMs `.18s`-`.24s`), and `stcc`,
DSB1 (Z80, `epr-19275.2s`, MPEG ROMs `.57s`-`.60s`). No other set has
`s`-socket chips, so if any other game has a DSB its chips are named
differently (weak; gk says none of these do).

## The sets

Years are the dat's; a few look like late or export dates (gk: Daytona USA,
Desert Tank and Virtua Fighter 2 are 1994). Parent/clone is the dat's.
Board: dat, plus the ROM clues above. Geometry and main CPU follow from the
board (i960KB on all, gk). Controls and special hardware are gk throughout.
"Complete" is ROM + dat: all chips of every set listed, by CRC.

### Model 2 (original)

| Set | Title | Year | Clones in zip | Sound | Controls | Special hardware | Complete |
|---|---|---|---|---|---|---|---|
| `daytona` | Daytona USA | 1996 (dat) | `dayton93`, `daytonas`, `daytonase`, `daytonat`, `daytonata`, `daytonagtx`, `daytonam` | SCSP | wheel, pedals, 4-speed shifter, view buttons | drive board (force feedback); link up to 8 | yes (66 files). Several clones are fan hacks (non-Sega chip names such as `gtx.12`, `maxx.12`, `bank0.bin`; gk) |
| `desert` | Desert Tank | 1996 (dat) | none | SCSP | tank handles with triggers, pedal (gk, weak) | motion cabinet; extra 128 KB `epr-16985.7` on a socket the main board also uses, so on another board (ROM) | yes |
| `vcop` | Virtua Cop | 1994 | `vcopa` | SCSP | 2 light guns | none known | yes |

### Model 2A-CRX

| Set | Title | Year | Clones in zip | Sound | Controls | Special hardware | Complete |
|---|---|---|---|---|---|---|---|
| `manxtt` | Manx TT Superbike (Rev D, Deluxe) | 1995 | none (`manxttc` is its own set) | SCSP | motorbike: lean steering, throttle, brake | deluxe moving bike; 128 KB `epr-18742.7` on another board (ROM); link | yes |
| `manxttc` | Manx TT Superbike (Rev C, Twin) | 1995 | none | SCSP | as `manxtt` | twin cabinet; link | yes |
| `motoraid` | Motor Raid | 1997 | none | SCSP, CRX_drv 3.2.8 | motorbike handlebars | ride-on cabinet | yes |
| `srallyc` | Sega Rally Championship | 1995 | `srallycb`, `srallyp` (prototype-named chips `ic12.bin`, `ic13.bin`) | SCSP | wheel, pedals, shifter | drive board; link | yes |
| `skytargt` | Sky Target | 1995 | none | SCSP | flight stick with trigger | none known | yes |
| `vcop2` | Virtua Cop 2 | 1995 | none | SCSP | 2 light guns | none known | yes |
| `vf2` | Virtua Fighter 2.1 | 1995 | `vf2o`, `vf2a`, `vf2b` | SCSP | stick, 3 buttons | none | yes |
| `doaa` | Dead or Alive (2A) | 1996 | clone of 2B `doa`, in `doa.zip` | SCSP | stick, 3 buttons | none | yes |
| `dynamcop` | Dynamite Cop (2A) | 1998 | `dyndeka2` (2A), `dynmcopb`, `dyndek2b` (2B), `dynmcopc` (2C) | SCSP, CRX_drv 3.2.8 | stick, 3 buttons | none | yes |
| `pltkidsa` | Pilot Kids (2A) | 1998 | clone of 2B `pltkids` | SCSP, CRX_drv 3.4.9 | stick, buttons | none | yes |
| `zeroguna`, `zerogunaj` | Zero Gunner (2A) | 1997 | clones of 2B `zerogun` | SCSP | stick, buttons | none | yes |

### Model 2B-CRX

| Set | Title | Year | Clones in zip | Sound | Controls | Special hardware | Complete |
|---|---|---|---|---|---|---|---|
| `doa` | Dead or Alive | 1996 | `doaa` (2A) | SCSP | stick, 3 buttons | none | yes |
| `dynabb97` | Dynamite Baseball 97 (Rev A) | 1997 | none | SCSP, CRX_drv 3.2.8 | stick, buttons (gk, weak) | none known | yes |
| `fvipers` | Fighting Vipers | 1995 | none | SCSP | stick, 3 buttons | none | yes |
| `gunblade` | Gunblade NY | 1995 | none | SCSP | 2 mounted machine guns | none known | yes |
| `indy500` | Indianapolis 500 (Rev A, Twin) | 1995 | `indy500d`, `indy500to` | SCSP | wheel, pedals, shifter | drive board; link | yes |
| `lastbrnx` | Last Bronx (Export, Rev A) | 1996 | `lastbrnxj`, `lastbrnxu` | SCSP, CRX_drv 3.1.1 | stick, 3 buttons | none | yes |
| `overrevb` | Over Rev (2B) | 1997 | clone of 2C `overrev` | SCSP (Jaleco driver) | wheel, pedals, shifter | unknown | yes |
| `pltkids` | Pilot Kids | 1998 | `pltkidsa` (2A) | SCSP, CRX_drv 3.4.9 | stick, buttons | none | yes (Psikyo chip names: `.pr0`, `.sd0`, `.tp0`...) |
| `rchase2` | Rail Chase 2 | 1995 | none | SCSP | 2 mounted guns | motion ride | yes |
| `schamp` | Sonic Championship | 1996 | `sfight` | SCSP | stick, 3 buttons | none | yes |
| `sgt24h` | Super GT 24h | 1996 | none | SCSP (Jaleco driver) | wheel, pedals, shifter | 128 KB `epr-18643a.7` on another board (ROM; role unknown, also in `von`, `stcc`) | yes |
| `vstriker` | Virtua Striker (Rev A) | 1994 | `vstrikro` | SCSP | stick, 3 buttons | none | yes (`ep`/`mp` chip names) |
| `von` | Virtual On Cybertroopers (US) | 1996 | `vonj` | SCSP | twin sticks, triggers | link (2 cabinets); `epr-18643a.7` | yes (US program chips named `vo-prog0.usa`) |
| `zerogun` | Zero Gunner | 1997 | `zerogunj`, `zeroguna`, `zerogunaj` (2A) | SCSP, CRX_drv 3.4.9 | stick, buttons | none | yes |
| `dynmcopb`, `dyndek2b` | Dynamite Cop / Deka 2 (2B) | 1998 | clones of 2A `dynamcop` | SCSP | stick, 3 buttons | none | yes |

### Model 2C-CRX

| Set | Title | Year | Clones in zip | Sound | Controls | Special hardware | Complete |
|---|---|---|---|---|---|---|---|
| `topskatr` | Top Skater | 1997 | `topskatrj`, `topskatru` | SCSP, CRX_drv 3.2.8; **DSB2** | skateboard deck (2 analog channels), jump buttons (confirmed, README) | none | yes; also unpacked in `topskatr/` |
| `skisuprg` | Sega Ski Super G | 1996 | none | SCSP, CRX_drv 3.2.8 | ski platform (analog channels 0-2: two ski angles, a third unconfirmed), three panel buttons (confirmed, README) | drive board (confirmed: boot checks it; modelled, README) | yes; supported and playable |
| `segawski` | Sega Water Ski | 1997 | none | SCSP, CRX_drv 3.4.8 | water-ski platform and tow handle (analog) | none known | yes |
| `waverunr` | Wave Runner | 1996 | none | SCSP, CRX_drv 3.2.7 | personal watercraft: handlebar, throttle (analog) | ride-on cabinet; link (weak) | yes |
| `hotd` | The House of the Dead | 1997 | none | SCSP, CRX_drv 3.2.8 | 2 light guns | none known | yes |
| `stcc` | Sega Touring Car Championship | 1996 | `stcca`, `stccb` (the zip holds all three program pairs; Remodel uses `epr-19570/19571`) | SCSP, CRX_drv 3.2.5 (confirmed); **DSB1 (Z80)**, modelled (`docs/hw/dsb1.md`) | wheel (ch. 0), gas (1), brake (2), shift up/down buttons, two view buttons (confirmed, README) | wheel drive board (commanded every frame, reply never checked; modelled, README); link (skipped by the default settings); `epr-18643a.7` (Z80 code for another board) | yes; supported and playable, with music |
| `overrev` | Over Rev | 1997 | `overrevb` (2B) | SCSP (Jaleco driver) | wheel, pedals, shifter | unknown | yes |
| `bel` | Behind Enemy Lines | 1998 (Sega / EPL) | none | SCSP (EPL driver; 128 KB sound program) | gun (gk, weak) | unknown | yes |
| `dynmcopc` | Dynamite Cop (2C) | 1998 | clone of 2A `dynamcop` | SCSP, CRX_drv 3.2.8 | stick, 3 buttons | none | yes (2 own chips; the rest comes from the 2A parent) |

Counts, by parent (one zip each): 3 original Model 2, 8 2A, 13 2B, 8 2C.
Clones on another board than their parent: 2A `doaa`, `pltkidsa`,
`zeroguna`, `zerogunaj`; 2B `overrevb`, `dynmcopb`, `dyndek2b`; 2C
`dynmcopc`. So `doa.zip`, `dynamcop.zip`, `overrev.zip`, `pltkids.zip` and
`zerogun.zip` each hold two or three boards' sets.

## Second game

Only 2C games reuse the MB86235 work, so the choice is among the eight 2C
games, ranked by how little new work each needs:

1. **`skisuprg`, Sega Ski Super G.** Closest to `topskatr`: same driver
   version (3.2.8), no music board, and nearly the
   same chip layout (program `.15/.16` plus a `.13/.14` pair, data
   `.9`-`.12`, polygons `.17/.21/.18/.22`, textures `.25/.27`, copro data
   `.29/.30`, samples `.32/.34`). Analog platform controls like the deck.
   Little-known, but the cheapest proof that the 2C stack generalises.
   *Found:* it does have an extra board: boot handshakes with a drive
   board over two I/O bytes and stops with "DRIVE BOARD TROUBLE" without
   one (README, "What's known about `skisuprg`"). *Done:* the drive board
   is modelled (`crates/model2/src/drive.rs`) and the controls moved into
   each game's manifest; it plays from coin-up to the results screen.
2. **`waverunr`, Wave Runner.** Same shape: SCSP only (driver 3.2.7),
   analog handlebar and throttle, similar layout with a few extra 512 KB
   polygon/texture/data chips. A possible link port is unconfirmed; it
   should be stubbable.
3. **`hotd`, The House of the Dead.** The best-known 2C game, and same
   driver as `topskatr`. New work: light-gun input (mouse or pointer to the
   gun's position, off-screen reload), and how 2C reads a gun is unknown.
   Larger set (84 MB: data `.5`-`.12`, eight polygon and four texture
   chips), and its copro program sits in a data ROM pair (`.9/.10`) rather
   than the program ROM (ROM), so it can't assume `topskatr`'s boot path.
4. **`segawski`, Sega Water Ski.** Like Ski Super G, with more ROM (eight
   polygon and four texture chips) and a newer driver (3.4.8).
5. **`overrev`, Over Rev.** The smallest 2C set and the closest layout
   (no copro data ROMs at all), but Jaleco's own sound driver, wheel and
   shifter controls, and unknown cabinet hardware. Well liked.
6. **`dynmcopc`, Dynamite Cop (2C).** Simplest controls (digital) and the
   3.2.8 driver, but it's a clone whose data chips were named for the 2A
   board, so which chip goes where on 2C has to come from MAME (gk) and then
   the game.
7. **`stcc`, Sega Touring Car Championship.** Done, with music: it
   needed no new boards to run (its drive board is never checked, the
   link is off by default, nothing waits on the DSB1), but geometrizer
   list jumps and mirrored ROM slots. Music took a Z80 core and a DSB1
   model, worked out from its Z80 program (`docs/hw/dsb1.md`); its MPEG
   data and decoder protocol match the DSB2's closely enough to share
   the decoder model.
8. **`bel`, Behind Enemy Lines.** Obscure, its own sound driver, small
   128 KB program chips laid out differently, unknown controls.

Recommendation: `skisuprg` to shake out what's still `topskatr`-specific
(done; `stcc` done too), then `hotd` as the game most worth having. `hotd` needs a light
gun: a new kind of control in the manifest (`romset::Controls` leaves room
for it).

## Where the evidence is weak

- Original versus 2A is the dat's grouping only; the ROMs show "Model 2 or
  2A" but don't separate them.
- Controls and special hardware are general knowledge for every game but
  `topskatr`. Extra 128 KB chips on sockets that the main board also uses
  (`desert`, `manxtt`, `sgt24h`, `von`, `stcc`) show another board is
  present, not which.
- The dat's years are sometimes late (see above).
- SHARC on 2B and TGP MB86234 on 2/2A come from MAME and public docs, not
  from these ROMs.

## How this was checked

With small scripts on copies outside the repo (nothing unzipped into it):

- Zip directories against the dat's chip lists by CRC, parent and clones.
- The 32 loose zips against the copies inside `m2emu1.1a.zip`, by CRC.
- `topskatr/`'s files against `topskatr.zip`, by CRC.
- `topskatr`'s two uploaded MB86235 programs (`out/topskatr/copro.bin` and
  `geo.bin`, from `remodel run`) cut into 32-byte windows, searched for in
  every chip and every 16-bit-interleaved chip pair of each set. `topskatr`
  matches all of them. Every 2C program (`bel`, `hotd`, `overrev`,
  `skisuprg`, `segawski`, `stcc`, `waverunr`, `dynmcopc`) matches about half
  of the geometrizer windows (17-23 of 37) and some of the copro windows
  (6-22 of 128), so they share the geometrizer library and part of the
  copro code. 2A and 2B sets (`vcop2`, `von`, `schamp`, `gunblade`, and the
  2B and 2A Dynamite Cop programs) match none.
- Sound program chips searched for Sega's driver ID string, both byte
  orders; the two `.2s` chips' first instructions identify their CPUs.
