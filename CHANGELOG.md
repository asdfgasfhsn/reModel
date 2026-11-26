# Changelog
reModel uses [semantic versioning](https://semver.org/). Before 1.0 a minor
version marks a milestone (new games, boards or features) and a patch
version a fix. Saved formats (save states, the settings file, texture
packs) and the documented `make` targets and CLI may still change between
minor versions; 1.0 comes when they are stable. Versions up to 0.6.0 were
tagged after the fact on the commit that reached each milestone.

Cutting a release: add a `## x.y.z (date): title` section here (the tag's
message is its text), set `version` in the root `Cargo.toml`, commit, then
`make release-tag VERSION=x.y.z`, which checks the tree, runs `make check`,
`make release` and `make bench-all` against `tools/expected-hashes.txt`, and
creates the annotated tag `vx.y.z` without pushing. Details in
[docs/usage.md](docs/usage.md), "Versions and releases".

## Unreleased

- `remodel scene probe NAME --at X,Y[,W,H]` (`make scene-probe`) and the server verbs `scene.probe` and `session.probe`: what draws each game pixel, top to bottom (2D plane, tile cell, `tile_KEY`, region; 3D texture key, palette variant, polygon) and which pack file supplies it, with no GPU (docs/texture-packs.md, "Probing a pixel").
- Player: photo mode's history no longer causes a slow frame every second. Its save state (24 MB in Sega Rally) reuses buffers instead of allocating a new one: 20 ms to 4.5 ms on the game's thread, and no slow redraws in a 1,500-frame `--perf-log` run (they came every 60 frames).
- Depth buffer (`--depth`): the tolerance for a decal behind its surface is 1% of 1/z (was 0.5%), which brings back `segawski`'s jersey logo and lettering where the skier's back turns. Depth off is unchanged.

## 0.16.0 (2026-11-26): live game sessions, per-palette texture packs, sharing audit

- Studio: `studio sharing` audits skin, hair and garments: skin and hair stay the original's, each garment one design. `owners.txt` marks an owner's
  parts by texture key: `NAME.skin` and `NAME.hair` are held to the original through the owner's palette (colour
  difference, flag `skin-off`, `--fixed-limit`), `NAME.GARMENT[=RRGGBB]` is one design whose parts must agree
  in lightness and hue and in the hues their drawings show (a mottled image that averages to the garment's colour is
  caught: `garment:NAME`, `--garment-light`, `--garment-hue`), and between the select screen and the races
  (`garment-select:NAME`, scenes named by `@select|text,..`). `--fix` repaints marked parts all the way,
  keeping the drawing (hue turned, lightness bent); `--recipe FILE` writes style recipes that redraw a garment's
  parts from the original in three tones of its design; `--suggest FILE` drafts the marks from the original's
  colours. The `sharing` gate counts the new flags. Synthetic skin and garment cases are in the tests.
- Studio: `remodel studio sharing STUDIO` (`make studio-sharing`) audits which textures
  share and whether the pack draws their owners consistently (docs/uplift-studio.md,
  "Sharing and owners"). From the project's scenes (their captures, no ROM and no GPU;
  360 Top Skater scenes in 8 s, then 1 s from `STUDIO/sharing/`) it lists, per texture
  key, each palette it is drawn with (scenes, object blocks, pixel share, and what the
  pack draws: variant, plain or original), groups the palettes drawn together on one
  object (a run of object blocks in one 3D window) into owners, named in a small
  `owners.txt` (`NAME|pal,pal,...`) and clustered as `auto-N` otherwise, and compares each
  part's drawn tone (the mean colour of the pack's image) with the original seen through
  its palette, against where its owner's other parts put it. It flags outliers (skin
  12%, other parts 30%), a character's part drawn as the original, and a plain image that
  serves palettes that look different. Output: a TSV, `--json` in the shared answer
  envelope, a contact sheet per owner (the original through the palette next to what the
  pack draws), `--fix DIR` (repainted variants of what is flagged on a character) and
  `--write-map` (a texture map with a row per palette of each shared key, so
  `studio select --key` finds a variant for a palette the project's texmap saw a key
  through once or not at all). `pack check` runs it as the optional `sharing` gate when the
  project has an `owners.txt`.
- Moves (docs/uplift-studio.md, "Moves"): `remodel moves learn GAME --tours FILE...
  [--out FILE]` (`make moves-learn`) cuts recordings' race stretches into the input
  spans that recur (button taps and holds, analog steers by strength and duration,
  and such inputs that overlap), measures each by replaying a window from a spine
  checkpoint with and without it, tags each sample (airborne, speed band, screen)
  and writes a move table; `remodel moves check` compares a table's predictions
  with other recordings. Top Skater's table, learned from the Novice and Expert
  recordings (18 moves, 95 rows; 65 s to learn with the spines made), is
  `crates/romset/src/games/moves/topskatr.txt`. In a session, `session.moves`
  lists the moves and their measured effects under the current conditions, and
  `session.step move=NAME` (repeatable, with `wait-N`) plays them, repeating under
  `until`; `serve --moves FILE` uses another table. Predictions on a held-out
  course are honest: sideways movement and a hop's airtime carry over, speed does
  not (docs/uplift/scenes-and-server.md, "Slice 5 as built").
- Texture packs: per-palette variants. A texture key is the texels and the
  colour-table address, and games rewrite the table (Top Skater's skaters share
  skin textures and get their tone from it), so one key could only have one pack
  image. `tex_<key>~<pal>_<w>x<h>.png` is now the image for that key drawn through
  the 16-colour palette whose id is `<pal>` (the hash of the ramp texmap reports as
  `ramp=`, written `pal=`); it wins over the plain `tex_` image, which stays the
  fallback. Drawn by the CLI's renderers, the player (hot reload included) and the
  browser; no cost for a pack without variants (the palette is worked out only for
  a key that has one: see docs/texture-packs.md, "Per-palette variants", for the
  measurement). `pack-lint` knows the name; texmap and the catalogue report `pal=`
  and name a key drawn through several palettes with a row each; `studio select`
  takes the variants of a key; `scene render --json` has `assets`, which says which
  variant drew. Render only: no state hash changes.
- Live sessions, from the first real use (docs/uplift/session-field-notes.md):
  `session.watch` reports the writes to named values or `ADDR[:LEN]` during a
  step, each with the frame, the size, the value and the i960 instruction that
  wrote (a watch never changes the emulation: the state hashes are the same with
  and without; no measurable cost when none is set, and about 1% with one on the
  interpreter; a watch turns the JIT's fast memory path off, 804 to 580 frames a
  second on `--jit`), and `session.disasm` lists the main CPU's code at an
  address. `session.step follow: tour:LABEL` applies a recording's inputs for
  the frames stepped, with `inputs` added on top as an overlay; `session.open
  overlay` opens a recording with extra held inputs (`jumpfront@2560-3000`) and
  exports as a branch that carries them. Reads take `:float` and `:signed`,
  ranges (`ADDR:LEN`, `ADDR..END`, as a hex dump or typed values), `format`
  `compact` and `tsv`, and `when: before_poke` (a poke applies at the start of
  the frame, so a read right after a step shows the poked value, which the answer
  now says). `until` takes `distance NX,NY,NZ to X,Y,Z lt D`, `changes NAME` and
  `screen NAME` (a game's value table may name its screens:
  `racing|screen=7|note`). `session.list`; `remodel call VERB --help` prints the
  verb's arguments; `--index-every` and `--checkpoint-every` on `serve` (and per
  open) set a spine's granularity; camera `preset=chase|side|top` for
  third-person renders. A test restores after pokes and compares the whole
  machine with a fresh replay: equal but for the renderer's version counters.
- Spines: a person's recording (a tour from power-on) opens fast at any
  frame and can be searched. `remodel spine index LABEL` replays it once and
  keeps a machine state every 600 frames and the game's named values every 30
  under `out/GAME/spines/LABEL/`, keyed by the tour's line (and its `from=`
  parents'), so a changed recording drops them. `session.open from:
  tour:LABEL@F` loads the nearest state and steps the rest: on Top Skater's
  Novice recording (19,368 frames), frame 5,000 opens in 0.9 s instead of
  13.9 s, 10,000 in 1.5 s instead of 31.5 s and 15,000 in 0.1 s instead of
  47 s, to the same state hashes. `remodel spine where LABEL --near pos-x=12
  --where screen=7` (and the server verbs `spine.index` and `spine.where`)
  answers "the frames nearest here" and "the stretches on this screen" from the
  game's value table, each with the `tour:LABEL@FRAME` to open. `--tours FILE`
  can be given more than once, so a branch tour whose spine is another file runs
  in `tour run` and counts in `coverage`; a branch's read map now goes stale
  when its spine's line changes. `make spine-index` and `make spine-where`
  (docs/uplift-studio.md, "Spines").
- `remodel serve`: a long-running server with live game sessions, speaking MCP
  (JSON-RPC) on standard input and output and, with `--socket`, on a Unix
  socket (`out/serve/GAME.sock`, mode 0600; no network port). An agent opens
  a machine of its own (`session.open` from power-on, a scene or a tour at a
  frame), holds inputs, steps it (only then does the game advance), reads
  named values, pokes, keeps and restores marks, moves photo mode's camera,
  draws the frame to a PNG and exports what it did as a branch tour that
  replays to the same state hash; `scene.render` draws a scene's still with
  the GPU kept warm. The verbs are one table, so the MCP tools and the
  command line (`remodel call VERB name=value`) share their arguments and
  answers (the usual envelope). Limits: `--marks`, `--max-step`,
  `--sessions`. `make serve` and `make serve-check` (docs/uplift-studio.md,
  "Live sessions"). The `serve` cargo feature of the CLI crate (on by
  default, unix only) holds it; no emulation or state hash changes.
- `scene render --camera SPEC` (and `make scene-render CAMERA=...`) draws a
  scene from another camera with no ROM and no running game, using photo
  mode's camera words (`orbit`, `tilt`, `yaw`, `pitch`, `roll`, `x`, `y`,
  `z`, `fov`, `near`, `hud`). `scene make` now also keeps the frame's
  display list (`display.bin`, about 4.3 MB) for it; scenes made before
  need `scene make NAME --replace`, and without `--camera` nothing
  changes. Rendering only: the state hashes are the same. Photo mode's
  `--photo` takes `fov` as another name for `zoom`.
- Named game values: a per-game table of work-RAM words found by running the
  game (`crates/romset/src/games/values/GAME.txt`), and `remodel ram values
  TOUR --at F1,F2,...` (`make ram-values`) to read them at frames of a tour
  or scene recipe, with `--json`. Top Skater's table has the screen, course,
  skater, clock, total time, score, speed, position, heading and airborne,
  each read against rendered frames (docs/uplift-studio.md, "Named values").
- `scene render` and `look-render` now draw a pack's tile images (HUD, menus,
  title) and regions as the player does; before they drew the original
  patterns there.

- Looks: three new parameters, rendering only (defaults leave existing looks
  as they were). `outline_silhouette` inks contours wider than texture
  lines, `toon_gain` stretches the light so toon steps show on lit models,
  and `halftone_ramp` swells the dots towards the dark like a printed tone.

## 0.15.0 (2026-11-25): System settings, seats, ROMs found by SHA-1

- ROMs: chips are found by their SHA-1, not only by file name. Sets with
  renamed members, split, merged or non-merged zips, loose chips in any
  folders work, and the launcher lists a game whose chips are there under
  other names. A chip found by name that is the wrong dump is named in a
  warning (the player) or an error (`verify`, `scan`: chip, file, expected
  and found SHA-1), and the right dump is used if it is anywhere in the
  folder. Zips are no longer read whole to look inside them, and hashes are
  cached (`~/.cache/remodel/rom-sha1.tsv`, `REMODEL_ROM_CACHE`). See
  docs/usage.md, "ROM folder layouts".
- Launcher: the home screen's hint bar names **System settings** (Tab, the
  menu button) and **Quit** (Esc, B), both clickable; until now neither had
  anything on screen. Quit opens System settings with Quit reModel chosen, so
  Enter quits and Esc backs out.
- Player: **System settings** and per-game overrides. The launcher's
  Settings (menu button, Tab) now open System settings: Folders (the ROM
  folder, moved here, plus new texture packs and photos folders,
  `packs_root` and `photos_root`, with today's locations as the defaults),
  Picture, Display, Motion, Sound, More, Controllers (the stick, steering and
  pedal tuning and steering smoothing) and a Players placeholder; they edit
  the defaults every game starts from, and each page's footer names the games
  that keep their own values. A game's settings (its page's Game settings,
  the Esc menu) show each row as the system's (dim, "System") or its own
  (cyan dot, the game's name, "System: X" beside it); changing a row writes
  the game's layer only, X or Delete on a row gives it back to the system,
  and the foot counts what differs and resets all. The "Applies to: this game
  / all games" switch, the Picture page's scope switch and "Use the settings
  for all games" are gone: anything changed during play applies to that game
  alone. Old settings files load as they were (a `scope` line is ignored).
- Player: seats. A launcher-level model of who plays P1 to P4 (the keyboard, or a
  gamepad known by its name and UUID, which returns to its seat), kept in
  `seats.txt`, and a **Players** screen to take, move (LB RB, Q E), swap and leave
  seats (Y, Delete), opened from the game page (**Players** for a game with a
  second player's controls, and each count under **Play together**) and from the
  Esc menu mid-game. One cabinet: seat N now plays the game's player-N controls,
  with the game's one keyboard and pad layout (player 2's `punch2`, `start2`,
  `coin2`, ... take player 1's keys and pad buttons), so a second pad plays player
  2 in `vf2`, `doa`, `dynabb97`, `lastbrnx`, `zerogun` and the other two-player
  games, and each pad aims its own gun in the light-gun games. Linked split
  screen: seat N plays machine N, replacing the order of first presses (a first
  press still takes the next free seat; `--link-keyboard off` keeps working). With
  the seats as they start the game takes the usual path and nothing changes;
  state hashes are unchanged.
- Tests: the external-upscaler test stops its stalled tool with `killpg`
  instead of running `kill -KILL -PID`, which on GitHub's Ubuntu runner took
  the runner itself down (every CI run lost, with no log, 2026-11-25). Test
  builds optimise the dependencies, `remodel-state`, `model2` and `gpu`:
  `make test` from about 22 minutes to about 4 here.

## 0.14.0 (2026-11-24): more link games, Zero Gunner, texture pack authoring

- `--record-tour`: a shifter's virtual buttons (neutral, gear up, gear
  down: no port bits) are no longer written as held for the whole run,
  and a gear whose bits hold another's (Daytona's gear 1 holds gear 3's
  bit) is written as itself alone. Tours replayed the same before; the
  lines are now what was played.
- Player: `--scene` and the pack watch (uplift item 8, first two parts).
  `remodel-play --game G --scene NAME` starts from a scene made by `remodel
  scene make` (its state, loaded as a save slot is). The player now watches
  the texture pack folder: a `tex_`, `art_`, `tile_` or material-map PNG that
  is written, replaced or deleted swaps in alone, about 0.15 s after the write
  on Top Skater and `vf2` (the old image shows until the new one has
  decoded); a changed `pack.txt` or region file opens the pack again. Draw
  only; game state hashes are unchanged. Not built: the pack page.

- Draw rules and overlays (uplift item 10, first part): a pack can say an
  image is drawn as painted. `material STEM selflit` on a `tex_` or `art_`
  stem in `pack.txt` skips the game's lighting for that image in the
  software renderer and the GPU (the pack-side form of the look's
  `emissive`; `selflit` is now a shown feature). `--overlay miss` (`scene
  render`, `look-render`, `gpu-check`, snapshots) draws every textured
  polygon with no pack image flat magenta; `scene render --json` adds
  `overlay`, `miss_share` and `miss_pixels`. Draw only; off is the same
  picture bit for bit. Not built: `filter`, `colour`, `@category`, the
  other overlays, `asset set draw=`.

- Context bundles (uplift item 7): `remodel asset bundle ID` (`make
  asset-bundle`) writes an agent's whole briefing for one asset to
  `cache/bundles/ID/` in a project: `bundle.json` (identity and artwork with
  its colourings and colour ramps, size, wrap, cut-out, where it shows in the
  texture map's frames, status, provenance, gates and the output rule), the
  original, a mask of the texels that ever show and two crops of it in
  context with everything else dimmed. `asset show ID` is the same answer
  with no pictures. `ID` is a name, a 16-digit key or a slug; no ROM, no GPU,
  0.3 to 1.0 s on Top Skater, `vf2` and Daytona. `variants.png`, `tiled.png`,
  `brief.txt` and scenes with boxes in game pixels are not built.
- Projects (uplift item 6): a studio workspace is a project that survives
  hand-over. `remodel project init` (`make project-init`) brings a workspace
  up to the manifest, or imports one (`--from-studio DIR`: private binaries,
  runs and scratch are left out, the absolute capture path is dropped, the
  ROM-derived originals and texture map only with `--with-cache`), writing
  `assets.tsv` (status derived from the files: todo, drafted, checked,
  approved, stale when an approved image was edited, rejected), `log.tsv`,
  `provenance/` and `.gitignore`. `project status`, `project approve` (the
  asset gates again; a gate at `error` refuses unless `--override "REASON"`,
  which is logged) and `project reject` use `REMODEL_ACTOR` and the
  project's `approvers`. `remodel pack build` writes the pack to ship, and
  the manifest gains `version`, `author`, `licence`, `ai`, `program`,
  `depends` and `conflicts`; `pack-lint` knows `look.txt`. Packs stack:
  `scene render --pack TOP --pack BELOW` draws the layers (the highest pack
  that has an asset wins) and `remodel pack resolve` says who won what, and
  the stack's memory against the native and browser budgets. Draw-only.
- Texture packs in link play (split screen): each view's renderer loads the
  pack, and a new `link_pack` setting (the Advanced page's **Pack in link
  play**, `--link-pack all|none|N,N..`) chooses which views draw it, so one
  view can be the game as it was drawn and another the pack, live, side by
  side. Draw-only: `make link-pack-test` runs the player with a synthetic
  pack on every view, on one and on none and checks the combined state hash
  equals `link-run`'s each time, and that the picture shows the pack only in
  the views asked for. Over a network each host already drew its own pack.
- Content coverage, the rest of it (uplift item 22). `remodel-play --record-tour
  FILE` (`make tour-record`) writes what you play as a tour line, exact, so it
  replays to the same state. `remodel ram search | show | pokes` (`make
  ram-search`, `ram-pokes`) finds a work-RAM variable by running a tour and
  comparing RAM at frames; a tour holds a value with `poke=ADDR:VALUE[:w]@from-to`
  or a name from the game's table (`crates/romset/src/games/pokes/`; Top Skater's
  course clock is the first), and such a tour, its states and `coverage` are
  marked as cheat runs. Nothing pokes unless a tour says so. `remodel crawl`
  (`make crawl`, `crawl-verify`) explores menus from a point in a tour and keeps
  the tries that read polygon or texture ROM no tour had, as branch tours in
  `out/GAME/crawl/crawl.txt` that replay to the crawler's state hashes; on `vf2`
  200 tries found 17 branches and 0.8 points more of the polygon ROM. Tours now
  apply frame 0's inputs, as the player does. No state hash changes.
- Artwork identity in texture packs: a pack can replace an artwork, not one
  colouring of it. `art_<surface>_<w>x<h>.png` is drawn for every colouring
  (every `tex_` key) of those texels that has no image of its own, in the
  software and GPU renderers and the player (`uses`/`needs artworks`; the
  browser does not read them yet). The catalogue and texture maps record each
  key's 16-colour ramp (`ramp=` in an entry's detail, from the next
  observation on). `remodel studio artworks` lists artworks with their
  colourings and ramps; `studio bake` (`make studio-bake`) writes the other
  colourings of an `art_` image as `tex_` images through the ramps (baked,
  measured by how far the image lies from the ramp it was painted over), or
  leaves them to be drawn as it is (fixed); a rebuild keeps hand-made `tex_`
  images. The `known` and `selected` gates resolve `art_` files and rows by
  surface, and `scene render` reports the share of textured polygon area
  drawn from pack images. Draw-only: no emulation state or hash changes.
- Three more ROM variants, the 2A clones of 2B titles: Zero Gunner on the Model 2A
  board (`zeroguna`, `zerogunaj`) and Pilot Kids on it (`pltkidsa`). They failed
  on a generated manifest (a black 3D layer in play; a stall after the title)
  because the MB86234 core did not know a repeat whose count is in a register
  (`3C04 8034`, bit 15 set), which the TGP programs' bulk-load functions use: the
  chip waited inside the repeat and every later call went unanswered. With the
  count taken from the register they play from a blank EEPROM (benched in
  `make bench-variants`, with idle loops); no other game's hash changed. The
  other five class C clones (the 2B and 2C `dynamcop` clones, which draw no 3D
  yet, and the two Daytona hacks) are in the manifest with a new `plays: false`
  flag: not listed in the launcher or the web build, `--game ID` still takes
  them (`docs/variants.md`, "Class C").
- Steering smoothing: a One Euro filter on pad sticks that steer, so a shaky
  stick no longer twitches the car. The Controls page's Steering block has a
  Smoothing row (Off, Light, Medium, Strong; Light by default; `steer_smooth`,
  per game or for all games); keys, scripted input and aiming sticks are
  untouched. The levels were chosen on a synthetic stick (docs/play.md,
  "Steering smoothing"). `remodel-play --stick-log FILE` (`make stick-log`)
  logs what a pad's sticks read, raw, tuned and smoothed, a line a frame, and
  prints how noisy each is at rest, to measure a real pad.
- Link play for `sgt24h` (Super GT 24h) and `overrev` / `overrevb` (Over Rev), two to
  four machines in `link-run`, the split-screen player and over the network, and network
  play and free play for `motoraid` and `waverunr`. LINK MAX (`sgt24h` byte `0x19`,
  `overrev` byte `0x36`) must equal the number of machines, so a game's record can now
  depend on the session's node count (`LinkPlay::node_bytes`). `sgt24h`'s record has a
  byte-sum checksum and a second copy with its marker zeroed, and its FREE PLAY is a
  byte of the backup SRAM, so a settings image can now carry the SRAM after the EEPROM's
  128 bytes. A game whose master never writes `0x1A14002` (`overrev`) no longer makes the
  hub warn: node 0 is the master then. Free play settings found for `motoraid`,
  `waverunr` and `overrev`. `make link-net-test`, `link-free-play-test` and
  `link-record-test` cover the new games.
- Linked play for three more games: Virtual-On (`von`, with `vonj`) links two
  machines in a one-against-one match, Sega Touring Car Championship (`stcc`,
  with `stcca` and `stccb`) up to seven machines in one race, and Ski Super G
  (`skisuprg`) up to four on one slope, in `link-run`, the split-screen
  player and over the network, with free play from each game's own coin
  setting. Virtual-On keeps its link setting in the backup SRAM (a third
  settings store: its record has a table-driven CRC-16 read from the game's
  code, and its blocks are 1,792 bytes, which `remodel-linknet` now carries:
  blocks up to 1,792 bytes, datagrams up to 2,000). STCC's `SEGASTCb`
  record's checksum was found by running the game and trying the known
  algorithms (CCITT, start `0xFFFF`, complemented, over bytes `0x10`-`0x7F`;
  confirmed against five records the game wrote), and its clones have their
  own records. Ski Super G's board check reads `0x1A14000` back (write 1,
  read bit 0 set; write 0, read it clear): an idle board fails it, which was the
  survey's "NETWORK BOARD IS NOT EXIST !"; the new `LinkBoard::Latch` answers it
  (`docs/hw/link-board.md` sections 15 to 17, `docs/hw/eeprom-records.md`).

- Link play and free play for four more variants: `daytonas` (Saturn Ads) and
  `daytonase` (Special Edition) up to 8 machines, `indy500d` (Deluxe, Rev A) and
  `indy500to` (older Twin) up to 4, in split screen, `link-run` and over the
  network. Each one's link role, cabinet number and FREE PLAY setting were found
  from its own test menu, and the record romset writes is what the game writes
  (`make link-record-test`): the Daytona sets keep the parent's layout with
  their own version and defaults (byte 0x7C is the record's save counter, not a
  free-play byte), `indy500to` the parent's, and `indy500d` an S32A record of
  44 bytes with the role at 0x28 and the cabinet ID at 0x29. `dayton93` stays
  without link play: its program has no link code (no LINK ID in its menu, no
  access to the link board). `make link-net-test`, `link-free-play-test` and
  `link-record-test` take the variants' ids.
- Quality gates for texture packs (uplift item 4): `remodel asset check`
  (`make asset-check`) runs the asset gates on a studio project's images,
  without a ROM or a GPU: `file`, `alpha` and `stale` are errors (exit 1),
  `alpha.soft`, `seam`, `changed` and `noise` warn, and every result states its
  number, its limit and a fix. `remodel pack check` (`make pack-gates`) adds
  `lint`, `budget`, `known`, `selected` and per-scene `coverage` and writes
  `reports/DATE-VERSION/` with `gates.tsv`, `report.md` and `report.json`
  (`--json` gives the shared answer); limits are named constants taken from a
  calibration on the Top Skater showcase, overridden in the project's
  `gates.txt` with a reason. `studio approve` records each approved file's
  hash for `stale`; `studio status --strict` exits 1 while a selected row is not
  approved; `studio seam` states its limits; `remodel pack-flicker` exits 1 when
  the pixels involved in replacement/original changes per 100 frames pass
  `--limit` (default 5,000, a first guess). The whole showcase's 6,286 files
  take 2.2 s. See docs/uplift-studio.md, "Gates".

## 0.13.1 (2026-11-22): network play over Wi-Fi

- Network play over Wi-Fi: a held frame is tried again every half millisecond
  instead of once per redraw, so a late record runs its frame as soon as it
  lands, not up to a display refresh later (looping a loopback link through a
  proxy with 5-15 ms delay and 1% loss, Daytona, D 2: 9-13 s of waiting a
  minute became 3-5 s). The audio no longer slows with short holds: a hold
  under about 100 ms (6 frames) is left to the sound queue, which a network game
  keeps 80 ms deep (the pitch moved up to 45 cents with every cluster of
  holds, now about 5, and the audio ran dry a few times a minute, now not at
  all). Single-player and same-screen link play are unchanged.
- Network play over Wi-Fi: the input delay D now follows the network. The host
  raises it when frames are held (over about a tenth of four seconds, by it or by
  any player) and lowers it one frame at a time after a long calm spell, between 2
  and 8; it announces each change in its own record and every machine switches at
  the same frame, so the game sees the same link data everywhere (the data
  after a raise repeats the last frame's for D' - D boundaries, after a lower it skips
  frames; `docs/hw/link-board.md` section 12.7). New game setting "Input delay
  (hosting)": Automatic (default) or 1 to 6 frames fixed; `REMODEL_NET_DELAY` still
  pins it; the status line follows D. `link-run --delay-at F:D,...` reproduces a
  session's schedule offline and `link-net` logs it (`--auto-delay`, `--delay-plan`);
  `make link-net-test` checks that the hosts' hashes equal it, and with a player
  leaving with a change in flight. Link protocol 5 (a 0.13.0 peer is refused with the
  version message).
- The network status line reads "Delay 3 frames ... held 12% of frames (last 10 s)".
  `REMODEL_NET_FAULTS` and `REMODEL_NET_LOG` are test hooks for the player's session.

## 0.13.0 (2026-11-22): every ROM variant, the game page, steering

- Steering with a stick no longer jumps. Past the dead zone a stick's reading now
  starts from none at its edge and rises to full at the end of the travel (it used
  to jump from none to the dead zone's size there, 12 of the 80 steps on Daytona's
  wheel, which flicked the car when a thumb was near the edge), for every stick
  and every curve. Steering inputs (the wheel, handlebar, bank or carving of
  Daytona, Sega Rally, Indy 500, STCC, Over Rev, Super GT, Wave Runner, Desert
  Tank, Manx TT, Motor Raid and Top Skater) have their own tuning on the Controls
  page, Steering, with an 8% dead zone and a new Fine curve by default
  (`steer_dead`, `steer_sens`, `steer_curve`); other sticks keep the 15% Stick
  tuning and triggers the Pedals one. Scripted input and the benchmarks are
  unchanged.
- The launcher's game page: a game's poster (A, Enter, a click) opens a full page
  with the live attract mode of the chosen version at the left and a menu at the
  right: Play, Version, Play together (for the games that link; greyed for a
  version with link play off), Saves, Game settings and About this version, each
  opening one level down (B goes back a level at a time). The card on the home
  screen loses its Play, Saves, Settings, Link play and Version buttons and the
  version pills for one line about the player and the game and a dim line with the
  number of versions and the linking; double-click a poster to play at once, Y
  still continues. Every version is listed, hacks and prototypes tagged: the
  "Show hacks and prototypes" setting is gone (an old `show_hacks` line is
  ignored). The network banner opens the game's page at Play together with the join
  to that host. The Game settings rows open the menu's own pages and going back
  returns to the page.
- ROM variants: the other releases of a title (revisions, regions, other boards,
  prototypes, hacks) are thin manifests over the parent's, resolved into a full
  game (`crates/romset/src/variants.rs`, `games/variants.rs`; docs/variants.md).
  `manxttc`, `sfight` and `overrevb` are now variants (their bench hashes are
  unchanged) and nine more were added: `topskatrj`, `topskatru`, `vcopa`,
  `srallycb`, `srallyp`, `daytonat`, `daytonata`, `lastbrnxj`, `lastbrnxu`, each
  booting, attracting and playing from a blank EEPROM (`make bench-variants`,
  `tools/expected-variant-hashes.txt`). A variant id works wherever a set id does
  (`--game`, `make GAME=`). The launcher shows one card per title with a Version
  selector (pills, a button, V; now the game page above) and lists prototypes and hacks only with the new
  "Show hacks and prototypes" setting (since removed); each variant has its own EEPROM, saves,
  settings and bindings, texture packs fall back to the parent's, and link play
  works for the variants whose settings record is the parent's. Save files are
  format 3: they record the variant and another variant's state is refused with a
  message naming both (format 1 and 2 files still load). Scenes and tours are
  no longer inherited from the parent. Chips in subfolders of the parent's zip
  (`topskatrj/epr-19753.15`) are found.
- ROM variants, class B: the fifteen clones whose program moved things are variants
  too: `stcca`, `stccb`, `vf2o`, `vf2a`, `vf2b`, `vstrikro`, `vonj`, `indy500d`,
  `indy500to`, `dayton93`, `daytonas`, `daytonase`, `zerogunj`, `dyndeka2` and `doaa`
  (the 2A clone of the 2B Dead or Alive), each with its chip swaps and what differed:
  idle loops at their new addresses, the protection answers' ROM addresses
  (`dyndeka2`, `doaa`), the Saturn sets' own settings records, `dayton93`'s own
  textures (texture packs do not fall back to the parent's for it) and `indy500to`'s
  own `coin-start` script (the parent's start press came too early). Each boots,
  attracts and plays from a blank EEPROM (`make bench-variants`). Link play is off for
  `indy500d`, `indy500to`, `dayton93`, `daytonas` and `daytonase` (their records are not
  the parent's: docs/variants.md). `remodel run` also writes the original Model 2's I/O
  board record (`iostore.bin`).
- A variant with no settings or controls of its own reads its parent's (read-only) until
  the player changes something for it, which writes the variant's own file. `lastbrnxj`
  and `lastbrnxu` have their own launcher attract frames (the inherited one was a warning
  or logo screen). The launcher's Version button, its hint in the bar and the network
  banner's Join button show no V or J keycap in pad mode.
- Network link play compares a hash over every chip the game loads (and the set's id),
  not the program ROMs alone, in the session agreement and the mDNS record ("different
  ROMs"); two sets that differ in one data, texture or sound chip, or two versions of a
  title, no longer link. Link protocol 4 (a peer of 3 gets the version refusal).
- `make bench` (and `bench-all`, `release-tag`) runs from a blank EEPROM
  (`remodel --fresh-eeprom run`: none loaded or kept), so its state hashes no
  longer depend on an `eeprom.bin` left by an earlier run; a fresh checkout or
  worktree failed 16 of 25 before. `tools/expected-hashes.txt` holds the
  fresh-start hashes (16 changed; checked stable over two runs).
- Linked play for four more games: Manx TT Super Bike (`manxtt`, `manxttc`),
  Indianapolis 500 (`indy500`), Motor Raid (`motoraid`) and Wave Runner
  (`waverunr`) now link two to four machines in `link-run` and the
  split-screen player, and Manx TT and Indy 500 over the network too
  (`make link-net-test` and `make link-free-play-test` cover them, with free
  play from each game's own COIN ASSIGNMENTS setting #27). The games' "S32A"
  settings record, which holds Indy 500's, Motor Raid's and Wave Runner's
  (and Super GT's, Over Rev's and Ski Super G's) link role, now has its
  checksum: found by reading the games' own code, not a CRC-16 but a shifting
  sum with its own start value (`docs/hw/eeprom-records.md`), so a node's
  record is built the way the game writes it, both copies included. New
  `make link-record-test` checks it against records the games save
  themselves, and `remodel link-settings FILE` checks an EEPROM image. Super
  GT, Over Rev and Ski Super G still need their own block layout or start
  (`docs/hw/link-board.md` section 14).

- Network link play: a player who leaves, quits or goes silent no longer ends
  the game for everyone. The players left carry on: they agree on the
  leaver's last frame (the newest of its records any of them has; a player
  missing some gets them relayed from one who has them), and from the next
  frame the hub shows the leaver as a copy of the lowest player still racing
  but for its own identity bytes and its car, which stays where it was, so the
  game's race clock, results and next-game steps keep working (freezing the
  leaver's last block stalls both games, `docs/hw/link-board.md` section
  12.3). The player says "Sam left - racing on" in the status line; the
  last player left plays on alone; the host leaving is no different, with one
  limit: after the master (the host) leaves, Sega Rally cannot start another
  game. A real state mismatch still ends the session. Link protocol 3.
  `remodel link-run --leave NODE:F` runs the same drop in one process (and
  `--ghost` the alternatives), `link-net --starve H:F` is the test hook,
  and `make link-net-test` now kills and quits players mid-race (two and
  three hosts, both games) and checks the survivors' hashes against
  `link-run --leave` for the drop frame they logged.
- Named scenes (uplift): `remodel scene list`, `make`, `verify` and
  `render`. A scene is a recipe (input script and frame; five for Top
  Skater, three for `vf2` in their manifests, more in a project's
  `scenes.txt`) and a local copy made from it (capture, state, hash).
  `scene render` draws a copy from its capture with no boot, ROM or
  emulator, optionally with a texture pack, a crop and against the
  original; `scene verify` catches emulation drift. Make targets
  `scene-list`, `scene-make`, `scene-verify`, `scene-render`, `scene-check`.
  Draw-only: no state hash or save state changes.
- Every scene command shares one answer (`crates/cli/src/answer.rs`):
  `--json` prints `ok`, `code`, `error`, `data`, `warnings`, `next`; exit
  codes 0 done, 1 check failed, 2 bad request (unknown names list near
  matches), 3 missing, 4 internal. `serde_json` is a new dependency.
- `studio compare` no longer reuses an old run's frames silently: it
  refuses a folder that holds one unless `--force`; `--scene` compares from
  scenes with no run (`make studio-compare SCENES=all`).
- Uplift tooling, first part: **tours and ROM read maps**. `remodel tour
  list|run` runs a game's named input scripts from power-on (a known
  EEPROM, with `from=` branches and `mark=` states) while recording which
  words of the polygon ROM and the texture ROM were read, and `remodel
  coverage` reports how much of each the tours have read, by region, with
  the unread stretches by address range. Draw-only: the maps are written to
  and never read back, so state hashes are unchanged (`make
  coverage-check`). Tours for Top Skater, `vf2` and `daytona`; `--json`
  answers in the envelope the uplift specs define. `make tour-list`,
  `tour-run`, `coverage`, `coverage-check`.
- Agent environment (docs/uplift/agent-environment.md): `tools/lock` gives
  builds, test runs, GPU renders and emulation their own locks (`make`
  takes them itself; `make lock-status`, bounded waits with `LOCK_WAIT`,
  a hand-typed `flock out/heavy.lock` still honoured), `make agent-setup`
  sets up a worktree (rom, EEPROMs, scratch folder, shared target dir) and
  `make agent-bin` keeps a pinned read-only binary so an art agent never
  builds. With `REMODEL_SCRATCH` set `remodel run` writes to a private
  folder. `make lock-test` joins `make check`; `make agent-env-test` is the
  acceptance test.
- `remodel img sheet | diff | stats` (no ROM, `--json`): contact sheets,
  pixel differences and image numbers where agents used shell and
  ImageMagick.
- `remodel studio`: strict and scriptable. Unknown subcommands and options
  are errors naming the near matches; nothing overwrites a draft or an
  approved image without `--replace` (`draft --force` is now `--replace`);
  exit codes mean something (0 done, 1 a check failed, 2 a wrong request,
  3 something missing, 4 internal); `--json` prints one object (`ok`,
  `code`, `error`, `data`, `warnings`, `next`).
- Fixed `studio draft --keep-size`, which took the next argument as its
  value, and generated drafts of cut-out textures, which came back opaque:
  they now keep the original's transparent texels.

## 0.12.1 (2026-11-21): network play fixes

- Network link play: the Esc menu no longer pauses a network game (it held
  every machine); the game runs on under it with that player's controls let
  go. Backspace edits the Link play name field instead of closing the panel.
- The launcher listens for games hosted on the network (mDNS) and shows a
  banner ("Sam is hosting Daytona USA · 1/4", Join with a click or J) that
  opens the join page connecting to that host. The join list's rows are two
  lines (name and game; players and address, or why not), no longer
  overlapping.

## 0.12.0 (2026-11-20): linked play

Linked cabinets: Sega Rally and Daytona USA race each other as a row of
linked machines, on one split screen or over a LAN, with free play.

- Link board: the Model 2 link board as the games use it (`docs/hw/link-board.md`),
  stepped in lockstep by a hub. `remodel link-run` runs two to four real
  `srallyc` machines or two to eight `daytona` machines in one process, from
  per-game data (`romset::LinkPlay`); runs are deterministic and a whole set
  saves and loads (`make link-run`, `make link-state-check`).
- Link play on one screen: `make play GAME=srallyc PLAYERS=2` (2 to 4, also
  `daytona`), or the launcher's Link play panel: that many linked machines
  side by side or in quadrants, one controller each, with each set's own
  EEPROMs. The launcher shows Link play in its hint bar and on the pad's View
  button for the games that can.
- Free play for linked games: `srallyc` and `daytona` have FREE PLAY among the
  coin settings of their own test menu (found by driving the menu and diffing
  the settings store; the bytes and checksums are in `docs/games/srallyc.md`
  and `docs/games/daytona.md`), kept as per-game data beside the link play
  settings (`romset::LinkPlay::free_play`). The player has a `free_play`
  setting (link play only by default, always, never; the More tab and the
  launcher, hidden for games without the data) and `--free-play` /
  `--no-free-play`; the host's choice travels to every machine of a network
  game in the master's settings, so the settings hash covers it. `remodel
  link-run` and `link-net` take `--free-play`, and `make link-free-play-test`
  races two machines of each game from start presses only.
- Network link play: hosts are found by mDNS / DNS-SD as well as by broadcast.
  A host in its lobby advertises `_remodel._udp.local.` on its real port with
  the game, the reModel version, the ROM hash the session agreement compares,
  players and free seats, D, the lobby state and its name, updating it as
  players come and go and withdrawing it when the lobby is full, starting
  or closed; the join list merges mDNS and the broadcast (kept as the
  fallback, and used alone, logging once, where multicast fails) and greys out
  hosts that cannot be joined with the reason. Uses the `mdns-sd` crate
  (Apache-2.0 OR MIT, pure Rust; its dependencies are MIT, Apache-2.0,
  BSD-3-Clause or Zlib, so the generated `THIRD-PARTY-NOTICES.txt` picks them
  up with no change to `tools/notices.py`) and shares port 5353 with Avahi.
  Players have user names (the OS login name by default, at most 16
  printable characters; kept in the player settings, edited on the Link play
  page, `--name`): the lobby, the join list, the status line and the
  disconnect and desync messages use them ("Sam left"). The link protocol is
  now 2 (names in the lobby packets, the host's version and ROM hash in the
  discovery answer; an older reModel is refused with a message naming the
  mismatch). Names are display only: state hashes and `link-run` equality are
  unchanged. `remodel link-net --browse [SECS]` / `make link-browse` list the
  hosts headlessly; `make link-net-test` finds a named host over mDNS and
  joins it.

- Linked play over a LAN: each machine runs its own linked cabinet (Sega Rally,
  Daytona USA) and the link board's blocks travel between the hosts over UDP,
  in lockstep with a fixed delay D (new crate `remodel-linknet`; no new
  dependencies). One machine hosts and the others join by `IP:port` (or
  pick a host found by LAN broadcast); the host picks D from the measured
  round trip. `remodel-play --game srallyc --host` / `--join HOST:PORT`, and
  the launcher's Link play panel (Host or Join a network game, with the
  address and the players listed, a status line while playing). The hub
  now gives each node a view of the others' records D - 1 frames late
  (ready byte and IDs included), so a node is the same machine over a
  network and in `remodel link-run` with the same inputs and D (D = 1:
  unchanged hashes); `link-run` saves are now `LINKRUN-SAVE-2`. Whole-block
  packets with acknowledgements and redundancy survive 5% loss and 50 ms
  jitter with only stalls; state hashes are exchanged every 60 frames
  and a mismatch stops the session with the frame and the hashes; a silent
  or departed player ends it with a message. Save states are off in a
  network game. Headless `remodel link-net` with fault injection and
  `make link-net-test` (both games over loopback, each host's hash against
  `link-run`'s). `docs/hw/link-board.md` sections 12 and 13: the design as
  built and what internet play would need.
- Linked Daytona draws its 3D scene in a straight run. The original Model 2's
  fixed-function geometrizer hung for good on a display list the game was
  still writing at a vblank (the link check's frames); it now ends that
  frame's list there and carries on. The 2B and 2C native geometrizers'
  hang (a failed command check, a command the program lacks) was not in
  the save state, so a load silently un-hung them; it is now saved (a
  trailing record only present in a halted state: the save format number
  is unchanged and older states load as before). New `make state-roundtrip`
  and `make link-state-check`.

- Texture-pack and look tooling: a live texture-ID pass, ink outlines and toon
  lighting (the Comic picture style, `make look-render`); 2D region
  overrides and the studio's project, batch and region commands; a
  per-frame pack diagnostic and flicker report with stand-ins, holds and
  preloading so replacements stop coming and going; and the catalogue
  (`--catalogue`), which records every texture, tile, layout and object a
  game shows.
- A hidden player window no longer leaks shared memory.
- `make check GPU_TESTS=0` skips the GPU tests with a warning.

## 0.11.1 (2026-11-18): Virtua Striker at its own speed, and fixes

- Virtua Striker ran its match about twice too fast: it sets the board's
  30 Hz mode (`0x98000C` bit 0), which the Model 2B board model ignored.
  Its logic and the geometrizer now step every second frame; a 120 s half
  lasts 125 s (the 4% is display sync; Exact arcade speed removes it).
  Smooth motion has nothing to add for it yet (tabled). Save states made
  before keep the old rate.
- The CRT look no longer brightens the picture (a mid grey was 12.6%
  brighter, highlights up to 60% and clipped); a test holds it.
- Launcher previews run on the JIT, as games do; display sync and clock
  pacing switch with hysteresis instead of flapping, and a late redraw
  catches up at most 2 frames.
- Every `remodel` and `remodel-play` run started by the Makefile is under a
  memory cap (`tools/capped`, `MEMCAP`, default 8G) where systemd is
  available.

## 0.11.0 (2026-11-17): analog sticks as levers

- A gamepad's analog sticks can stand for a game's digital lever: eight
  new bindable stick directions (8-way, a push of at least 0.5 or the
  dead zone, with hysteresis on push and angle so an edge doesn't
  chatter), bindable on the Controls page by moving the stick, in the
  native player and the browser.
- Every game whose stick is digital (the fighters, the shooters) now takes
  the left stick as well as the d-pad by default; games that steer or aim
  with the left stick keep it as an axis, and remapped buttons are left as
  the player set them.
- Cyber Troopers Virtual-On: the left lever on the left stick (and d-pad),
  the right lever on the right stick; shots on the shoulders, dashes on
  the triggers, with face-button alternates.
- Settings files from 0.10.0 load unchanged; save states stay at format 2.

## 0.10.0 (2026-11-17): a new look, release builds, and two more games

- The player's interface, redone: a shared theme; the Esc menu is a side
  panel with save-slot thumbnails; picture styles (Arcade original, Sharp,
  Smooth modern, CRT, Custom) apply live, with a Compare wipe and an
  Advanced page; the launcher is a carousel or a grid with quick resume,
  saves and filters; a Controls page with a picture of the device and
  stick tuning; photo mode's controls are a dock along the bottom edge.
  Sized for 720p up, and usable from a pad or the keyboard throughout.
- Photo mode: no MSAA on top of supersampling, its textures kept between
  frames, and a fallback to a smaller size when the GPU runs out of
  memory, instead of a failed photo (a 4K photo now draws at 7680x4320).
- The JITs are on by default, and Model 2B's SHARCs have one too (copro
  and geometrizer); the i960's plain memory accesses are inline.
- Model 2B: Rail Chase 2 (its ride drive board's network check answered)
  and Cyber Troopers Virtual-On (a link board stub, twin levers).
  Thirty-four games in all.
- Release builds on version tags: a Linux AppImage, Windows, and a
  universal macOS app in a .dmg. CI checks every push.
- A session log, `--perf-log`, `--sound-log`, and GPU memory profiling
  for reports; a draw that panics no longer takes the player down.
- Cleanup: the CLI parses its options once into `Opts`; shared copro FIFO
  helpers in the bus; games list their chips with a `rom()` helper.
- Save states stay at format 2: 0.9.0's load.

## 0.9.0 (2026-11-15): three more games

- Model 2A: Dynamite Cop, with its keyed protection device answered from
  the manifest. Model 2B: Zero Gunner and Pilot Kids (Psikyo), on the same
  keyed device; the i960 now keeps its interrupt table and stack from the
  last reset or reinit, which both needed at boot. Thirty-two games in
  all.
- Save states move to format 2 (the i960's interrupt table and stack);
  older states no longer load.
- Cleanup: shared bit and sign-extension helpers, dead code removed,
  Makefile and web build script tidied.

## 0.8.0 (2026-11-15): the JITs, and five more games

- A JIT for every Model 2 CPU but the 2B's SHARCs (opt-in, `--jit`,
  `make play JIT=1`): the i960, the MB86234 (TGP) and the MB86235 (2C copro
  and geometrizer), on Cranelift, exact to the interpreters (same state
  hashes and lockstep). No recompiled build needed; 1.7-1.8x the
  interpreter, faster than the recompiled build.
- The JITs translate on background workers, so a game never waits for
  them, and keep each game's hot blocks between runs to translate at boot
  (`make jit-warm GAME=x` warms the list; `make jit-hitch` measures).
- Model 2A: Virtua Cop 2, Sky Target, Manx TT Superbike (with its twin
  set) and Motor Raid. Model 2B: Gunblade NY.
- Builds and save states carry the reModel version (`--version`, the
  launcher, the menu's About line).
- Photo mode's photos go to `reModel/<set>/` in the system's pictures
  folder.
- `make clean-stale` deletes old builds Cargo leaves in `target/`.

## 0.7.0 (2026-11-14): the original Model 2, and photo mode

- The original Model 2 board and its sound board (68000, two MultiPCMs, an
  OPN2 as the tempo timer): Daytona USA (all three courses), Virtua Cop
  (two light guns) and Desert Tank play.
- Sonic The Fighters (`sfight`), the Japanese Sonic Championship, as a
  clone.
- Photo mode in the native player (F8): freeze, free camera, stills up to
  8K, depth of field, grading, film effects, motion blur and a 10-second
  time scrub, all without touching the game's own state.
- The player draws on its own thread alongside the next frame's emulation:
  run-ahead with heavy picture settings fits a 60 Hz frame.
- Fixes: the MB86234's floats have no IEEE infinities or NaNs; the i960's
  `scaler` is single precision; the MultiPCM's base rate; the original
  board's black pedestal; ramped keyboard steering for Daytona.
- The browser build rebuilds only the modules whose sources changed, and
  `make web GAME=` builds one game.

## 0.6.0 (2026-11-13): Virtua Fighter 2

- Virtua Fighter 2 (`vf2`, Model 2A) plays: the MB86234's circular
  indexed operands, the 2A's 512K-word texture banks, the geometrizer's
  window carried across frames, and the timer read fix (which also fixed
  Fighting Vipers' ponytail).
- Idle-loop skipping for 18 games, exact to the instruction count.

## 0.5.0 (2026-11-13): visual enhancements and the menu redesign

- Smooth shading, material maps and texture packs, depth buffer,
  anti-aliasing and temporal anti-aliasing, smooth-motion in-between
  frames, bloom.
- The in-game menu and the browser's settings panel in tabs, with picture
  presets (Original, Enhanced) and per-game run-ahead caps.

## 0.4.0 (2026-11-09): Model 2A and the TGP

- Sega Rally Championship (`srallyc`) plays: the Model 2A board, the
  MB86234 TGP running the game's uploaded program, the 2A's polygon ROM,
  textures and fixed-function geometrizer.

## 0.3.0 (2026-11-09): the Model 2B games

- Over Rev (2B), Super GT 24h, Dynamite Baseball '97, Dead or Alive,
  Indianapolis 500, Fighting Vipers, Sonic Championship, Virtua Striker and
  Last Bronx on the Model 2B board (two SHARCs), with native geometrizers.
- The launcher with live attract previews, run-ahead, smooth motion,
  bezels, key and gamepad remapping.

## 0.2.0 (2026-11-06): the Model 2C games

- Sega Ski Super G, Sega Touring Car Championship, Sega Water Ski, Wave
  Runner, Over Rev, The House of the Dead (light guns) and Behind Enemy
  Lines join Top Skater on the Model 2C board.
- The browser build: a game list and a page per game, posters.

## 0.1.0 (2026-11-05): Top Skater

- Top Skater (`topskatr`, Model 2C) plays, in a native window and in the
  browser (WebAssembly and WebGPU): the i960 interpreter and recompiler,
  the Model 2C board, the MB86235 geometrizer, live sound.
