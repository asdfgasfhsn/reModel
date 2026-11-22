# ROM variants: a survey of every clone of the supported games

The sets table in [roms.md](roms.md) lists which clones each zip holds. Of
the 35 clones surveyed here, 27 are supported as variants (see
[Status](#status-what-is-built)); the other 8 (class C) are not. The survey
covers the 32 clones that were not hand-written manifests, of 14 of the 31
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

Steps 1 to 4 of the [order](#order) are built, and `doaa` of step 5: the
infrastructure, the three earlier clones re-expressed as variants, the nine
class A variants and the fifteen class B variants, all booting, attracting and
reaching play from a blank EEPROM. What is left is the cross-board clones of
step 5 (the 2A clones of `zerogun` and `pltkids`, the 2B and 2C clones of
`dynamcop`) and the Daytona hacks of step 6.

**The model** (`crates/romset/src/variants.rs`, data in
`crates/romset/src/games/variants.rs`): a `Variant` is a thin manifest that
inherits from its parent title's `Game`, resolved once at first use into a
full `&'static Game` (`romset::games::find(id)`, cached, so downstream code
is unchanged and `game.id` is the variant's own id, `game.parent` the
title's):

| Field | Meaning |
|---|---|
| `id`, `parent`, `label`, `kind` | the set id (what `--game` takes); the title's set; the Version list's name ("Rev B", "Japan"); `VariantKind` = Revision, Region, CrossBoard, Prototype or Hack (the launcher lists every kind in the Version list, a hack and a prototype tagged as such); `kind_confirmed` is false where the kind rests on the dat's name and the chip names only |
| `swaps: &[ChipSwap]`, `drop`, `add` | the parent's chip (by name, so by socket and role) replaced by the variant's own (name, size, SHA-1, role); parent chips the variant lacks (`manxttc`); chips the parent lacks, after the parent's (`doaa`'s six 2A geometry programs) |
| `title`, `year`, `board`, `packed`, `attract_frame`, `idle_loops`, `backup_ram`, `volume_db`, `texture_ram_mirrored`, `shading`, `recommended`, `input_lag`, `run_ahead_max` | optional overrides; `None` inherits |
| `protection_rom` | the protection answers' ROM addresses (reply index, address), the rest of the protection inherited (`dyndeka2`, `doaa`) |
| `coin_start` | the `coin-start` input script where the parent's does not reach play (`indy500to`) |
| `own_textures` | its textures are its own chips: texture packs do not fall back to the parent's (`dayton93`) |
| `scenes` | the variant's own; **never the parent's**, and `games::tours` no longer falls back to the parent's either |
| `link: VariantLink` | `Parent` (its settings record is the parent's: `srallycb`, `srallyp`, `daytonat`, `daytonata`, `manxttc`), `None` (the default: link play off for it), or `Own(&LinkPlay)` |

Chips are looked up as `<variant id>/<chip>` before the bare name in each of
the variant's folder and zip and the parent's (`topskatr.zip` keeps
`topskatrj/epr-19753.15` and `topskatru/epr-19753.15`; `RomSet::load`, and
`Zip` now matches a name with a folder against the stored path). A variant
whose swapped chips are not all there is not listed (`romset::sets_with_clones`,
`variant_available`). `remodel games` lists the titles and then the variants.

**Saves and settings**: EEPROM (`out/<set>/eeprom.bin`), save states, the
resume state, JIT hot lists and link settings are keyed by the set id, so a
variant has its own and never touches its parent's. The save file header
(format 3) records the set it was saved from and a load by another variant is
refused naming both. The player's per-game settings (`<set>/settings.txt`) and
remapped controls (`<set>/controls.txt`) **start as the parent's**: a variant
that has no file of its own reads its parent's, read-only, and the first change
made for the variant writes the variant's own file (a variant whose own settings
are all reset keeps an empty file, so the parent's do not come back). The
launcher's remembered version is per title (`version.<title>` in
`play-settings.txt`), so there is nothing to inherit there. Texture packs fall
back to the parent's (`pack_sets`), except for a variant with `own_textures`.
`remodel run` also writes the original Model 2's I/O board record the game left
(`iostore.bin` in the output folder, 128 bytes), which is how a program's own
defaults and layout are found there.

**The launcher** has one card per title; its game page (docs/play.md, "The game
page") has a Version list of every version, hacks and prototypes tagged, and
an About this version row with the variant's kind, link status and whether
texture packs carry over. `--game`, `make GAME=` and the CLI take a variant id
anywhere a set id goes. In pad mode the network banner's Join button carries no
keycap (the J key is the keyboard's).

**Network and link play**: the ROM hash in the session agreement is over every
chip and the set id (protocol 4), so two variants never link; a variant links
as its parent where `VariantLink::Parent`, and has no link play where it is
`None` (see the table of what each class B variant's link and free play is,
below).

**Bench**: `make bench-variants` runs `tools/expected-variant-hashes.txt`
(blank-EEPROM coin-start, 600M instructions, as `bench-all`); the three
earlier clones keep their lines in `tools/expected-hashes.txt`, and their
hashes did not change when they became variants (`manxttc`
`d6558827f751bbb0`, `sfight` `2a0a5a5d0d957997`) nor did `overrevb`'s
(`de800c1e32ddb8aa`, measured before and after). A variant's hash does not
depend on its idle loops (skipping is exact), so the survey's state hashes
(measured with the parent's loops and a chip swap) are the committed ones
wherever nothing else changed.

| Variant | Kind | Chips swapped | State hash (600M, coin-start) | Status |
|---|---|---|---|---|
| `manxttc`, `sfight`, `overrevb` | revision, region, other board | as before | unchanged | the three clones, now variants |
| `topskatrj` Japan | region | program pair | `d5b806cefa994cbe` | boots, attracts, coin-start reaches the race; the inherited attract frame (1650) is in the attract mode, not on the warning (checked headless in the launcher) |
| `topskatru` USA | region | program pair | `de6f5171b2730155` | as `topskatrj`, FBI warning |
| `vcopa` Rev A | revision | program pair | `5c206708efb1d3c8` | plays, the same stage |
| `srallycb` Rev B | revision | program pair | `2d9857e7d3dacb29` | plays; link as the parent |
| `srallyp` "Pro Drivin'" | prototype (*unconfirmed*) | program pair | `ef2b8b45eccd5732` | plays; tagged "Prototype" in the Version list |
| `daytonat` Turbo | hack (*unconfirmed*) | program pair | `0c64f85a8b95e340` | plays; tagged "Hack" in the Version list; link as the parent |
| `daytonata` Turbo Rev A | hack (*unconfirmed*) | program pair | `95da2bac42132d58` | as `daytonat` |
| `lastbrnxj` Japan, Rev A | region | program pair | `af452e8e6839ef0a` | plays; own attract frame 1800 (the parent's 700 is the warning text) |
| `lastbrnxu` USA, Rev A | region | program pair | `5db58c2199e50ec7` | plays; own attract frame 1500 (700 is the SEGA logo) |
| `stcca` Rev A | revision | program pair | `969ed016b6b232c8` | plays: name entry, qualifying; idle loop 0x73424 (76.2% skipped); **link on** (its own settings record) |
| `stccb` Rev B | revision | program pair | `a50918a0e5333ac8` | as `stcca`; loop 0x73574; **link on** (its own settings record) |
| `vf2o` Original | revision | four program chips | `be420a1f95e7e0c6` | plays: fights; loop 0x10A74 (60.7%) |
| `vf2a` Rev A | revision | four program chips | `aa8922832829ebc6` | plays; loop 0x10A98 |
| `vf2b` Rev B | revision | four program chips | `6790a5d07fdfd8f7` | plays; loop 0x10B3C |
| `vstrikro` Original | revision | four program chips and the `.7/.8` pair | `940ba002561c05cb` | plays: ranking, a match; loop found by the hot spots of the run |
| `vonj` Japan | region | program pair | `c3bf3413fbdd155b` | plays; loop 0x28E2C; **link on** (the parent's data) |
| `indy500d` Deluxe, Rev A | revision | program pair, `.7/.8` pair, sound program | `db57fdba291b8487` | plays; Japan warning then races; loop 0x345D4; attract frame 2250; link play and free play: its own (below) |
| `indy500to` Twin, older | revision | program pair, `.7/.8` pair, sound program | `56b54eb96991f0a8` | plays from its own `coin_start` (start press at 1900); loop 0x2E1C4; attract frame 2250; link play and free play: its own (below) |
| `dayton93` '93 Edition | revision | ten chips (program pair, main data `.8/.9` and `.6/.7`, polygons `.19/.23`, textures `.26/.27`), other sizes | `2a4707a921d968df` | plays; loop 0x12B0 (the parent's second loop is not in this program); own textures; **no link play** (its program has none: below) |
| `daytonas` Saturn Ads | revision | program pair and `.6/.7` | `39fb66ba9afb4930` | plays with its own preset settings record; loops 0x13A8 and 0x1BDA8; link play and free play: its own (below) |
| `daytonase` Special Edition | revision | program pair and `.6/.7` | `e6e62eba70d36995` | as `daytonas`; loops 0x13A8, 0x1BD38 |
| `zerogunj` Japan | region | program pair | `171d4f54c4caa140` | plays: mission 1, continue; its loop is in RAM-run code, found by the run's hot spots (0x5490A0) |
| `dyndeka2` Dynamite Deka 2 | region | program `.12/.13` | `a3b912a344e41ee1` | plays; protection answers 0x10 lower (`protection_rom`); loop 0x71184 (82.8%) |
| `doaa` Model 2A | other board | program pair, plus the six 2A `opr-*` chips | `c815a2be3623d98d` | plays: Kasumi against Zack; protection answer at 0x2830 (`protection_rom`; 400M-instruction run `3cdae73e2525aa8b`, as the survey); loop 0x2800 (79%) |

The first nine hashes are the survey's own, measured again on the committed
implementation: they agree (the survey's temporary patch and the resolver build
the same machine), and so do the class B ones that were measured then (`doaa`'s
survey figure was a 400M run, and its 600M hash is the one in the bench file;
`indy500to`'s differs from the survey's `ed4ac127b1c34bff` because the survey's
`coin-start` did not start a race and the variant's now does). "Plays" is as the
survey defines it: boots, attracts, coin-start reaches play, frames look like
the game; nobody played them by hand. The hacks' and the prototype's *kind* is
the dat's and the chip names', not documented. The Turbo hacks run on the
parent's idle loops and settings record, so their link play is the parent's *by
the survey's checks only*.

### Class B: what each needed

- **Idle loops** are the parent's loop found again in the variant's program.
  The method is the parent's (docs/idle-skip.md): the hottest code of a
  coin-start run (`remodel run`'s "hot spots (current frame)") read in the
  program, and, for loops that were not in the last frame's hot spots
  (`vf2b`, `doaa`, the two Daytona Saturn sets), the parent's instruction
  words searched for in the variant's program. Each was then confirmed by its
  effect: a run with the loop listed skips 60 to 83% of the instructions and
  ends on the same state hash. `zerogunj`'s, which the survey could not find,
  is RAM-run code like the parent's and was in the hot spots (0x5490A0,
  0x50 above the parent's). `dayton93` has only the first of the parent's two
  loops: the second is a function that this older program does not contain.
  Nothing a loop's address can do changes a state hash.
- **Protection answers**: `dyndeka2`'s two 16-byte replies are 0x10 lower
  than the parent's (0xBC7E0, 0xBC7F0) and `doaa`'s single reply is 0x20
  higher (0x2830), each found by searching the program for the reply's
  bytes; both run to play with them and fail without.
- **`doaa`** is the cross-board clone: board 2A, the parent's data, polygon,
  texture and sound chips, its own program pair and the six `opr-*` programs
  every 2A set has (`add`, role `CoproData`, as `dynamcop`'s); the 2A model
  needed nothing else from the parent's 2B manifest (`packed` stays empty).
- **`dayton93`**'s chips are at other sizes (`.6/.7` 1 MB for 512 KB, polygons
  `.19/.23` 512 KB for 2 MB) and the manifest takes each chip's size from the
  variant's own entry; its textures are its own, so `own_textures` stops the
  parent's texture pack serving it.
- **Settings records**: `daytonas` and `daytonase` reject the parent's preset
  record (`backup_ram`: a version and CRC of the parent's). Each program's own
  defaults were found by running it with no preset and reading the record the game
  left in the I/O board's store (`remodel run` writes `iostore.bin`): version 4
  (Saturn Ads) and 3 (Special Edition) in the header's third byte, an extra
  item (the "PROMOTE SATURN" page) and other defaults, all valid CRC-16 (the
  parent's rule: CCITT, start 0, over bytes 0x0A-0x7F, stored at 8, recomputed
  here and equal to what the game wrote). With the link ID byte (0x0B) set to
  0 (SINGLE) and the CRC recomputed, as for the parent, each is the variant's
  `backup_ram`, and the game goes to attract and a race instead of the network
  check. (`dayton93` needs none: with an empty store its program writes its own
  record, version 1, whose link ID byte is already 0 (SINGLE), and it
  attracts and races; it keeps the parent's preset in the manifest, which
  the program rewrites to version 1 keeping the parent's settings bytes, and
  its state hash is the survey's. Whether the preset is accepted or only
  migrated was not separated.)
  `indy500d` and `indy500to` keep their settings in the EEPROM, which the game
  writes itself from a blank one, so they need no preset.
- **`indy500to`'s start**: the survey's `coin-start` presses start at frame 1250,
  while this release is still on its warning screen (until about 1500) and the
  ranking screen is not up; the credit is counted and the game waits on PRESS
  START BUTTON. The variant's own script presses start at 1900 and the race
  follows.
- **Attract frames**: `indy500d` and `indy500to` 2250 (their parent's 1500 is the
  title logo), `lastbrnxj` 1800 and `lastbrnxu` 1500 (700 is the warning text and
  a logo); the others show attract at the inherited frame (checked on contact
  sheets of a no-input run for every variant).

### Link play and free play of the variants

| Variant | Link play | Why |
|---|---|---|
| `srallycb`, `srallyp`, `daytonat`, `daytonata`, `manxttc` | the parent's | the settings record was checked to be the parent's |
| `daytonas`, `daytonase` | own (up to 8, as the parent) | the I/O board's record, as the parent's kind (link ID 0x0B: 0 SINGLE, 1 MASTER, 2 SLAVE; CAR NUMBER 0x0C; CCITT CRC at 8), with the program's own version byte (4, 3) and defaults, found by running each program's GAME SYSTEM page (LINK ID is its first row; the Saturn Ads page also has PROMOTE SATURN). FREE PLAY is COIN ASSIGNMENT #27 as the parent's (bytes 0x10 and 0x14 the same); byte 0x7C is **not** a free-play byte: it is the record's save counter (the game raises it at every save, whichever setting changed: 1 to 2 here, 0 to 1 in the parent), which the entry carries so the record equals what the game writes. *Confirmed (by running it)*: `make link-record-test GAMES="daytonas daytonase"` |
| `dayton93` | none: **no link code** | its GAME SYSTEM page has no LINK ID or CAR NUMBER (ADVERTISE SOUND, COUNTRY, CABINET, DIFFICULTY only); with the idle link board fitted (`make link-trace`) a coin-start run makes **no access** to the board's address range, and two `link-run` nodes with link IDs 1 and 2 in their records ran two stand-alone races (no "starts the board", 40 cars, POSITION 40/40 on both). *Confirmed (by running it)*; the program was not read through for link code. Its free play bytes are the parent's (found; byte 0x7C counts 1 to 2), but free play is only offered with link play, so none |
| `indy500d` | own (up to 4, as the parent) | its record is the S32A kind with **another size**: the record is 44 bytes (the copy at 0x34, not 0x2C), the checksum the parent's routine (start 0xDEBDEC00) over the 42 bytes after the word, NETWORK TYPE at **0x28** and CABINET ID at **0x29** (copies 0x54, 0x55); the two bytes 0x1E/0x1F, 18 and 17, are other settings. FREE PLAY is COIN ASSIGNMENTS #27 as the parent's (bytes 0x10, 0x11, 0x15), from this variant's default #17 (3 coins 1 credit, byte 0x10 0x10, byte 0x11 3). Format word 0x0711. *Confirmed (by running it)*; the record equals the game's own save (`make link-record-test`) |
| `indy500to` | own (up to 4, as the parent) | the parent's layout and checksum (record 36 bytes, copy at 0x2C; NETWORK TYPE 0x1E, CABINET ID 0x1F) with other defaults (byte 0x1C 1 where the parent's is 2). The earlier "the parent's test-menu route changed nothing" was the route's presses, not another menu (*probable*: that script was not kept): a test press held a few frames repeats, moving the value two or three steps, so 15 presses end away from #27; with one frame of press every 100 frames, 8 presses from #12 reach #27 FREE PLAY (the menu pages are the parent's, with ENGINE VOLUME and DEFAULT VIEW rows after CABINET ID). FREE PLAY bytes as the parent's (0x10, 0x11, 0x15; default #12). The game accepts MASTER and SLAVE. *Confirmed (by running it)* |
| `vonj` | the parent's | the backup SRAM record (`SEGA.AM#3.VRON.`, the checksum, the defaults) is the parent's byte for byte, but for a run counter; a linked match runs (`docs/hw/link-board.md` section 15.4) |
| `stcca`, `stccb` | their own (`VariantLink::Own`) | the record is `SEGASTCa`, four settings bytes shorter than the parent's and not the same defaults; the checksum rule (CCITT over `0x10`-`0x7F`), the role byte (`0x10` bits 5-7) and the link are the parent's; their free play bytes are at `0x14`-`0x15` (`docs/hw/link-board.md` section 16) |
| the others (`vf2*`, `vstrikro`, `zerogunj`, `dyndeka2`, `doaa`) | none | their parents have none |

`make link-free-play-test`, `link-net-test` and `link-record-test` take the four variants
with link play as well (`GAMES="daytonas daytonase indy500d indy500to"`): a two-machine
race, free play from a start press, the network run's hashes equal `link-run`'s, and the
records equal what each game writes from its own test menu. `link-run` also ran the parents'
maximum: `daytonas` and `daytonase` with 8 machines (POSITION 1 to 8 of 8) and `indy500d` and
`indy500to` with 4 (1 to 4 of 4). `link-state-check` runs on the parents only and is unchanged.
Two machines of one variant link; a machine of another variant, or of the parent, is refused
(the set id is in the ROM hash).

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

(The tables are the survey's, as measured before anything was built; what was
built, and the corrections to them (`vstrikro`'s hash, `zerogunj`'s loop, `indy500to`'s
start, `daytonas`' record), is in [Status](#status-what-is-built).)

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
| `indy500d` Deluxe | revision (Japan warning screen), official | program pair `epr-18251a.15/18252a.16` (20% equal), `.7/.8` pair `epr-18345/18346` (about 33%), sound program `epr-18253.31` (94%) | plays: Japan warning, race; `db57fdba291b8487`; idle skip 0% | idle loop 0x34914 to 0x345D4; **the settings record is another size and layout (checksum over the parent's range fails, no copy at 0x2C, role and cabinet bytes at 0x1E/0x1F hold other values: 18, 17)**, defaults differ (the race showed credit 2/3); *since built*: its own `LinkPlay` (record 44 bytes, the copy at 0x34, checksum the parent's routine over 42 bytes, NETWORK TYPE 0x28, CABINET ID 0x29; see Link play and free play of the variants) | B |
| `indy500to` Twin (older) | revision, official | program pair `epr-18254a.15/18255a.16` (19%), `.7/.8` `epr-18389/18390`, sound program `epr-18391.31` (98%) | boots, attract in Japanese, **the `coin-start` script's start press does not start a race by frame 3600** (still on "PRESS START BUTTON"); `ed4ac127b1c34bff`; idle skip 0% | idle loop to 0x2E1C4; the record has the parent's S32A checksum and seed (valid) but fewer settings, so offsets such as the role byte need re-finding (*since found*: the parent's 0x1E/0x1F); input timing | B |

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

32 clones of 14 supported titles, plus the 3 already supported. Built: all of
class A (9) and class B (15); class C (8) is not.

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
tag for a hack or a prototype. Every version is listed (the launcher once
hid hacks and prototypes behind a setting, since removed). Only variants whose chips are present are
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

(Steps 1 to 4 and `doaa` of step 5 are built: [Status](#status-what-is-built).)

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
6. The Daytona hacks (`daytonagtx`, `daytonam`) last, if at all.

## What this survey could not determine

- Whether a variant's own gameplay, sound level or HUD differs: only the
  first race, fight or stage of a 600M-instruction coin-start run was looked
  at, no sound was checked, nothing was played.
- (Since built: `zerogunj`'s and `doaa`'s loops, and `vf2b`'s and the Saturn
  sets', are found and confirmed by their skips. Still open: the 2A clones'
  loops, which are not built.)
- (Since built: the link data of `daytonas`, `daytonase`, `indy500d` and `indy500to`;
  `dayton93` has no link code. See the table in Status.) Whether linked play between
  variants could work is open (not tried).
- Whether `dayton93` accepts the parent's preset record or only migrates it
  (its program runs stand-alone either way).
- Why the 2A clones `zeroguna`, `zerogunaj` and `pltkidsa` and the three 2B and
  2C `dynamcop` clones fail on a generated manifest: the generated manifests'
  chip roles and packing were guesses, so these are failures of the guess
  until shown otherwise.
- What `bank0.bin`/`bank1.bin` (`daytonagtx`) and `maxx.14` (`daytonam`) are.
- Whether the kinds are right: they are the dat's names, the chip names and
  the screens, not documentation; "official" does not mean a given revision
  was released in the markets its label suggests.
- Whether the `coin-start` scripts, written for the parents, reach play on
  every variant (one, `indy500to`, did not and has its own now), or `make state-check`,
  lockstep and the recompiled build on any variant.
