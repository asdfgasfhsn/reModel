# Content coverage: reaching all of a game

Shared words, roles and rules: [README.md](README.md). Nothing here is
built. Added on 2026-11-19 after the user's review: a full uplift cannot
be made from a catalogue that only holds what somebody happened to play.

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
  never visited.
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

Three parts: a way to know how much has been seen, ways to reach the
rest, and a pack that behaves when the list was still short.

### 1. A number to count against: ROM read maps

The geometrizer reads every model it draws from the polygon ROM, and
every texture header and UV list from the texture ROM. The texture
watcher already knows which data ROM bytes each load came from. So record,
on the side, which bytes of each ROM a run has used.

```
coverage topskatr
polygon ROM   61% of non-blank bytes read   (3 tours)
texture ROM   58% of headers read
data ROM      44% of texel ranges loaded
tiles         catalogue only: no ROM source known
code          31% of discovered instructions run
```

(The figures are invented, to show the shape.)

This needs no knowledge of where models begin, works for every game the
same way, and reads only: nothing in the machine changes. It gives the
one thing the catalogue cannot: the share of the game's art data that no
run has touched. It will not reach 100%, since ROMs hold padding and
unused art, so the report also lists the largest unread stretches.

Two things follow that are worth a spike each:

- **Unread models can be found.** A model ends at a marker, so an unread
  stretch of polygon ROM can be parsed as models from its start. Whether
  the next model begins straight after the last is not known.
- **Unread models could be drawn** in a viewer without the game ever
  showing them. Their shapes need nothing else. Their textures need the
  right sheet loaded, which only the game does.

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

An agent sits on top for what the crawler cannot do: read a new screen,
recognise "this is a course select with three entries", and decide what
is worth time. It is shown new screens only, one thumbnail each.

A tour that uses pokes is marked, and so is any state made from it. Tours
are for cataloguing. They never touch play, replays or the benchmark
hashes.

### 3. A pack that copes with a short list

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
  equals the same inputs from power-on.

## Performance

The three runs are 20,557 frames: about six minutes at the game's speed
and about 80 seconds recompiled (worked from the figures above; not
run). The catalogue's and the read maps' own cost is not measured.
Recording which code ran is done by the interpreter only, so the
crawler's finest signal costs half the speed.

## Open questions

- **How much of the polygon ROM do today's runs read?** The first spike.
  If it is 90%, coverage is nearly solved by a few more tours; if 30%,
  the crawler matters.
- **Do tours survive timing fixes?** A corrected emulator may shift
  frames, breaking `from=` and `mark=`. `scene verify` catches it;
  whether marks should be found by screen and not by frame is open.
- **Where 2D patterns come from** is not documented, so tiles have no
  read map yet.
- **Can settings in EEPROM** (difficulty, region, blood) change the art?
  A tour would need to carry them.
- **Is the test switch useful** for any game?
- **How far can the crawler get** past a menu, where reaching new art
  takes skill and not breadth? Pokes are the likely answer.

## MVP

1. **Spike (a day):** read maps for the polygon and texture ROMs, and the
   figure for Top Skater's three runs.
2. `tours.txt` with `from=` and `mark=`, `tour run`, and `coverage`.
3. The player's tour recorder.
4. `ram search` and `poke=`, with the cheats work.
5. **Spike:** the crawler on Top Skater's menus.
