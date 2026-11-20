# Content coverage: reaching all of a game

Shared words, roles and rules: [README.md](README.md). Nothing here is
built into reModel; what was tried is in "Evidence" (spike code is on
unmerged branches, not for merging as it stands). Added on 2026-11-19
after the user's review: a full uplift cannot be made from a catalogue
that only holds what somebody happened to play.

## The problem

Every other spec starts from the catalogue: the list of textures, tiles,
layouts and objects a game was seen to draw. A pack is "complete" only
against that list. If the list is short, the pack has holes that show the
first time a player picks the course nobody recorded.

**Observed.**

- **The catalogue records only what is shown** while a game runs
  (`crates/model2/src/catalogue.rs`, module doc; `docs/uplift-studio.md`,
  "Observing").
- **Top Skater's catalogue comes from three scripted runs**: a coin
  start, a walk along the character select, and one course
  (`out/topskatr/catalogue-runs.txt`, local). Together they are 20,557
  frames, about six minutes of play.
- **Its own coverage figure misleads.** The report gives textures
  "100.0%" while the last of the three runs added 151 of the 565
  (`out/topskatr/catalogue/report.txt`, local). The figure is one minus
  the share of sightings seen once, "a guide, not a measurement"
  (`crates/cli/src/catalogue.rs`, `coverage_estimate`). It measures how
  well the visited screens were sampled, and knows nothing of screens
  never visited. The same holds on four more games: `vf2`, `daytona`,
  `gunblade` and `fvipers` each reported 99.9% or 100% after one run,
  and a second run then supplied 50%, 40%, 35% and 61% of the final
  texture list (measured 2026-11-19 and 2026-11-20).
- **There is no list of models to count against.** A model's address
  appears only in the display list the game builds as it runs. A model
  has no length; it ends at a marker (`docs/hw/geometrizer.md`, 2.1 and
  3.1-3.3). No tool walks the polygon ROM.
- **A texture's key needs a running game.** Its size, luma table number
  and translucency are fixed in a header in ROM, but the texels are in
  texture RAM, which the game fills from its data ROM a little each
  frame, and the colour is a palette entry the game writes
  (`docs/hw/geometrizer.md`, 5.1 and 5.3; `raster::tex_info`).
- **Loads are already traced to ROM.** The catalogue logs each change to
  texture RAM and finds the same bytes in the data ROM
  (`crates/model2/src/texwatch.rs`; `docs/uplift-studio.md`, "Loading").
- **Games can be driven by script**: `--input` with named buttons and
  frame ranges, `--save-state FILE@FRAME` and `--load-state`
  (`docs/usage.md`). A headless run is about 2.2 times real time
  interpreted and 4.4 recompiled (`docs/status.md`).
- **Nothing records a person's inputs.** The player cannot write an input
  script, and there is no replay format.
- **Nothing searches or changes RAM.** Cheats are planned, not built
  (`out/NEXT.md`, local). Manifests do name the `test` and `service`
  switches.
- **Which code ran is recorded**, per run, and overwritten by the next:
  every instruction address the interpreter executed
  (`coverage.txt`, written by `remodel run`).

## Design

Four parts: a way to know how much has been seen, ways to reach the
rest, a browser for what no run has shown, and a pack that behaves
when the list was still short.

### 1. A number to count against: ROM read maps

The geometrizer reads every model it draws from the polygon ROM, and
every texture header and UV list from the texture ROM. The texture
watcher already knows which data ROM bytes each load came from. So record,
on the side, which bytes of each ROM a run has used.

```
coverage topskatr
polygon ROM   61% of non-blank bytes read   (3 tours)
texture ROM   58% of bytes read
data ROM      44% of texel ranges loaded
tiles         catalogue only: no ROM source known
code          31% of discovered instructions run
```

(The figures are invented, to show the shape.)

This needs no knowledge of where models begin, works for every game the
same way, and reads only: nothing in the machine changes, and state
hashes are unchanged with it on (E1, E2). Texture headers and UV lists
are read where the raster stream is built (`raster::build_impl`), which
both renderers share, so one hook covers both (E2).

It gives the one thing the catalogue cannot: the share of the game's art
data that no run has touched. It will not reach 100%, since ROMs hold
padding and unused art, so the report also lists the largest unread
stretches. What the maps showed:

- **Today's catalogues have seen a minority of the model data**: 28%
  on Top Skater and 9% to 47% on the others tried (E1, E3). That is a
  ceiling on how complete any pack made from them can be.
- **The unread part is whole regions**, not holes (E1), and nearly
  every unread stretch parses as models from its start (on Top Skater
  the reader chained through 1.72 MB of the 6.0 MB unread; the rest is
  a format it does not know, or not models). A strict reader finds
  them from the first word
  where models sit back to back, and a few words in where they sit 4
  words apart (E1, E3). So unread stretches of polygon ROM can be parsed
  from their start, and a model browser is within reach: at least 1.72
  MB of models nobody has seen can be listed, and their shapes drawn,
  without playing. Their textures need the right sheet loaded, which only
  the game does.
- **There is no true count of texture headers.** Nothing marks one, so
  the report gives bytes read and the largest unread stretches, not
  "headers". "Texels loaded" is only a lower bound: Top Skater's loads
  were the same in every run, which looks like one fill at boot (E2).
- **A reused output folder changes the state hash by itself**, because
  it carries `eeprom.bin` over from the last run. A tour must start from
  a stated EEPROM, or it does not reproduce (E2).

**Known defect.** The read map logs models drawn from polygon RAM as if
they were read from ROM. The figures it affects: the `vf2` and `fvipers`
columns in E3 include up to two such small models a frame and were not
redone; `vstriker` and `lastbrnx` had it filtered out. The fix
proposed, not tried: record reads only where the geometrizer reads the
polygon ROM.

The catalogue's own figure is renamed to what it is, "sampling of the
screens visited", and the report leads with new entries per thousand
frames in the latest run, which is the honest sign of saturation.

### 2. Reaching the rest

Five ways, from cheapest to most capable. All of them produce the same
thing, a **tour**.

**A tour** is a named input script from power-on: today's line in
`catalogue-runs.txt` (`LABEL|STEPS|INPUT`). It is the durable root of
everything else. Run with the user's ROM it rebuilds the same catalogue
entries and the same frames, because emulation is exact. A scene
([scenes-and-server.md](scenes-and-server.md)) is a tour and a frame
number. A save state is only a cache of a point in a tour, and may be
thrown away when the state format changes.

Two additions to the line format:

- `from=LABEL@FRAME` starts a tour at a point in another, so ten
  characters are ten short branches from one walk to the select screen.
- `mark=NAME@FRAME` names a frame as a scene.

| Way | Reaches | Cost |
|---|---|---|
| **Attract soak** | Whatever the demo loop shows | None: run with no input for ten minutes |
| **Recorded play** | Anything a person can play to | A recorder in the player that writes a tour as the game is played. Small, and missing today |
| **Pokes** | Late stages, hidden modes, a stopped clock, without skill | The planned cheats work: `ram search` to find an address, then `poke=ADDR:VALUE@from-to` in a tour |
| **The test switch** | Whatever a game's test menu offers | Free where it helps; unverified for any game |
| **The crawler** | Breadth: every branch of every menu | Below |

**The crawler** explores the way a fuzzer does. It keeps a set of save
states. From one, it holds a button or a direction for a short while,
then asks whether anything new appeared: a catalogue entry, a newly read
stretch of ROM, a new screen. If so the new state joins the set and the
inputs that reached it are kept as a branch tour. Menus are where a
game's breadth is, and they are what this is good at: left, right and
start, from every screen. Loading a state takes about 100 ms and a state
is about 1.2 MB, so thousands of tries fit in an hour on one core.

What it can and cannot do (E4):

- **It needs no new code to start.** A catalogue session can start from
  a loaded state, and a tour replayed from power-on twice is identical
  byte for byte.
- **It finds menu screens unaided**, and finds art a hand-made catalogue
  lacks. It does not play: random stick holds do not skate a course, so a
  run ends at TIME UP and the game-over loop. Later course sections, and
  anything past a menu that needs skill, need recorded play or pokes.
- **Waiting is an input.** Menus advance on timers; "wait six seconds"
  and "hold start" find the most.
- **The policy must spread tries across branches.** A greedy policy
  saturates and descends almost wholly from one branch.
- **Screens are compared with digits masked.** A credit counter
  otherwise makes a new screen per digit.
- **A try is two passes.** A try costs about 2 seconds, and the catalogue
  is 60 to 70% of it. So run plainly with the read map first, and run the
  catalogue only on tries that read something new.
- **A branch tour can be checked by state hash.** A state reached by
  chained loads is byte-equal to the same frame reached in one run from
  power-on, as long as that run starts in a fresh folder (E4,
  corrected).

An agent sits on top for what the crawler cannot do: read a new screen,
recognise "this is a course select with three entries", and decide what
is worth time. It is shown new screens only, one thumbnail each, with
each state's children and tries since its last find, and two commands:
"wait N" and "go back to state X".

A tour that uses pokes is marked, and so is any state made from it. Tours
are for cataloguing. They never touch play, replays or the benchmark
hashes.

### 3. A browser for what no run has shown

Proposed on 2026-11-20; only the parsing is tried. Unread stretches of
polygon ROM parse as models from their start (E1, E3), so they can be
listed and drawn without playing.

- `coverage models --unread` lists each model found in data no run has
  read: its address, polygon count and size.
- `model show ADDRESS` draws one, turning, in flat shading. Its
  textures need the right sheet loaded, which only the game does. Where
  a model's texture headers point at ROM data the catalogue has seen
  loaded ([asset-identity.md](asset-identity.md), "A second anchor"),
  the textures could be put on (inferred; untried).
- **What it is for.** The pack lead sees what the catalogue lacks as
  shapes, not byte ranges, and can tell which course or character a
  tour should aim at. It also shows art the game never uses.
- **First step.** The list and plain renders for the 1.72 MB that
  parses on Top Skater.

Everything is drawn from the user's own ROM, on their machine. Nothing
of it enters the repository.

### 4. A pack that copes with a short list

- **A pack states what it was made against.** `pack check` reports
  "covers 412 of 417 known surfaces; the catalogue has read 61% of the
  polygon ROM". An honest incomplete pack is better than a claim.
- **Unknown variants fall back to the artwork.** A colour never seen
  before still finds its surface
  ([asset-identity.md](asset-identity.md)).
- **Players can report gaps.** With a setting on, the player writes what
  it drew that the pack did not cover and the catalogue never listed, as
  keys and a screen number, to `gaps.tsv`. An author merges it. Nothing is
  sent anywhere.
- **Catalogues are rebuilt, not shared.** Tours are small text and can
  live in a game's manifest or a pack's repository, as the `coin-start`
  scripts do now. Thumbnails and keys are made on each machine from the
  user's ROM.

### The crawler as a test of the emulator

Outside uplift, and noted for whoever owns emulation. The crawler runs
thousands of short, varied sessions from saved states, which is also a
test.

- **It has already found one thing** nobody was looking for: a leftover
  `eeprom.bin` changing a run (E2). A second report, a chained-load
  state that did not hash like a straight run, was the same thing seen
  again (E4, corrected).
- **What it could report with little more:** a tour that does not give
  the same state hash twice, a panic or a hang, and the count of
  instructions and ROM data no session has reached. Each failure comes
  with the short input script that reproduces it.
- **Picking helps the same person.** Pointing at a wrong pixel and
  getting the model, polygon and texture that drew it
  ([scene-understanding.md](scene-understanding.md)) is a debugging
  tool for the renderer, not only for art.

## Interfaces

| Verb | Does |
|---|---|
| `tour list`, `tour run NAME` or `--all` | Runs tours with the catalogue and read maps on, and saves a state at each mark |
| Player: "Record a tour" | Writes inputs as a tour line while a person plays |
| `coverage [--json]` | The report above, and the largest unread stretches |
| `crawl --from TOUR@FRAME --minutes N` | Explores; writes branch tours that found something new |
| `ram search`, `poke=` in a tour | Find and hold a value |
| `pack check` | Adds coverage against the catalogue, and the catalogue's against the ROM |
| Player setting "Report gaps" | Writes `gaps.tsv` |

## Data formats

- **`tours.txt`**: `LABEL|STEPS|INPUT`, with `from=`, `mark=` and `poke=`
  as input terms. A project's own file, plus any in the game's manifest.
- **Read map**: per ROM, a sorted list of byte ranges, merged across
  runs. Kept beside the catalogue; never in the repository.
- **`gaps.tsv`**: kind, key, screen, frames seen.

## The roles

- **Pack lead.** Reads `coverage` before planning a pack, and decides
  when the list is good enough to start.
- **Art agent.** Unaffected: it is handed assets. Budget for a coverage
  agent steering the crawler: one thumbnail per new screen.
- **Artist.** Plays the game with "Record a tour" on. That is the whole
  job.
- **Tester.** A tour run twice gives the same catalogue and the same
  state hashes. With read maps on, state hashes are unchanged.
- **Player.** Sees fewer holes, and can report the ones that remain.
- **Tool engineer.** A new board gets read maps by recording reads in
  one place per ROM.

## Testing

- **Without ROMs:** range merging; the tour parser; a crawler on a
  synthetic machine with a known menu tree finds every leaf; a poke
  marks its tour.
- **With ROMs:** the three existing runs as tours give today's catalogue;
  hashes equal with read maps on and off; a branch tour from a state
  gives the same pictures and catalogue entries as the same inputs from
  power-on (the state hashes differ; see the crawler); read maps ignore
  models drawn from polygon RAM.

## Performance

The three runs are 20,557 frames: about six minutes at the game's speed
and about 80 seconds recompiled (worked from the figures above; not
run). The read map slowed one Top Skater run from 17 s to 28 s and the
other two not at all (E1). The texture hook cost 1.0 to 3.3 ms a frame,
about 80% more wall time, as written (E2). The catalogue's own cost is
60 to 70% of a crawler try of 2 seconds (E4). Recording which code ran
is done by the interpreter only, so the crawler's finest signal costs
half the speed.

## Evidence

Spike code is on unmerged branches; none of it is in reModel. Each entry
says what was observed and what is inferred.

### E1. Polygon ROM read map (2026-11-19)

Top Skater's three runs, native geometrizer; branch `uplift-spikes`,
about 200 lines.

Observed:

- The three runs read 28% of the polygon ROM: 2.38 MB of 8.39 MB. The
  coin start alone read 16%, the character walk added 1%, and the course
  added 11%. None of the ROM is blank.
- The unread part is whole regions, not holes: 40 stretches, the
  largest 4.89 MB, and only 64 KB in stretches under 16 KB.
- 95% of models read begin exactly where another ended.
- A strict reader (attribute bits, finite numbers, not only the end
  marker) read all 40 unread stretches from their first word, and
  chained through 1.72 MB of the 6.0 MB unread.
- State hashes were unchanged. The map slowed one run from 17 s to 28 s
  and the other two not at all.

Inferred: the rest is either a format the reader does not know or not
models at all.

Not tested: drawing unread models in a viewer.

### E2. Texture data read map (2026-11-20)

Branch `uplift-spikes-b`, about 230 lines.

| | Top Skater, 3 runs | `gunblade`, coin start | `daytona`, coin start |
|---|---|---|---|
| Texture ROM read (headers and UV lists) | 18% of 8 MB | 13.5% | 3.6% of 16 MB |
| Data ROM copied into texture RAM | 6.4% of 32 MB | 9.8% | 4.1% |

Observed:

- Top Skater's texel loads were the same 2.14 MB in all three runs.
- A pattern learned from the headers that were read guesses 65% read on
  Top Skater, 24% on `gunblade` and 5.5% on `daytona`.
- The hook cost 1.0 to 3.3 ms a frame, about 80% more wall time, as
  written. State hashes were unchanged.
- A reused output folder changes the state hash by itself: it carries
  `eeprom.bin` over from the last run.

Inferred: the header guesses are fitted to what was read, so they are
guesses, not counts.

### E3. Polygon ROM across five and seven games (2026-11-20)

`vf2` and `daytona` first: a coin start and a run with no input each.
The two Model 2B games were added the same day.

| | Top Skater | `vf2` | `daytona` | `gunblade` | `fvipers` |
|---|---|---|---|---|---|
| Board | 2C | 2A | Original | 2B | 2B |
| Polygon ROM, share not blank | 8 MB, 100% | 12 MB, 87% | 16 MB, 74% | 16 MB, 86% | 12 MB, 86% |
| Read, of the whole ROM | 28% | 28% | 18% | 14% | 14% |
| Read, of what is not blank | 28% | 33% | 24% | 17% | 16% |
| Models sit | Back to back | 4 words apart | 4 words apart | Back to back | 4 words apart |

Correction: an earlier version of this table gave the whole-ROM figures
under the "not blank" label. Both rows are kept.

Observed:

- One coin start and one run with no input read 47% of `vstriker`'s
  polygon ROM (8 MB, a football pitch shows most of its models at once)
  and 9% of `lastbrnx`'s (8 MB, one fight of many).
- On Daytona a scripted race read only 7 KB more than the title run:
  the demo had already shown that course, and the other courses need
  choosing.
- On all four added games nearly every unread stretch parses as models:
  from its first word where models sit back to back, and from a start a
  few words in where they are 4 words apart.
- `gunblade` has by far the most unread stretches (98,313, the largest
  4.2 MB), so its data is more broken up than the others'.
- The map logged models drawn from polygon RAM as ROM reads. It was
  filtered out for `vstriker` and `lastbrnx`; the `vf2` and `fvipers`
  figures above include up to two such small models a frame and were not
  redone.

### E4. The crawler on Top Skater's menus (2026-11-20)

A Python loop around the CLI: 1,238 tries in 41 minutes of emulator
time, one process.

Observed:

- A catalogue session can start from a loaded state, and a tour
  replayed from power-on twice is identical byte for byte.
- It found most menu screens unaided: course select, all six skaters,
  the countdown, play, TIME UP and two caution screens. It rediscovered
  354 of the 565 known textures and found 51 textures, 148 objects and
  17 screens the hand-made catalogue lacks.
- It did not reach the Expert course, and it saturated: 80 of the 118
  states kept came in the first quarter, 5 in the last. The policy was
  greedy, so 117 of 118 descend from one branch.
- A try costs 2 seconds, and the catalogue is 60 to 70% of it. Process
  start and state load are a tenth.
- The credit counter made a "new screen" per digit: 36 screens by the
  catalogue's count, about 12 by eye.
- As first reported, a state reached by chained loads was not
  byte-equal to the same frame reached in one run: its hash and
  instruction pointer differed, while pictures 120 and 300 frames later
  were identical.
- **Corrected 2026-11-21:** that was the leftover `eeprom.bin` of E2,
  not the loads. The crawler's kept states are byte-equal to runs from
  power-on in a fresh folder, with the crawler's own binary: one load
  from the root (frame 1693) and its deepest state (frame 9015, 49
  loads from the root by the crawler's record). On main (`8ad6a1e`),
  one and two loads are byte-equal too, with the catalogue on and in a
  shared folder. Only a power-on run started in a folder that holds an
  `eeprom.bin` differs, in hash and instruction pointer.

Inferred: the design holds for breadth.

Not tested: the two-pass try, a spread policy, masked screens, an agent
steering the crawler.

## Open questions

- **What is the other 4.3 MB** of the polygon ROM that the strict reader
  could not chain through: unused art, another record format, or data
  that is not models?
- **Can unread models be drawn** in a viewer without the game ever
  showing them? Their shapes need nothing else; textures need the right
  sheet loaded.
- **How much of the unread art can the game ever show?** ROMs keep
  leftovers. A crawler that stops finding new reads is the only test.
- **Do tours survive timing fixes?** A corrected emulator may shift
  frames, breaking `from=` and `mark=`. `scene verify` catches it;
  whether marks should be found by screen and not by frame is open.
- **Where 2D patterns come from** is not documented, so tiles have no
  read map yet.
- **Can settings in EEPROM** (difficulty, region, blood) change the art?
  A tour must carry the EEPROM it starts from either way: a leftover
  `eeprom.bin` changes the state hash (E2).
- **Is the test switch useful** for any game?
- **How far can the crawler get** past a menu, where reaching new art
  takes skill and not breadth? Pokes are the likely answer.

## MVP

1. Read maps for the polygon ROM and the texture ROM, merged across
   runs, recording reads only where the geometrizer reads ROM so RAM
   models are not counted.
2. `tours.txt` with `from=` and `mark=`, `tour run`, and `coverage`.
3. The player's tour recorder.
4. `ram search` and `poke=`, with the cheats work.
5. The crawler: spread policy, masked screens, two-pass tries, then an
   agent steering it.
