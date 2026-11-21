# ROM variants: a survey of every clone of the supported games

The sets table in [roms.md](roms.md) lists which clones each zip holds; three
are supported today as hand-written clone manifests (`manxttc`, `sfight`,
`overrevb`). This page surveys the other 32 clones, of 14 of the 31
supported titles (14 have no clone in the dat and 3 have only the clones
already supported): what each is, which
chips it changes, whether it runs on its parent's manifest with those chips
swapped, and what per-game data stops fitting. It ends with a recommendation
for supporting all of them as sub-versions of a title. The survey's runs used
temporary code that was reverted; what was built from it is under
[Status](#status-what-is-built), which comes first, and the survey's own
tables and recommendation follow unchanged.

Nothing here quotes ROM contents. Set names, chip names and sizes are the
dat's (a catalogue only; see [roms.md](roms.md)); the run results
were observed by running the chips from `rom/`. Findings not confirmed by a run
or the chips are marked *unconfirmed*.

## Status: what is built

Steps 1 and 2 of the [order](#order) are built: the infrastructure, the three
earlier clones re-expressed as variants, and the nine class A variants.

**The model** (`crates/romset/src/variants.rs`, data in
`crates/romset/src/games/variants.rs`): a `Variant` is a thin manifest that
inherits from its parent title's `Game`, resolved once at first use into a
full `&'static Game` (`romset::games::find(id)`, cached, so downstream code
is unchanged and `game.id` is the variant's own id, `game.parent` the
title's):

| Field | Meaning |
|---|---|
| `id`, `parent`, `label`, `kind` | the set id (what `--game` takes); the title's set; the Version list's name ("Rev B", "Japan"); `VariantKind` = Revision, Region, CrossBoard, Prototype or Hack (the last two hidden by the launcher unless "Show hacks and prototypes"); `kind_confirmed` is false where the kind rests on the dat's name and the chip names only |
| `swaps: &[ChipSwap]`, `drop` | the parent's chip (by name, so by socket and role) replaced by the variant's own (name, size, SHA-1, role), and parent chips the variant lacks (`manxttc`) |
| `title`, `year`, `board`, `packed`, `attract_frame`, `idle_loops`, `backup_ram`, `volume_db`, `texture_ram_mirrored`, `shading`, `recommended`, `input_lag`, `run_ahead_max` | optional overrides; `None` inherits |
| `protection_rom` | the protection answers' ROM addresses (reply index, address), the rest of the protection inherited (not used yet: `dyndeka2`, `doaa`) |
| `scenes` | the variant's own; **never the parent's**, and `games::tours` no longer falls back to the parent's either |
| `link: VariantLink` | `Parent` (its settings record is the parent's: `srallycb`, `srallyp`, `daytonat`, `daytonata`, `manxttc`), `None`, or `Own(&LinkPlay)` |

Chips are looked up as `<variant id>/<chip>` before the bare name in each of
the variant's folder and zip and the parent's (`topskatr.zip` keeps
`topskatrj/epr-19753.15` and `topskatru/epr-19753.15`; `RomSet::load`, and
`Zip` now matches a name with a folder against the stored path). A variant
whose swapped chips are not all there is not listed (`romset::sets_with_clones`,
`variant_available`). `remodel games` lists the titles and then the variants.

**Saves and settings**: EEPROM (`out/<set>/eeprom.bin`), save states, the
resume state, the player's per-game settings and bindings, JIT hot lists and
link settings are keyed by the set id, so a variant has its own and never
touches its parent's. The save file header (format 3) records the set it was
saved from and a load by another variant is refused naming both. Texture packs
fall back to the parent's (`pack_sets`). *Not done:* a variant starts with no
settings or bindings of its own (it does not read its parent's), so a remap
made for `topskatr` is made again for `topskatrj`.

**The launcher** has one card per title and a Version selector (docs/play.md,
"Versions"); `--game`, `make GAME=` and the CLI take a variant id anywhere a set
id goes.

**Network and link play**: the ROM hash in the session agreement is over every
chip and the set id (protocol 4), so two variants never link; a variant links
as its parent where `VariantLink::Parent`.

**Bench**: `make bench-variants` runs `tools/expected-variant-hashes.txt`
(blank-EEPROM coin-start, 600M instructions, as `bench-all`); the three
earlier clones keep their lines in `tools/expected-hashes.txt`, and their
hashes did not change when they became variants (`manxttc`
`d6558827f751bbb0`, `sfight` `2a0a5a5d0d957997`) nor did `overrevb`'s
(`de800c1e32ddb8aa`, measured before and after).

| Variant | Kind | Chips swapped | State hash (600M, coin-start) | Status |
|---|---|---|---|---|
| `manxttc`, `sfight`, `overrevb` | revision, region, other board | as before | unchanged | the three clones, now variants |
| `topskatrj` Japan | region | program pair | `d5b806cefa994cbe` | boots, attracts, coin-start reaches the race; its attract opens on the Japan warning (the card's `attract_frame` is the parent's: not tuned) |
| `topskatru` USA | region | program pair | `de6f5171b2730155` | as `topskatrj`, FBI warning |
| `vcopa` Rev A | revision | program pair | `5c206708efb1d3c8` | plays, the same stage |
| `srallycb` Rev B | revision | program pair | `2d9857e7d3dacb29` | plays; link as the parent |
| `srallyp` "Pro Drivin'" | prototype (*unconfirmed*) | program pair | `ef2b8b45eccd5732` | plays; hidden unless "Show hacks and prototypes" |
| `daytonat` Turbo | hack (*unconfirmed*) | program pair | `0c64f85a8b95e340` | plays; hidden by default; link as the parent |
| `daytonata` Turbo Rev A | hack (*unconfirmed*) | program pair | `95da2bac42132d58` | as `daytonat` |
| `lastbrnxj` Japan, Rev A | region | program pair | `af452e8e6839ef0a` | plays |
| `lastbrnxu` USA, Rev A | region | program pair | `5db58c2199e50ec7` | plays |

These hashes are the survey's own, measured again on the committed
implementation: they agree (the survey's temporary patch and the resolver
build the same machine). "Plays" is as the survey defines it: boots, attracts,
coin-start reaches play, frames look like the game; nobody played them by hand.
The hacks' and the prototype's *kind* is the dat's and the chip names', not
documented. The Turbo hacks run on the parent's idle loops and settings
record, so their link play is the parent's *by the survey's checks only*.

## Method

1. **Catalogue.** The clrmamepro dat inside `m2emu1.1a.zip` gives each
   clone's title, year, parent and own chips (a merged set lists only the
   chips that differ from its parent). The dat was read as text; nothing
   else from that archive was used. Kind (official revision, region,
   prototype, hack) is from the dat's description, the chip names and the
   screens; it is not authoritative.
2. **Chips.** For each clone, its own chips were compared with the parent
   chip in the same socket (the `.NN` number in the name) by size and by
   byte equality of the unpacked chips (the percentages below are the share
   of bytes equal at the same offset).
3. **Run.** A temporary, uncommitted patch let `remodel` take the parent's
   manifest with chips swapped (`REMODEL_SWAP=parentchip=clonechip,...`,
   clone chips looked up in the parent's zip, member paths such as
   `topskatrj/epr-19753.15` for same-named chips in subfolders, size checks
   off) and, for one experiment, shift the protection answers' ROM
   addresses (`REMODEL_PROT_SHIFT`). Each variant ran 600M instructions
   from a blank EEPROM (`--fresh-eeprom`) with the parent's `coin-start`
   script, one snapshot every 600 frames, on the interpreter and the native
   geometrizer, like `make bench`. The parents' own state hashes matched
   `tools/expected-hashes.txt` in the same session, so the harness did not
   disturb the runs. The patch was not kept; the Recommendation says what a
   committed form would be.
4. **Cheap signals.** Whether the parent's idle-loop address still holds
   (the `idle skip` line: 0 skipped instructions means the loop is
   elsewhere, and the loop's bytes were searched for in the clone's
   program); whether the parent's protection answers (`Protection.replies`,
   ROM addresses) are at the same place (bytes searched in the clone);
   whether the settings record the clone writes has the parent's size,
   checksum and offsets (EEPROM saved by a run without `--fresh-eeprom`,
   checksums recomputed with the manifest's `RecordCrc` rules); and the
   snapshots (title, region text, whether it reaches a race or a fight).
5. **Cross-board clones** (a 2A clone of a 2B game and the like) need a
   manifest for the other board. For these a throwaway manifest was
   generated from the parent's with the board changed and its board-specific
   chips swapped (the six `opr-*` and clone program chips for 2A; the
   program pair only for 2B and 2C). The chip roles and packing were guessed
   from the parent and sibling 2A manifests, so a failure there says little
   about the clone, and a success says a lot.

What a "run" can and cannot show: 600M instructions reach the first race,
fight or stage in most games (frame 4141). Nothing was played by hand, no
sound was checked and the per-variant gameplay differences are not looked
for. "Plays" below means: boots, attracts, coin-start reaches play and
frames look like the game.

## The clones already supported (the reference)

What the three hand-written clones needed beyond a chip swap shows what
"same as parent except chips" means in practice:

| Clone | Parent | Chips | Own data in its manifest |
|---|---|---|---|
| `manxttc` | `manxtt` | program (4), sound program, one sample ROM; lacks two chips | title, its own idle loop address, attract frame; same controls, drive board and link record |
| `sfight` | `schamp` | program pair and the `.7/.8` pair | title, idle loop (0x2C lower), attract frame, shading and recommended |
| `overrevb` | `overrev` | program pair; **another board** (2B for 2C) | board, idle loop, attract frame, recommended |

## Variants by title

Legend: **kind** is official revision, region, prototype, hack or
cross-board; **chips** are the clone's own chips by parent socket; the
**state hash** is that of the 600M-instruction coin-start run from a blank
EEPROM with the swap (the hash a `make bench` entry would hold; for the
2A clones of 2B games, a 400M run); **effort** is A (same as the parent
except chips), B (chips plus a few addresses or bytes) or C (needs
investigation).

### Top Skater (`topskatr`, 2C): 2 clones

| Variant | Kind | Chips that differ | Run | Per-game data that differs | Effort |
|---|---|---|---|---|---|
| `topskatrj` Top Skater (Jap) | region, official | program pair `epr-19753.15/.16` (68% of bytes equal to the export program) | plays: Japan warning, Japanese text, Easy/Hard courses, race; `d5b806cefa994cbe`; idle skip 82% | idle loop same address (bytes identical); attract: warning screen first, so `attract_frame` 1650 shows something else; scenes (frames 1200, 2100) differ by the warning | A |
| `topskatru` Top Skater (USA) | region, official | program pair `epr-19753.15/.16` (92%); the same member names as the Japan pair, in subfolders `topskatrj/` and `topskatru/` of the zip | plays: FBI warning, then as the parent; `de6f5171b2730155`; idle skip 82% | idle loop same address; frames line up with the parent from 1200 | A |

### Sega Touring Car Championship (`stcc`, 2C): 2 clones

| Variant | Kind | Chips that differ | Run | Per-game data that differs | Effort |
|---|---|---|---|---|---|
| `stcca` Rev A | revision, official | program pair `epr-19272a.15/19273a.16` in the parent's `.15/.16` sockets (44% equal) | plays: car select, name entry, qualifying start; `969ed016b6b232c8`; **idle skip 0%** | idle loop moved 0x763D4 to 0x73424 (unique byte match, the run's last pc is inside it); attract frame; DSB1 and sound chips are the parent's | B |
| `stccb` Rev B | revision, official | `epr-19272b.15/19273b.16` (44%) | plays as `stcca`; `a50918a0e5333ac8`; idle skip 0% | idle loop moved to 0x73574 (same check) | B |

### Virtua Fighter 2 (`vf2`, 2A): 3 clones

| Variant | Kind | Chips that differ | Run | Per-game data that differs | Effort |
|---|---|---|---|---|---|
| `vf2o` Virtua Fighter 2 (original) | revision, official | four program chips `epr-17568.12`, `17569.13`, `17562.14`, `17563.15` (16% equal) | plays: attract, character select, fights; `be420a1f95e7e0c6`; idle skip 0% | idle loop 0x10F98 to 0x10A74; **tours and scenes are frame counts and the fights go differently (ring out at 2400 only in the parent)**; `hud_element_windows [1]` not checked | B |
| `vf2a` Rev A | revision, official | `epr-17568a.ic12` ... `17563a.ic15` (socket names `ic12`..`ic15`) | plays as `vf2o`; `aa8922832829ebc6` | idle loop to 0x10A98; the same | B |
| `vf2b` Rev B | revision, official | `epr-17568b.12` ... `17563b.15` | plays; `6790a5d07fdfd8f7`; idle skip 0% | idle loop to 0x10B3C (the run's last pc is elsewhere: unconfirmed); the same | B |

### Virtua Cop (`vcop`, Model 2): 1 clone

| Variant | Kind | Chips that differ | Run | Per-game data that differs | Effort |
|---|---|---|---|---|---|
| `vcopa` Rev A | revision, official | program pair `epr-17166a.12/17167a.13` (99.8% equal to the parent's `b` pair) | plays, the same stage and shots; `5c206708efb1d3c8`; idle skip 76% | none found: idle loop same address, the guns' controls as the parent | A |

### Sega Rally Championship (`srallyc`, 2A): 2 clones

| Variant | Kind | Chips that differ | Run | Per-game data that differs | Effort |
|---|---|---|---|---|---|
| `srallycb` Rev B | revision, official | program pair `epr-17888b.12/17889b.13` (99.99% equal) | plays; `2d9857e7d3dacb29`; idle skip 85.6%, the same count as the parent | none: the idle loop is code copied to RAM, the same address works; **the default settings record is byte-identical to the parent's (checksum valid), so the `LinkPlay` entry fits** | A |
| `srallyp` "Pro Drivin" | prototype | `ic12.bin`, `ic13.bin` (99.9%) | plays; different car liveries and course scenery (a prototype); `ef2b8b45eccd5732`; idle skip 86.4% | none found; same record | A |

### Daytona USA (`daytona`, Model 2): 7 clones

| Variant | Kind | Chips that differ | Run | Per-game data that differs | Effort |
|---|---|---|---|---|---|
| `dayton93` '93 Edition | revision, official | ten: program pair, main data `.8/.9` (2 MB, 62% equal) and `.6/.7` (1 MB against 512 KB), polygons `.19/.23` (512 KB against 2 MB), textures `.26/.27` (about 40% equal) | plays: course select, race with textures; `2a4707a921d968df`; idle skip 0% | idle loop 1 moved 0x1394 to 0x12B0; loop 2 (0x1B8B8) not found in the program; sizes differ from the manifest's, so its own file list is needed; it has its own textures (a texture pack from the parent would miss) | B |
| `daytonas` Saturn Ads | official variant (adverts for the Saturn) | program pair and `.6/.7` (`epr-17965.12/17966.13`, `epr17967.6/17968.7`) | boots to a settings page, then "NETWORK BOARD NOT PRESENT, CANCELLED"; never reaches the game; `a9af952cc874cda4` | **the settings record the manifest presets (`backup_ram`, "SEGA" header, version, CRC-16) is not this program's**: it shows its own defaults (LINK ID MASTER, an extra "PROMOTE SATURN" item) and stops on the missing link board; both idle loops moved (0x13A8, 0x1BDA8); record, version bytes and CRC for this program needed | B |
| `daytonase` Special Edition | official | program pair and `.6/.7` (`epr-17369a.12/17370a.13`, `epr-13731.6/13732.7`) | as `daytonas`; `a56d1b0ce1c4e922` | the same: own record (COUNTRY JPN, CABINET SPECIAL); idle loops 0x13A8, 0x1BD38 | B |
| `daytonat` Turbo | hack (chips `turbo1.12/turbo2.13`) | program pair (99.7% equal) | plays, a black car; `0c64f85a8b95e340`; idle skip 74% | none: idle loops and the record fit | A |
| `daytonata` Turbo Rev A | hack (`dayturbo.12/.13`) | program pair (99.7%) | plays as `daytonat`; `95da2bac42132d58` | none | A |
| `daytonagtx` GTX2004 | hack (`gtx.12/.13`, `bank0.bin`, `bank1.bin`) | program pair (98%) and `.6/.7` (1-3% equal: not the same kind of data in those sockets, or not interleaved the same) | **black screen, the CPU jumps to garbage (`0xAAAABBBF`) in the third frame**; `da8656a96454146f` | unknown: what `bank0.bin`/`bank1.bin` are | C |
| `daytonam` To The Maxx | hack (`maxx.12/.13/.14`) | program pair (97%) plus a third chip, `maxx.14` (128 KB, mostly zeros), that has no socket in the parent | boots to the settings page and stays there; the preset record looks accepted (LINK ID SINGLE, CABINET TWIN: unconfirmed); `5a4df49d77143b26` | unknown role of `maxx.14` (the third chip was left out of the run); idle loops same | C |

`daytona` is the only parent whose manifest presets its settings
(`backup_ram`, the I/O board's store), and `daytonas`/`daytonase` are the
evidence that a clone can reject the parent's preset record.

### Indianapolis 500 (`indy500`, 2B): 2 clones

| Variant | Kind | Chips that differ | Run | Per-game data that differs | Effort |
|---|---|---|---|---|---|
| `indy500d` Deluxe | revision (Japan warning screen), official | program pair `epr-18251a.15/18252a.16` (20% equal), `.7/.8` pair `epr-18345/18346` (about 33%), sound program `epr-18253.31` (94%) | plays: Japan warning, race; `db57fdba291b8487`; idle skip 0% | idle loop 0x34914 to 0x345D4; **the settings record is another size and layout (checksum over the parent's range fails, no copy at 0x2C, role and cabinet bytes at 0x1E/0x1F hold other values: 18, 17)**, defaults differ (the race showed credit 2/3); needs its own `LinkPlay` or none | B |
| `indy500to` Twin (older) | revision, official | program pair `epr-18254a.15/18255a.16` (19%), `.7/.8` `epr-18389/18390`, sound program `epr-18391.31` (98%) | boots, attract in Japanese, **the `coin-start` script's start press does not start a race by frame 3600** (still on "PRESS START BUTTON"); `ed4ac127b1c34bff`; idle skip 0% | idle loop to 0x2E1C4; the record has the parent's S32A checksum and seed (valid) but fewer settings, so offsets such as the role byte need re-finding; input timing | B |

### Last Bronx (`lastbrnx`, 2B): 2 clones

| Variant | Kind | Chips that differ | Run | Per-game data that differs | Effort |
|---|---|---|---|---|---|
| `lastbrnxj` Japan Rev A | region, official | program pair `epr-19046a.15/19047a.16` (46% equal) | plays: logo, fights; `af452e8e6839ef0a`; idle skip 78% | idle loop same address; the intro is a few hundred frames different (a SEGA logo at frame 1200 where the parent shows a fight), so `attract_frame` 700 | A |
| `lastbrnxu` USA Rev A | region, official | program pair `epr-19059a.15/19060a.16` (45%) | plays; `5db58c2199e50ec7`; idle skip 76% | idle loop hit at 0x7400 (the same loop, the run's last pc) | A |

### Virtua Striker (`vstriker`, 2B): 1 clone

| Variant | Kind | Chips that differ | Run | Per-game data that differs | Effort |
|---|---|---|---|---|---|
| `vstrikro` original revision | revision, official | four program chips `ep18068.15` ... `ep18067.14` (9% equal) and the `.7/.8` pair `ep18070/18071` (95% equal), the parent's with an `a` suffix | plays: ranking, a match; `940ba002561c05cb`; idle skip 0% | idle loop 0x3D54 to 0x3DD4 | B |

### Cyber Troopers Virtual-On (`von`, 2B): 1 clone

| Variant | Kind | Chips that differ | Run | Per-game data that differs | Effort |
|---|---|---|---|---|---|
| `vonj` Japan | region, official | program pair `epr-18664b.15/18665b.16` for the parent's `vo-prog0.usa/vo-prog1.usa` (59% equal); `.13/.14` shared | plays, the frames match the parent's; `c3bf3413fbdd155b`; idle skip 0% | idle loop 0x29A3C to 0x28E2C | B |

### Zero Gunner (`zerogun`, 2B): 3 clones

| Variant | Kind | Chips that differ | Run | Per-game data that differs | Effort |
|---|---|---|---|---|---|
| `zerogunj` Japan, 2B | region, official | program pair `epr-20290.15/20291.16` (61%) | plays: Japanese title, mission 1, continue, credits; `171d4f54c4caa140`; idle skip 0% | **protection answers at the same ROM addresses (all 16 matched by byte search)**; the idle loop is code copied to RAM: the manifest's address (0x549050) is probed 3,694 times and refused every time, so the loop is elsewhere (not found; no way to search RAM-run code in the ROM) | B |
| `zeroguna` 2A | cross-board | program pair `epr-20437.12/20438.13` and the six 2A `opr-*` chips | generated 2A manifest: title and the game's HUD draw, **the 3D layer is black in play**; the same with the data banks packed as the 2A manifests do (state hash unchanged) | protection answers at the same addresses; the cause of the black 3D unknown | C |
| `zerogunaj` Japan, 2A | cross-board | `epr-20288.12/20289.13` and the `opr-*` chips | as `zeroguna` | as above | C |

### Dynamite Cop (`dynamcop`, 2A): 4 clones

| Variant | Kind | Chips that differ | Run | Per-game data that differs | Effort |
|---|---|---|---|---|---|
| `dyndeka2` Dynamite Deka 2, 2A | region (Japan), official | program `.12/.13` `epr-20922/20923` (82% equal; `.14/.15` shared) | on the parent's manifest: "SCROLL ALLOCATION ERROR!! 00510CE0 19", black screen. **With the protection answers' ROM addresses shifted by -0x10 it plays** (attract in 3D, a fight); `a3b912a344e41ee1` | protection replies shifted -0x10 (both found by byte search); idle loop 0x71194 to 0x71184 | B |
| `dynmcopb` 2B | cross-board | program pair `progrom0.15/progrom1.16` | generated 2B manifest: title "Dynamite" at credit 0, then an i960 fault at address 0 after 40M instructions | the 2B chip roles and layout unknown | C |
| `dyndek2b` Deka 2, 2B | cross-board | `epr-20936.15/20937.16` | generated 2B manifest: runs all frames, black screen with a stray "2" | as `dynmcopb` | C |
| `dynmcopc` 2C | cross-board | `epr-20952.15/20953.16` | generated 2C manifest: runs, stays on the parental advisory screen; both MB86235 programs uploaded | the 2C layout unknown | C |

### Dead or Alive (`doa`, 2B): 1 clone

| Variant | Kind | Chips that differ | Run | Per-game data that differs | Effort |
|---|---|---|---|---|---|
| `doaa` 2A | cross-board | program pair `e19310aa.12/e19311aa.13` (512 KB each), and the six 2A `opr-*` chips as in every 2A set; data, polygons, textures and sound are the parent's | generated 2A manifest (the parent's data in the parent's roles, Role::CoproData for the `opr-*`): **plays**: Tecmo title, Kasumi against Zack, a winner screen; `3cdae73e2525aa8b` (400M) **but only after the protection check's reply address was shifted +0x20** (without, the 3D fighters never draw and the fight freezes) | the protection (`Tables`) reply at 0x2810 is at 0x2830; board changed to 2A; idle loop (0x27E0) gave 0 skips: not re-found | B (cross-board) |

### Pilot Kids (`pltkids`, 2B): 1 clone

| Variant | Kind | Chips that differ | Run | Per-game data that differs | Effort |
|---|---|---|---|---|---|
| `pltkidsa` 2A | cross-board | `epr-21281.pr0/21282.pr1` and the `opr-*` chips | generated 2A manifest: an attract scene and the title draw (frame 1200), **then it stops making progress** (the run was killed after 13 minutes of CPU, frames 600 and 1200 only; twice) | protection answer at the same address (0x518C8); cause of the stall unknown | C |

### Titles with no clones in the dat

`desert`, `vcop2`, `skytargt`, `motoraid`, `gunblade`, `fvipers`, `rchase2`,
`sgt24h`, `dynabb97`, `skisuprg`, `segawski`, `waverunr`, `hotd`, `bel` have
none; `schamp`, `overrev` and `manxtt` have only the clones already
supported (`manxttc` is a parent in the dat, not a clone).

## Totals

32 clones of 14 supported titles, plus the 3 already supported.

| Effort | Count | Variants |
|---|---|---|
| A: same as the parent except chips | 9 | `topskatrj`, `topskatru`, `vcopa`, `srallycb`, `srallyp`, `daytonat`, `daytonata`, `lastbrnxj`, `lastbrnxu` |
| B: chips plus a few addresses or bytes | 15 | `stcca`, `stccb`, `vf2o`, `vf2a`, `vf2b`, `vstrikro`, `vonj`, `indy500d`, `indy500to`, `dayton93`, `daytonas`, `daytonase`, `zerogunj`, `dyndeka2`, `doaa` |
| C: needs investigation | 8 | `daytonagtx`, `daytonam`, `zeroguna`, `zerogunaj`, `pltkidsa`, `dynmcopb`, `dyndek2b`, `dynmcopc` |

By kind: 13 revisions or official variants (two of them Daytona's
Saturn-advertising and Special Edition sets), 7 regions, 1 prototype, 4 hacks
(all Daytona), 7 cross-board (the board is not the parent's). By chips: in 24 only the program chips differ (plus the 2A
`opr-*` set for the 2A ones); in 8 (`dayton93`, `daytonas`, `daytonase`,
`daytonagtx`, `daytonam`, `indy500d`, `indy500to`, `vstrikro`) a data or
sound chip differs too.

What differed from the parent in practice, over the 25 same-board variants
that ran (the 7 cross-board ones ran on generated manifests):

| Per-game data | How often it differed |
|---|---|
| idle loop address | 14 of 25 moved or no longer fit (the others have it where it was); in 13 the new address was found by byte search of the program (`dayton93`'s second loop not), in `zerogunj` it is RAM-run code and was not found |
| attract frame, scenes, tours (frame counts) | whenever the program differs: intros, warning screens, rev timings (`topskatrj`, `lastbrnxj`, `vf2*`) |
| protection answers' ROM addresses | of the six clones of protected games, `dyndeka2` -0x10 and `doaa` +0x20; `zerogunj`, `zeroguna`, `zerogunaj` and `pltkidsa` the same address |
| settings record | `daytonas`, `daytonase` (preset record rejected), `indy500d` (other layout), `indy500to` (other layout, same checksum); `srallycb`/`srallyp` identical |
| input script timing | `indy500to` (the start press did not take) |
| controls, drive board, hud windows, shading | nothing differed where checked; `hud_windows` and `hud_element_windows` were not verified for `vf2*` or `topskatr*` |

## Recommendation

### Modelling

Make a variant a **thin, inheriting manifest over the parent's `Game`**, and
resolve it once at start-up into a full `&'static Game`, so everything
downstream (the board model, the player, `romset::pack_sets`, the CLI) keeps
using `Game` unchanged. The three hand-written clones already are this by
copy-and-edit; the survey says what the thin form is:

- identity: `id`, `parent`, a short `label` ("Rev A", "Japan", "USA",
  "Deluxe", "Prototype", "2A board"), `kind` (revision, region, prototype,
  cross-board, hack), `year`, and a title override where the game's own title
  differs;
- **chip swaps**: a list of (parent chip, variant chip) by socket, where the
  variant chip may be `subfolder/name`. The lookup should try `<variant
  id>/<chip>` in the zip before the bare name (how a merged set keeps
  same-named chips of different clones, `topskatrj/` and `topskatru/`), and a
  chip's size and SHA-1 come from the variant's own entry (a size can differ
  from the parent's, `dayton93`). For a cross-board variant the file list is
  replaced, not swapped (its roles and `packed` differ);
- **overrides, as optional fields**, because these are what differed in
  practice: `idle_loops` (almost always), `attract_frame`, the protection
  replies' ROM addresses (`Protection::replies[*].rom`: keep the commands and
  offsets, override the addresses), `backup_ram` (Daytona's preset record),
  the `LinkPlay` record and free-play bytes (looked up by variant id, with a
  fallback to the parent's only where the record was checked equal, as
  `srallycb`), `board` and `packed` for cross-board, and the sound
  `volume_db` where the sound program is another;
- **everything else inherited**: controls, drive and link boards, input lag,
  shading, `recommended`, dropouts, mirroring. Nothing contradicted
  inheriting them in the 25 runs, but `hud_windows` and
  `hud_element_windows` should be checked per variant before being trusted.
- **scenes and tours are not inherited**. They are frame counts from power-on
  and the programs' timing differs (`games::tours` falls back to the parent's
  tours for a clone today: only `topskatr`, `vf2` and `daytona` have tours,
  so their 12 clones inherit them, and the frame counts do not fit those
  whose timing differs, `vf2o/a/b` for one). A variant has none until its own are made; an `inherit_tours` flag set
  only after a check is the safe way.

A tool to make this cheap: the survey's override, productised as
`remodel --swap parent-chip=variant-chip,... run`, plus a `remodel
idle-find` that searches a variant's program for the parent's loop bytes (it
found 13 of 14 here) and falls back to naming the hottest loop of a run (how
`sfight`'s was found); and a check at start-up that warns when a variant's
idle loop records 0 skips. The three hand-written clones should be
re-expressed as variants once the form exists, with their bench hashes
unchanged as the test.

### Launcher

One card per title (the parent's, with its poster and bezel: clones already
share them). On a card with variants, a **Version** control (a dropdown or a
second row in the card's detail, and the same list under the game's
settings) lists what is installed: the default first (the parent set,
whichever revision it is), then official revisions newest to oldest, then
regions, then prototypes and cross-board versions, each with its label and a
badge for kind. Hacks (the four Daytona ones) and prototypes sit behind a
"Show hacks and prototypes" setting, off by default, since some are not
worth running and two do not boot. Only variants whose chips are present are
listed (`sets_with_clones` already knows which). The choice is remembered per
title and "quick resume" resumes the variant last played; the card's live
attract preview uses the selected variant's own `attract_frame`. A variant
that is class C stays out of the list until it plays (a manifest flag, as
`IN_PROGRESS` does for games).

### Saves, EEPROMs, packs, scenes, netplay, bench

- **Save states and EEPROM per variant id**, in `out/<id>/` as now: a
  variant's settings records differ (`indy500d`, `daytonas`), and a state
  embeds machine memory of one program, so loading across variants must be
  refused. Check the program hash in the state header, not only the game
  name. Never share an EEPROM between a parent and a clone; a "copy settings
  from the parent" button would write another layout's record.
- **Texture packs**: `pack_sets` (variant, then parent) is right. Pack keys
  are the textures' content, so a clone that shares its textures (31 of the
  32) uses the parent's images; one with its own texture chips (`dayton93`,
  4 chips about 40% different) misses harmlessly and wants a pack of its own.
  Nothing to change except to list a variant's pack in the studio under its
  own id.
- **Scenes**: per variant, empty by default (above); a scene's capture keeps
  the id it was made from.
- **Netplay and link play**: the ROM hash (`linknet::rom_hash(&roms.program)`,
  in the handshake and mDNS TXT record) must include the variant id and
  should cover every chip, not only the program; linking different variants
  is not expected to work (their link blocks and records may differ:
  unconfirmed), so require equal ids. `LinkPlay` by variant id: reuse the
  parent's where the record was checked identical (`srallycb`, `srallyp`),
  and mark the others as having none until surveyed (`indy500d`/`to`: other
  layouts; Daytona clones unknown).
- **Bench hashes**: keep `tools/expected-hashes.txt` and `make bench-all` at
  the parents (25 sets). Put variants in a second file
  checked by `make bench-variants`, one line per variant, added when the
  variant is confirmed to play; the measured hashes above (blank EEPROM,
  600M, coin-start, with the temporary swap) are the starting values but
  were taken on code that is not committed, so re-measure on the real
  implementation. A clone that shares its program with another does not need
  a second line.

### Order

(Steps 1 and 2 are built: [Status](#status-what-is-built).)

1. The infrastructure on the three existing clones: the `Variant` type and
   resolver, subfolder lookup in zips, listing in the launcher, the program
   hash in state headers and the netplay hash; `manxttc`, `sfight`,
   `overrevb` re-expressed with unchanged bench hashes.
2. The 9 A variants (a manifest entry each, plus `idle_loops` and
   `attract_frame` for speed and the card): `topskatrj/u`, `vcopa`,
   `srallycb/p`, `lastbrnxj/u`, `daytonat/ata`.
3. The B variants with only an idle loop to move: `stcca/b`, `vf2o/a/b`,
   `vstrikro`, `vonj`, `dayton93` (own file list), `dyndeka2` (protection
   addresses), `zerogunj` (find its loop).
4. The settings-record variants: `daytonas`, `daytonase` (their preset
   `backup_ram`), `indy500d`, `indy500to` (records and `LinkPlay`).
5. Cross-board: `doaa` (works with a 2A manifest and one protection address),
   then the three 2A clones of `zerogun`/`pltkids` (why the 3D is black or
   the run stalls), then the 2B and 2C `dynamcop` clones (they need their
   chip layout worked out from the code).
6. The Daytona hacks (`daytonagtx`, `daytonam`) last, behind the hacks
   setting, if at all.

## What this survey could not determine

- Whether a variant's own gameplay, sound level or HUD differs: only the
  first race, fight or stage of a 600M-instruction coin-start run was looked
  at, no sound was checked, nothing was played.
- Real idle loops where the loop is in RAM-run code (`zerogunj`, `doaa`,
  the 2A clones); `daytonas`/`daytonase`/`vf2b` found by bytes only (the run
  ended elsewhere).
- Settings records and link data of the Daytona clones, and whether linked
  play between variants could work.
- Why the 2A clones `zeroguna`, `zerogunaj` and `pltkidsa` and the three 2B and
  2C `dynamcop` clones fail on a generated manifest: the generated manifests'
  chip roles and packing were guesses, so these are failures of the guess
  until shown otherwise.
- What `bank0.bin`/`bank1.bin` (`daytonagtx`) and `maxx.14` (`daytonam`) are.
- Whether the kinds are right: they are the dat's names, the chip names and
  the screens, not documentation; "official" does not mean a given revision
  was released in the markets its label suggests.
- Whether the `coin-start` scripts, written for the parents, reach play on
  every variant (one, `indy500to`, did not), or `make state-check`, lockstep
  and the recompiled build on any variant.
