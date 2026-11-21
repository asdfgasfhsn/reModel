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

- ROM variants: the other releases of a title (revisions, regions, other boards,
  prototypes, hacks) are thin manifests over the parent's, resolved into a full
  game (`crates/romset/src/variants.rs`, `games/variants.rs`; docs/variants.md).
  `manxttc`, `sfight` and `overrevb` are now variants (their bench hashes are
  unchanged) and nine more were added: `topskatrj`, `topskatru`, `vcopa`,
  `srallycb`, `srallyp`, `daytonat`, `daytonata`, `lastbrnxj`, `lastbrnxu`, each
  booting, attracting and playing from a blank EEPROM (`make bench-variants`,
  `tools/expected-variant-hashes.txt`). A variant id works wherever a set id does
  (`--game`, `make GAME=`). The launcher shows one card per title with a Version
  selector (pills, a button, V) and lists prototypes and hacks only with the new
  "Show hacks and prototypes" setting; each variant has its own EEPROM, saves,
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
