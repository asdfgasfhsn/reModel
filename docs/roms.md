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
  not run `topskatr`, so its board facts are hypotheses (see `docs/hw/model2c.md` and `docs/architecture.md`).

## What's in the folder

- 32 set zips, each byte-identical to the "Model 2 Romset (Merged)" folder
  inside `m2emu1.1a.zip` (ROM: CRC of every zip compared). They are merged
  sets: a parent's zip also holds its clones' chips.
- `m2emu1.1a.zip` (642 MB) is not a game. It is the same 32 zips again, plus
  the dat, a configurator and ElSemi's Model 2 Emulator 1.1a (a closed-source
  Windows binary with a few Lua scripts). reModel is clean-room: use the dat
  as a catalogue, don't use the emulator. `remodel scan` lists it as a set
  ("m2emu1.1a, not supported yet") because it treats every zip as one.
- `topskatr/` is the 20 chips of `topskatr.zip`'s parent set, unpacked; every
  file's CRC matches the zip (ROM). The zip also holds the clones
  `topskatrj` and `topskatru` (two program chips each, in subfolders).
- Completeness: every set in the dat, parents and clones, is present in full
  in its zip, by CRC; no zip has a chip the dat doesn't list (ROM + dat). The
  dat's sets mostly follow MAME's names, but this was not checked against
  MAME's own list, so a MAME-specific set or chip could still be missing.

## Board generations and what they mean for reModel

Every board has the i960KB main CPU, the 68000 + SCSP sound board (but the
original Model 2: ROM, `docs/hw/model2.md` §5), the
same tilemap/2D layers and the same kind of polygon raster stream, so the
i960 recompiler, ROM loading, the sound board, the GPU renderer and both
players carry over (gk for the shared parts; `docs/architecture.md`'s reuse table says
the same). The geometry hardware is what changes:

| Board | Geometry (gk unless marked) | What reModel would need |
|---|---|---|
| Model 2 (1993-94) | Fujitsu TGP MB86234 (gk). **ROM:** the game uploads its program (`daytona`, `vcop`; each game its own), contrary to the gk that it is internal and undumped. | *`daytona`, `vcop` and `desert` play.* the 2A board model with the original board's differences (`docs/hw/model2.md`); its sound board (two MultiPCMs, an OPN2) is modelled (`docs/hw/multipcm.md`). |
| 2A-CRX (1995) | Same TGP as Model 2 (gk). **ROM:** `srallyc` and `vf2` upload theirs too; no geometrizer program is uploaded (fixed function, unconfirmed). | As Model 2, plus 2A's map and a fixed-function geometrizer. Once one 2A game runs, the others are mostly manifests. |
| 2B-CRX (1995) | Two ADSP-21062 SHARCs (gk for the chip; **ROM** for two): copro and geometrizer, both programmed by the i960 through their I/O-processor DMA. | *Done for `overrevb`, `sgt24h`, `dynabb97` and `doa`:* the SHARC core (interpreter and recompiler), 2B's uploads and port map, and the geometrizer SHARC's output, which has the 2C stream's format (`docs/hw/model2b.md`). Sega's 2B games run their own SHARC programs and may use more of the board. |
| 2C-CRX (1996-98) | Fujitsu MB86235 (TGPx4) copro and geometrizer, both running programs the i960 uploads. **ROM:** every 2C program set contains MB86235 code shared with `topskatr`'s uploaded programs; no 2A or 2B set does. | Mostly a manifest per game. The MB86235 interpreter and recompiler, the geometrizer, the 2C memory map and the sound board are all there. New work is per game: inputs, any extra board, and whatever the game's code uses that `topskatr`'s doesn't. |

Two ROM clues sort the boards (the chips are ROM evidence; what they are is gk): every Model 2 and 2A
set carries the same nine shared chips, `opr-14742a.45` to `opr-14747.63`
(6 x 128 KB) and `mpr-16310.15`, `mpr-16311.16`, `mpr-16312.14`
(3 x 512 KB); no 2B or 2C set does. The 128 KB pairs are maths tables for
the TGP (confirmed numerically: sine, reciprocal and inverse square root,
`docs/hw/model2a.md`); the 512 KB three hold 8-bit lookup tables, probably for the
rasteriser (unconfirmed). The 2B/2C zips that contain them (`doa`,
`dynamcop`, `pltkids`, `zerogun`) only do so because of a 2A clone merged
in.

**Sound.** The 2A, 2B and 2C sets have the 68000 + SCSP board (gk); the
original Model 2's `daytona` has another (ROM: its program drives two
MultiPCM chips and an OPN2 FM chip, `docs/hw/model2.md` §5, modelled as `sound::PcmBoard`). The sound programs of
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
| `daytona` | Daytona USA | 1996 (dat) | `dayton93`, `daytonas`, `daytonase`, `daytonat`, `daytonata`, `daytonagtx`, `daytonam` | **not SCSP** (ROM): 68000 with two MultiPCM-type and an OPN2-type chip (`docs/hw/model2.md` §5) | wheel, pedals, 4-speed shifter, view buttons | drive board (force feedback); link up to 8 | yes (66 files). Several clones are fan hacks (non-Sega chip names such as `gtx.12`, `maxx.12`, `bank0.bin`; gk) |
| `desert` | Desert Tank | 1996 (dat; the title screen says 1994) | none | as `daytona` (ROM, by running it: `epr-16985.7` on `sound::PcmBoard`) | handle, turret, pedal, drive lever, machine gun, cannon, three view buttons (ROM: its input test, `docs/games/desert.md`) | motion cabinet (gk); polygon ROMs paired `.16/.21`, `.17/.20` (ROM) | yes |
| `vcop` | Virtua Cop | 1994 | `vcopa` | as `daytona` (ROM, by running it: `epr-17170.7` on `sound::PcmBoard`; `.32`/`.33` are 1 MB chips in 2 MB sockets, `docs/hw/multipcm.md`) | 2 light guns (ROM: the guns' registers at `0x1C00100`, `docs/hw/model2.md`) | none known | yes |

### Model 2A-CRX

| Set | Title | Year | Clones in zip | Sound | Controls | Special hardware | Complete |
|---|---|---|---|---|---|---|---|
| `manxtt` | Manx TT Superbike (Rev D, Deluxe) | 1995 | none (`manxttc` is its own set) | SCSP (ROM, by running it) | throttle, brake, bank (the lean) on analog channels 0-2, shift up/down, start/view (ROM: its input test, `docs/games/manxtt.md`) | the "Active Motion Slider" platform, checked at boot (ROM: modelled as `DriveBoard::MotionSlider`); 128 KB `epr-18742.7` (a 68000 program) and `mpr-18743.32` on another board, not used; link | yes; *in progress:* attract and coin-start races with sound |
| `manxttc` | Manx TT Superbike (Rev C, Twin) | 1995 | none | SCSP | as `manxtt` | twin cabinet, also with the motion slider (ROM: its boot check); link | yes; a clone of `manxtt` here (`parent`), its shared chips found in `manxtt.zip` |
| `motoraid` | Motor Raid | 1997 | none | SCSP, CRX_drv 3.2.8 | accelerator, brake, bank (the lean) on analog channels 0-2, punch, kick, start/view (ROM: its input test, `docs/games/motoraid.md`) | ride-on cabinet; main data `.6/.7`, `.4/.5` in the second bank at `0x6000000` (ROM) | yes; *in progress:* attract and Championship stages with sound |
| `srallyc` | Sega Rally Championship (revision C) | 1995 | `srallycb`, `srallyp` (prototype-named chips `ic12.bin`, `ic13.bin`) | SCSP | wheel, pedals, shifter | drive board; link | yes; supported and playable, with sound (2A; the game's TGP program on an MB86234 interpreter, its data ROM on the TGP's bus, the 2C native geometrizer's command handling for the fixed-function geometrizer; `docs/hw/model2a.md`) |
| `skytargt` | Sky Target | 1995 | none | SCSP | flight stick with trigger | none known | yes |
| `vcop2` | Virtua Cop 2 | 1995 | none | SCSP | 2 light guns | none known | yes |
| `vf2` | Virtua Fighter 2.1 | 1995 | `vf2o`, `vf2a`, `vf2b` | SCSP | stick, 3 buttons | none | yes |
| `doaa` | Dead or Alive (2A) | 1996 | clone of 2B `doa`, in `doa.zip` | SCSP | stick, 3 buttons | none | yes |
| `dynamcop` | Dynamite Cop (2A) | 1998 | `dyndeka2` (2A), `dynmcopb`, `dyndek2b` (2B), `dynmcopc` (2C) | SCSP, CRX_drv 3.2.8 | stick, punch, kick, jump, two players (ROM: its input test, `docs/games/dynamcop.md`) | a keyed protection device at `0x1D80000`/`0x1D90000`, checked at boot (ROM: answered as `ProtectionInterface::Keyed`); main data `.6/.7`, `.4/.5` in the second bank at `0x6000000` (ROM) | yes; supported and playable, with sound |
| `pltkidsa` | Pilot Kids (2A) | 1998 | clone of 2B `pltkids` | SCSP, CRX_drv 3.4.9 | stick, buttons | none | yes |
| `zeroguna`, `zerogunaj` | Zero Gunner (2A) | 1997 | clones of 2B `zerogun` | SCSP | stick, buttons | none | yes |

### Model 2B-CRX

| Set | Title | Year | Clones in zip | Sound | Controls | Special hardware | Complete |
|---|---|---|---|---|---|---|---|
| `doa` | Dead or Alive | 1996 | `doaa` (2A) | SCSP | stick, 3 buttons (confirmed, `docs/games/doa.md`) | a protection device at `0x1D80000` (confirmed from the code): without it the game stops itself during the first fight; answered by the board model from the manifest (docs/games/doa.md) | yes, but `mpr-19324.19` is a bad dump: data line 9 reads some 1s as 0 (confirmed from the data; repaired at load where the polygon format keeps a copy, `docs/games/doa.md`); supported and playable, with sound (2B; its own copro program, the shared geometrizer code; full speed recompiled) |
| `dynabb97` | Dynamite Baseball 97 (Rev A) | 1997 | none | SCSP, CRX_drv 3.2.8 | per player a stick, two buttons and a bat lever (analog channels 0, 1) (confirmed, `docs/games/dynabb97.md`) | none needed (confirmed) | yes; supported and playable, with sound (2B; its own copro program, the Jaleco games' geometrizer code) |
| `fvipers` | Fighting Vipers | 1995 | none | SCSP | per player a stick and punch, kick, guard (confirmed, `docs/games/fvipers.md`) | none needed (confirmed); copro data `.29/.30` (the same chips as `schamp`'s) on the copro SHARC's bus; texture RAM banks of 1 MB (`docs/games/fvipers.md`) | yes; supported and playable, with sound (2B; `indy500`'s geometrizer code, its own copro program; polygon pairs packed every 4 MB) |
| `gunblade` | Gunblade NY (Rev A) | 1995 | none | SCSP | 2 mounted machine guns: X on analog channels 0 (1P) and 1 (2P), Y on 2 and 3, a trigger each (confirmed, `docs/games/gunblade.md`) | none needed (confirmed); copro data `.29/.30` on the copro SHARC's bus | yes; supported and playable, with sound (2B; SHARC programs of its own, nothing new from the board model) |
| `indy500` | Indianapolis 500 (Rev A, Twin, newer revision) | 1995 | `indy500d` (Deluxe), `indy500to` (older Twin revision): own program, `.7/.8` and sound program chips, not supported | SCSP | wheel (ch. 0), gas (1), brake (2), shift up/down, two view buttons (confirmed, `docs/games/indy500.md`) | copro data ROMs `.29/.30` on the copro SHARC's external bus (confirmed, `docs/games/indy500.md`); a wheel drive board, commanded but its reply never read (modelled); link (stand-alone by default) | yes; supported and playable, with sound (2B; its own copro and geometrizer programs) |
| `lastbrnx` | Last Bronx (Export, Rev A) | 1996 | `lastbrnxj` (Japan, Rev A), `lastbrnxu` (USA, Rev A): their own program pairs `epr-19046a/47a` and `epr-19059a/60a`, not supported | SCSP | per player a stick and punch, kick, guard (confirmed, `docs/games/lastbrnx.md`) | none needed (confirmed); no copro data or `.7/.8` chips: both SHARC programs are in the program ROM (docs/games/lastbrnx.md) | yes; supported and playable, with sound (2B; the shared geometrizer code plus three commands of its own, a copro program of its own; tile RAM and SHARC core findings in `docs/games/lastbrnx.md`) |
| `overrevb` | Over Rev (2B) | 1997 | clone of 2C `overrev` (in `overrev.zip`; only the program pair `epr-19992b/19993b` is its own) | SCSP, Jaleco's driver (confirmed) | as `overrev` (confirmed) | none needed (confirmed) | yes; supported and playable, with sound (docs/games/overrevb.md) |
| `pltkids` | Pilot Kids | 1998 | `pltkidsa` (2A, its own program pair `epr-21281.pr0/.pr1`): not supported | SCSP, CRX_drv 3.4.9 | per player a stick, shot (SW-1) and bomb (SW-2) (confirmed, `docs/games/pltkids.md`) | the keyed protection device, checked once at boot (confirmed, answered as `ProtectionInterface::Keyed`); textures pair `.tx1/.tx0`, `.tx3/.tx2` (odd chip low) | yes (Psikyo chip names: `.pr0`, `.sd0`, `.tp0`...); supported and playable, with sound (2B; `docs/games/pltkids.md`) |
| `rchase2` | Rail Chase 2 | 1995 | none | SCSP | per rider a stick (ADC: 1P X channel 1, Y 3; 2P X 0, Y 2) and a shot button (confirmed, `docs/games/rchase2.md`) | the ride's drive board, linked "by network", answered by a minimal model (`DriveBoard::RideLink`; confirmed from the game's boot wire test); no copro data ROMs; program in two pairs, main data and polygons packed | yes; supported and playable, with sound (2B; its own copro program, the native geometrizer runs its geometrizer) |
| `schamp` | Sonic Championship | 1996 | `sfight` (Sonic The Fighters: own program and `.7/.8` chips; supported, a clone) | SCSP | per player a stick and punch, kick, barrier (confirmed, `docs/games/schamp.md`) | none needed (confirmed); copro data `.29/.30` (the same chips as `fvipers`'s); texture RAM banks of 1 MB (`docs/games/schamp.md`) | yes; supported and playable, with sound (2B; the shared geometrizer code, its own copro program) |
| `sgt24h` | Super GT 24h | 1996 | none | SCSP, Jaleco's own driver (confirmed; the same code as `overrev`'s) | wheel (ch. 0), gas (1), brake (2), shift up/down, view (confirmed, `docs/games/sgt24h.md`) | none needed; `epr-18643a.7` is a Z80 program (Sega 837-10074, most likely the link board's; unconfirmed), and the game runs stand-alone without it (`docs/games/sgt24h.md`) | yes; supported and playable, with sound (2B, on `overrevb`'s SHARC code) |
| `vstriker` | Virtua Striker (Rev A) | 1994 | `vstrikro` (the original revision: the chips without the `a` suffix, not supported) | SCSP | per player a stick and short pass, shoot, long pass (confirmed, `docs/games/vstriker.md`) | none needed (confirmed); no copro data ROMs; texture RAM banks of 1 MB (`docs/games/vstriker.md`) | yes (`ep`/`mp` chip names); supported and playable, with sound (2B; its own geometrizer and copro programs; main data, `.7/.8` included, and polygon pairs packed every 4 MB) |
| `von` | Cyber Troopers Virtual-On (US) | 1996 | `vonj` (Japan, its own program pair `epr-18664b.15/epr-18665b.16`): not supported | SCSP (no driver version string) | two digital 8-way sticks, each with shot and dash, one start (confirmed, `docs/games/von.md`) | a communication (link) board, checked at every boot (confirmed from the code): without one the game starts nothing; answered by the board model from the manifest (`romset::LinkBoard::Idle`, stand-alone only); `epr-18643a.7` is the board's Z80 program, never read by the i960; copro data `.29/.30` on the copro SHARC's bus | yes (US program chips named `vo-prog0.usa`/`vo-prog1.usa`); supported and playable, with sound (2B; `indy500`'s geometrizer code, its own copro program; `docs/games/von.md`) |
| `zerogun` | Zero Gunner | 1997 | `zerogunj` (2B, its own program pair `epr-20290/20291`), `zeroguna`, `zerogunaj` (2A): not supported | SCSP, CRX_drv 3.4.9 | per player a stick and two buttons, shot and bomb (confirmed, `docs/games/zerogun.md`) | a keyed protection device at `0x1D80000`/`0x1D90000`, checked in play (confirmed from the code); answered by the board model from the manifest (`docs/games/zerogun.md`) | yes; supported and playable, with sound (2B; SHARC programs of its own, its geometrizer on the SHARC only; needed the i960 to keep its interrupt table across a cleared PRCB) |
| `dynmcopb`, `dyndek2b` | Dynamite Cop / Deka 2 (2B) | 1998 | clones of 2A `dynamcop` | SCSP | stick, 3 buttons | none | yes |

### Model 2C-CRX

| Set | Title | Year | Clones in zip | Sound | Controls | Special hardware | Complete |
|---|---|---|---|---|---|---|---|
| `topskatr` | Top Skater | 1997 | `topskatrj`, `topskatru` | SCSP, CRX_drv 3.2.8; **DSB2** | skateboard deck (2 analog channels), jump buttons (confirmed, `docs/games/topskatr.md`) | none | yes; also unpacked in `topskatr/` |
| `skisuprg` | Sega Ski Super G | 1996 | none | SCSP, CRX_drv 3.2.8 | ski platform (analog channels 0-2: two ski angles, a third unconfirmed), three panel buttons (confirmed, `docs/games/skisuprg.md`) | drive board (confirmed: boot checks it; modelled, `docs/games/skisuprg.md`) | yes; supported and playable |
| `segawski` | Sega Water Ski (Japan) | 1997 | none | SCSP, CRX_drv 3.4.8 (confirmed) | slide (ch. 0), pitch left/right switches, select up/down and set buttons (confirmed, `docs/games/segawski.md`) | none needed (confirmed); a second main data bank at `0x6000000` (`docs/games/segawski.md`) | yes; supported and playable |
| `waverunr` | Wave Runner | 1996 | none | SCSP, CRX_drv 3.2.7 (confirmed) | handlebar (ch. 0), throttle (2), body roll (1) and pitch (3), start, view (confirmed, `docs/games/waverunr.md`) | ride-on cabinet board: optional, skipped when absent (`docs/games/waverunr.md`); link not read by default | yes; supported and playable |
| `hotd` | The House of the Dead | 1996 (title screen; the dat says 1997) | none | SCSP, CRX_drv 3.2.8 | 2 light guns, read through a gun interface on I/O registers 9-13 (confirmed, `docs/games/hotd.md`) | none needed beyond the gun interface (modelled); a second main data bank at `0x6000000` (`docs/games/hotd.md`) | yes; supported and playable |
| `stcc` | Sega Touring Car Championship | 1996 | `stcca`, `stccb` (the zip holds all three program pairs; reModel uses `epr-19570/19571`) | SCSP, CRX_drv 3.2.5 (confirmed); **DSB1 (Z80)**, modelled (`docs/hw/dsb1.md`) | wheel (ch. 0), gas (1), brake (2), shift up/down buttons, two view buttons (confirmed, `docs/games/stcc.md`) | wheel drive board (commanded every frame, reply never checked; modelled, `docs/games/stcc.md`); link (skipped by the default settings); `epr-18643a.7` (Z80 code for another board) | yes; supported and playable, with music |
| `overrev` | Over Rev | 1997 | `overrevb` (2B; the zip holds both program pairs: `overrev` uses the 2C `epr-20124a/20125a`, `overrevb` the 2B `epr-19992b/19993b`, also supported) | SCSP, Jaleco's own driver (confirmed; streams its music, no music board) | wheel (ch. 0), gas (1), brake (2), shift up/down, view (confirmed, `docs/games/overrev.md`) | none needed; the link board check reads it as absent and runs stand-alone (`docs/games/overrev.md`) | yes; supported and playable, with sound |
| `bel` | Behind Enemy Lines | 1997 (title screen; the dat says 1998) (Sega / EPL) | none | SCSP, EPL's driver (confirmed; much of its code shared with Jaleco's, `docs/hw/sound.md` §7) | two mounted guns on ADC channels 0-3 (yaw, pitch), triggers and missile buttons (confirmed, `docs/games/bel.md`) | none; needs packed main data and polygon banks and the UART's transmit interrupt (`docs/games/bel.md`) | yes; supported and playable, with sound |
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
   one (docs/games/skisuprg.md). *Done:* the drive board
   is modelled (`crates/model2/src/drive.rs`) and the controls moved into
   each game's manifest; it plays from coin-up to the results screen.
2. **`waverunr`, Wave Runner.** Same shape: SCSP only (driver 3.2.7),
   analog handlebar and throttle, similar layout with a few extra 512 KB
   polygon/texture/data chips. *Done:* it needed no board model (its
   cabinet board is skipped when absent, the link isn't read), but its
   `.7/.8` data pair is read through a window at `0x06000000`, and it runs
   routines copied to RAM, which the recompiler now leaves to the
   interpreter (docs/games/waverunr.md).
3. **`hotd`, The House of the Dead.** The best-known 2C game, and same
   driver as `topskatr`. New work: light-gun input (mouse or pointer to the
   gun's position, off-screen reload), and how 2C reads a gun is unknown.
   Larger set (84 MB: data `.5`-`.12`, eight polygon and four texture
   chips), and its copro program sits in a data ROM pair (`.9/.10`) rather
   than the program ROM (ROM), so it can't assume `topskatr`'s boot path.
   *Done:* the boot path was the same code reading another address; the
   new work was the gun interface (I/O registers 9-13, a command and
   counter protocol worked out from the code) and generic light-gun
   controls; it plays (docs/games/hotd.md).
4. **`segawski`, Sega Water Ski.** Like Ski Super G, with more ROM (eight
   polygon and four texture chips) and a newer driver (3.4.8). *Done:* a
   different game from `skisuprg`, not a release of it; it needed only a
   second main data bank (`.7/.8`, `.5/.6` at `0x6000000`) and plays.
5. **`overrev`, Over Rev.** The smallest 2C set and the closest layout
   (no copro data ROMs at all), but Jaleco's own sound driver, wheel and
   shifter controls, and unknown cabinet hardware. Well liked.
   *Done:* it needed no board model: its every-boot network check takes
   the link board's registers reading all ones as "no board" and goes on
   stand-alone. Jaleco's sound driver runs on the SCSP board model
   unchanged (docs/games/overrev.md; `docs/hw/sound.md`
   §6).
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
   *Done:* its main data and polygon pairs are packed every 4 MB, it
   uploads its copro program only when the copro status says it has none,
   and it sends sound commands from the UART's transmit interrupt; the
   guns are analog, the driver needed nothing new. It plays (`docs/games/bel.md`).

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
  present, not which. The one `sgt24h`, `stcc` and `von` share,
  `epr-18643a.7`, is a Z80 program headed Sega 837-10074, most likely the
  link board's (docs/games/sgt24h.md).
- The dat's years are sometimes late (see above).
- That the 2B chips are SHARCs and the 2/2A copro an MB86234 comes from
  public docs (the 2B upload writes 2106x I/O-processor registers, which
  fits); that both boards' programs are uploaded is from the ROMs.

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
