# Browser build

reModel compiled to WebAssembly, running in a browser and drawing with
WebGPU (`crates/web`), with sound. It is playable with keyboard or
gamepad. The sound and music boards run alongside the game
(`model2::audio::Live`, fed each byte the game sends at its exact
sample) in a Web Worker on another core (`crates/web/sound-worker.js`).
Each frame the game posts the bytes it sent on the sound link, with their
CPU cycles. The worker renders the audio and posts it straight to an
AudioWorklet (`crates/web/audio-worklet.js`). The worker shares the
page's compiled WebAssembly module and costs about 2.3 ms of its own core
per frame (Chromium, desktop).
Browsers only start audio after a tap or key press, so the page enables
it on the first one.

**Pacing.** The page follows the display's refreshes from its
animation frames (`model2::pacing::Refresh`). At about 60 Hz the game
runs exactly one frame per refresh; at about 120 Hz, one per two. That is
4.3% fast at 60 Hz, but every refresh shows a new frame. The sound worker
resamples the audio to match. At other rates, frames follow a steady
57.524 Hz clock. The refreshes are counted, not assumed one per animation
frame: each interval between frames is a whole number of refreshes (exact
for frames within a quarter of a refresh of theirs, so a late frame and an
early one after it count two), and the refresh period is the slope of a
least-squares line through the last 256 frames' times against their
refresh numbers, exact to well within their jitter. Once following the
display, the page keeps doing so until the rate is well outside the band
it started in (92% to 114% of the game's speed, against 95% to 110% to
start), so a jittery estimate can't flip it between the two. Frames the
game couldn't run in their refresh (held back by the geometry worker, or
missed in a stall of up to 100 ms) run with the next ones, at most 6
late and 3 at a time.

**Audio rate.** The resampler's step comes from `model2::audio::Pace`,
shared with the native window. While the game keeps up it is the speed
the game is meant to run at (the counted refresh rate following the
display, or 1 on the clock) times a correction from the worklet's queue of
at most 1% either way (17 cents: inaudible, and enough for the drift
between the clocks). Anything more is a gap in the audio, which the
worklet re-primes from, rather than a bend in its pitch. The step moves at
most 2% a second plus 4 times the way it has to go, so noise of a few
cents moves the pitch at most about 35 cents a second.

When the game persistently can't keep up (a geometrizer slower than the
refresh, `sgt24h` at the start of a race), the audio follows the frames
it actually runs, in slow motion and lower in pitch, instead of breaking
up: frames run over frames due, each a rate through two low-pass stages,
at most 1 (catching up never raises the pitch). It starts following once
that pace has been below 97% for 3 times the queue's target (net, so a
pace flickering about the threshold still adds up), stops once it has
been back for as long, and the factor from it is smoothed over 3 times
the target. Frames a stall leaves past what the page may catch up, in a
game that was keeping up, are dropped time, not the game falling behind.

The queue's target is 70 ms on a desktop and 150 ms on a phone or tablet
(a coarse main pointer, or iOS, where every browser is WebKit and an iPad
may say it's a Mac), whose animation frames come less evenly: the
pace's time constants are multiples of it, so the bigger queue also buys
more patience. The worklet primes at 6/7 of the target (60 ms, 129 ms)
and drops audio beyond 250 ms or 8/3 of it (250 ms, 400 ms). The Buffer
setting (below) or `?buffer=` overrides it.

The bar's status line shows the audio rate (100% is the game's own
speed, 104.3% following a 60 Hz display), how far it is from the speed
the game is meant to run at and how much it moved in the last second (in
cents), and the queue against its target; the sound label counts dropouts
(the queue running out while playing) since sound started.
`window.remodelAudio` holds the same numbers for a test driver, with the
buffer in use.

`make pace-sim` (`crates/model2/tests/pace_sim.rs`) models the page:
the frame loop above, the sound worker's step, and the worklet's
128-frame quanta draining at the device's clock (0.03% off the page's),
with level reports every 8 quanta, driven by synthetic animation frames.
It compares the controller before this (an average of frame intervals for
the refresh rate, one frame per animation frame, a pace over 150 ms and a
queue correction of up to 20%, all with the desktop's 70 ms) with the one
now. Pitch is against its mean over the measured part; movement is over
100 ms windows:

| scenario | | max ¢ | rms ¢ | max ¢/s | p99 ¢/s | underruns a minute |
|---|---|---|---|---|---|---|
| steady 60 Hz | before | 15.7 | 4.6 | 117 | 82 | 0 |
| | after | 3.9 | 1.5 | 5 | 3 | 0 |
| phone: frames up to 4 ms early or late, 8% missed, a 50 to 100 ms stall every 2 to 6 s, quanta in fours | before | 914 | 121 | 8834 | 3117 | 5.4 |
| | after | 4.4 | 1.4 | 19 | 7 | 0 |
| 30 Hz (low power mode) | before | 46.5 | 22.5 | 727 | 691 | 0 |
| | after | 2.1 | 0.5 | 2 | 2 | 0 |
| 85% for a minute (sgt24h-like), desktop | before | 32.2 | 9.5 | 357 | 220 | 0 |
| | after | 11.7 | 3.5 | 49 | 47 | 1.2 |
| 80%, 88%, 95% or 100% for 2 to 6 s at a time, desktop | before | 337 | 145 | 1484 | 875 | 0 |
| | after | 236 | 145 | 573 | 452 | 2.7 |
| phone, 85% for a minute | before | 710 | 114 | 5446 | 2153 | 7.2 |
| | after | 158 | 51 | 292 | 271 | 1.2 |

While the game keeps up the pitch now holds within 5 cents and moves
well under 20 cents a second (the phone's wobble before was the refresh
average swinging with the jitter, flipping between following the display
and the clock, and the pace and the queue correction chasing each stall).
A sustained slowdown is still followed, the rate matching the game's to
within 1%; the price is a gap or two as it starts, before the audio has
followed it (the 1.2 a minute above: all at the step down), and on a
desktop a game whose speed jumps by 20% every few seconds breaks up about
once at each jump down. The pitch movement in those rows is the game's
own speed changing. The model can't capture a real browser's audio
thread or the phone's own scheduling; on a phone, the status line shows
what it does there. Confirmed by ear on an iPhone (Chrome, so WebKit) in
`topskatr`, which wobbled before: steady after (2026-11-08).

Before that the rate was the display's speed with only a
0.5% correction, so every frame the game lost to the geometry worker took
17 ms out of the 70 ms queue for good: modelled then from per-frame costs
measured in WebAssembly, a race from frame 2100 to 6600 at 60 Hz ran dry
61 times a minute in `sgt24h` at 97% speed, and 155 times a minute with
costs 20% higher (85%); following the game's pace took that to about 1.

```sh
make web            # out/web: game list, a page per game, one shared interpreter module
make web RECOMP=1   # each game gets its own module with its recompiled code (after `make recomp GAME=...`)
```

**Rebuilding a live site.** The build works in `out/web.next` (starting
from a copy of `out/web`) and swaps each top-level entry into `out/web` at
the end, `model2/` first, so a page loaded mid-build never mixes an old
module with new scripts. It swaps entries rather than the directory because
a server may bind-mount `out/web` (as below); `rom/` is left alone.

**Layout.** `make web` (`tools/web.sh`) writes:

| URL | file | what |
|---|---|---|
| `/` | `index.html` (`crates/web/index.html`) | the game list: no WebAssembly, just `games.json`, the logo, posters and screenshots |
| `/posters/<set>.webp` | `assets/posters/` | the cards' posters (a clone shows its parent's; `placeholder.webp` for a game without one) |
| `/bezels/<set>.webp` | `assets/bezels/` | the bezels beside a 4:3 picture (`games.json` names each game's: its own, else its parent's) |
| `/LICENSE`, `/THIRD-PARTY-NOTICES.txt` | `LICENSE`, `tools/notices.py` | the licence and the crates linked into the module, linked from the landing page's footer |
| `/games.json` | | the games to list (`remodel web-index`) |
| `/model2/<set>/` | `model2/<set>/index.html` (`crates/web/game.html`) | the game's page |
| `/model2/<set>/shot.png` | | its card's screenshot |
| `/model2/<set>/pkg/` | | its WebAssembly module (`RECOMP=1`) |
| `/model2/pkg/` | | the interpreter module the games share (without `RECOMP`, or for a game with no recompiled code) |
| `/model2/*.js` | `crates/web/*.js` | the workers (sound, geometry, texture pack), the audio worklet, the touch controls and the settings panel, shared |
| `/rom/<set>/` | | the ROM files: not written, the server provides them (see Serving). A clone's page fetches from its parent's folder when its chips are there (`overrevb`'s from `rom/overrev/`): `make web` writes that folder into the page's `remodel-rom` meta tag (`remodel --game <set> rom-folder`) |
| `/texpack/<set>/` | | the game's texture pack, if any: its PNGs and an `index.txt` (see Texture packs). Not written, the server provides it; `make web`'s swap leaves `texpack/` alone as it does `rom/`. A clone without a folder of its own uses its parent's |

The game list holds the supported games whose chips are all in
`rom/<set>/` (or, for a clone, its parent's folder) as loose files (the
browser fetches them one by one): a card
each, grouped by board, with its poster (`assets/posters/README.md`), an
attract mode screenshot inset in it (enlarged on hover or focus), and the
title, year and main keys as text.
It remembers the last game played (`localStorage`) and offers to continue
it. The old address `/?game=<set>` (and its other options) redirects to
`/model2/<set>/`. A game page takes its set from its path, links back to
the list (except in fullscreen) and loads the module the `remodel-pkg` meta
tag names (`make web` fills it in); the sound and geometry workers import
the same one.

**Screenshots** are made once, by `remodel --game <set> --height 360
--aspect 4:3 shot FILE`: it boots the game, runs to the manifest's
`attract_frame` with no inputs and saves the screen (software renderer, the
same path as `run`'s snapshots) as a 480x360 PNG, 100-150 KB. Each takes
15-40 s; `make web` makes the missing ones in parallel. Delete a `shot.png`
to remake it; if one fails, its card has no picture.

**Per-game modules.** Each recompiled program is a crate of its own in
`out/recomp/` (docs/recompiler.md). `remodel --game <set> recomp-files`
names the ones that belong to a set (the hash of its program ROM, and of
the copro and geometrizer programs `run` captured in `out/<set>/`), and
`make web RECOMP=1` builds each game in turn from a workspace of its own,
`target/web-recomp/<set>/` (`remodel recomp-workspace --web`), whose
registry lists just those, in one target directory (`target/web`): a
program crate compiles once, and only the registry and `remodel-web`
rebuild between games. Measured here (16 cores):
`topskatr` 62 s (12 MB module), `skisuprg` 110 s (18 MB), `stcc` 120 s
(15 MB), 5 minutes for `make web RECOMP=1` in all, against 44 MB for one
module holding all three before. Without `RECOMP` the shared interpreter
module is 0.8 MB and `make web` takes about half a minute (most of it the
first screenshots).

**Adding a game.** Nothing in the web build names a game: a new manifest
in `crates/romset/src/games/` (with its `year` and `attract_frame`), its
chips in `rom/<set>/`, and `make web` lists it. Make its poster too,
`assets/posters/<set>.webp` (600x900, see `assets/posters/README.md`);
without one the card gets the placeholder and `make web` warns. And its
bezel, `assets/bezels/<set>.webp` (see `assets/bezels/README.md`). For recompiled code, `make
run GAME=<set>` and `make recomp GAME=<set>` first. The touch controls,
help bar and card come from the manifest's controls: buttons can carry a
short `labelled(...)` name for the on-screen buttons and the card.

**Three cores.** The page's thread runs the main CPU, the copro and the
drawing. The geometrizer runs in `crates/web/geo-worker.js`: at each
vblank the game posts the frame's job (a copy of buffer RAM, the read
pointer), and at boot the uploaded program. The worker runs the jobs in
order (`model2::geo::Engine::run_job`) and posts back each frame's raster
stream and texture point RAM. The page draws the newest one, so polygons
can lag the 2D layers by about a frame. Nothing the i960 reads depends on
the geometrizer, so results are the same as with the engine on the page's
thread (`?geo=local` keeps it there), but the page doesn't let the game
get more than two frames ahead of the worker (`GEO_QUEUE`): the worker
answers every job, and with two unanswered the page waits, running the
game slower and saying so on the bar ("waiting for geometry"). Before
that limit, a worker slower than the game fell further behind every
frame: `overrevb`'s interpreted SHARC geometrizer takes 33 ms a race
frame in WebAssembly against the page's 17, so its polygons (the whole
3D scene, steering included) trailed the inputs by half a second more
for every second of racing, and its queued jobs (128 KB each) piled up.
Now the game runs at the worker's pace (about half speed for `overrevb`
in a race) with the polygons at most a few frames behind. If the
worker fails, the page reloads with `?geo=local`. The geometrizer runs on
the native implementation where it implements the program the game
uploads (the 2B and 2C games, `docs/hw/geometrizer-2b.md`,
`docs/hw/geometrizer-2c.md`): the same stream, an order of magnitude
faster than the interpreted chip. `?geo=chip` (or the panel's "game's own
program on its chip") runs the game's own program on the SHARC or TGP in
the worker instead (on the page's thread, `?geo=local`, it's always the
native one); `?geo=sharc` and `?geo=tgp` say the same, and `?geo=native`,
the opt-in before it was the default, still works.

**Toolchain.** It needs the `wasm32-unknown-unknown` target and a
`wasm-bindgen` CLI matching the version in Cargo.lock (`WASM_CARGO`,
`WASM_BINDGEN`). With a system Rust and no rustup, a user-local rustup
works if its proxies stay out of `~/.cargo/bin`. Cargo looks for
subcommands there first, so rustup's `cargo-fmt` and `cargo-clippy`
would shadow the system ones. The Makefile looks in
`~/.local/share/rustup-wasm/bin` first:

```sh
sh rustup-init.sh -y --no-modify-path --profile minimal --target wasm32-unknown-unknown
mkdir -p ~/.local/share/rustup-wasm/bin
mv ~/.cargo/bin/{rustup,cargo,rustc,rustdoc,cargo-fmt,cargo-clippy,clippy-driver,rustfmt} ~/.local/share/rustup-wasm/bin/
RUSTC=$(~/.local/share/rustup-wasm/bin/rustup which rustc) $(~/.local/share/rustup-wasm/bin/rustup which cargo) install wasm-bindgen-cli --version <Cargo.lock's> --locked
```

**Serving.** WebGPU only works on secure pages (HTTPS, or localhost), so
serve over HTTPS. A game page loads its ROM files from `/rom/<set>/` at the
site root (`../../rom/<set>/` from the page), one file per chip, as in the
repo's `rom/topskatr/`. They are your dump: keep the server private to
your network. One setup is a file server
container on a reverse proxy's Docker network:

```sh
mkdir -p out/web/rom
docker run -d --name remodel-web --network PROXY_NET \
  -v "$PWD/out/web:/srv:ro" -v "$PWD/rom:/srv/rom:ro" \
  caddy:2 caddy file-server --root /srv --listen :8080
```

Then reverse-proxy to `remodel-web:8080` from the proxy's HTTPS site. To
restrict by client IP, beware of Docker's IPv6 port forwarding: outside
clients can appear from Docker's own private addresses, so an allowlist
of all private ranges also admits them.

**Settings.** Picking a game shows a settings panel over the page
before the game boots (`crates/web/panel.js`, `crates/web/settings.js`;
plain HTML, no egui). The module and the ROMs download behind it, so Play
starts at once if they've arrived; the panel's last line shows the
progress. It offers:

- **Display:** render height (384, the native height; 768; 1080; or the
  screen's, the default: the window's height in device pixels, up to
  1080), aspect (fit the window, the default, 4:3 to 21:9; 4:3; 16:9),
  HUD placement in wide views (at the edges, or centred), the bezel (on,
  the default, or off; below), anti-aliasing, the depth buffer, shading
  (flat, the default; smooth; per pixel) and its crease angle (the game's,
  the default; 30°, 40°, 60°; below, "Smooth shading"), the texture
  pack (off by default; below) and its filtering (off, mipmaps, 2x to 16x);
- **Sound:** on or off, the effects (sound board) and music levels in
  dB (defaults +8 and -8, `model2::audio::DEFAULT_MIX`; the game's own
  level is added to both), and the buffer: the audio queued (auto, the
  default: 70 ms, or 150 ms on a phone or tablet; 70, 150 or 250 ms;
  above, "Audio rate");
- **Controls:** each input with its keys, gamepad bindings (Xbox names)
  and on-screen control, remappable (below, "Remapping"); on touch
  screens, touch controls on or off and, where an analog follows the left
  stick, tilt steering;
- **Advanced:** the geometrizer in a worker (the default) or on the
  page's thread, or the game's own program on its chip (the 2B SHARC,
  the 2C TGP) in a worker instead of the native geometrizer; and smooth
  motion (off by default; below).

Play (or Enter, or a gamepad's Start, or A on Play) starts the game; the
Play tap also starts the audio and, on iOS, asks for the motion
permission tilt needs (a gamepad press isn't a user gesture, so after a
pad's Start the audio waits for a tap or key). On a gamepad the d-pad or
left stick moves through the panel and changes values. While the panel is
open, keys go to it, not the game. "Don't show this again for this game"
boots that game straight away from then on; the ⚙ button on its card in
the game list (`model2/<set>/#settings`), or on the page before it starts,
brings the panel back.

In game, ⚙ in the bar (or on touch screens beside ⓘ), or Esc, opens the
panel and pauses the game (`set_paused`: no frames run or drawn, the
audio context is suspended, nothing reaches the workers; it resumes from
the same frame). Resume applies the HUD placement (`set_hud`), the bezel, the mix
(a `mix` message to the sound worker), the remapped controls
(`set_bindings`) and touch and tilt at once; render
height, aspect, sound on or off, the sound buffer, the geometrizer and smooth motion need a restart (anti-aliasing, the depth buffer, shading and its crease angle, the texture pack and its filtering apply at once: `set_quality`, `set_pack`), and
the panel offers "Restart to apply" when one of them changed (it reloads
the page with `#play`, skipping the panel once).

Settings are kept in `localStorage` (`remodel.settings`): defaults for
all games, and each game's own, which hold only what differs from those
defaults. The panel saves for the game it's on; "Use these for all games"
makes its values the defaults instead, removing that game's own copies
(except the remapped controls, which are always the game's own).
The page works without storage (private windows, blocked site data): the
panel just starts from the built-in defaults every time. Precedence,
lowest first:

1. the built-in defaults;
2. the stored defaults for all games;
3. the game's stored settings;
4. the page's URL parameters (`/model2/stcc/?aspect=16:9`).

The panel marks a URL override "from link" and doesn't store it, so a
shared link doesn't change anyone's settings; changing it in the panel
drops the parameter from the address and saves the new value. The URL
parameters:

- `height=1080`;
- `aspect=16:9`;
- `hud=center` (or `edges`; at the edges, the 3D parts of a game's HUD move
  with it, as in `docs/play.md`);
- `bezel=0` (no bezel; `bezel=1` on);
- `sound=0` (don't run the sound boards at all);
- `sfx=8&music=-8` (the mix in dB);
- `buffer=150` (the audio queue in ms, 20 to 1000; `buffer=auto` for the
  device's default);
- `touch` or `touch=1` (on-screen controls on any device), `touch=0`
  (off);
- `tilt=1` (tilt steering on from the start, where the game steers);
- `geo=local` (run the geometrizer on the page's thread), or `geo=chip`
  (in its worker, on the game's own program on its SHARC or TGP even where
  the native geometrizer has it; `geo=sharc` and `geo=tgp` too);
- `smooth=1` (smooth motion on; `smooth=0` off);
- `aa=4` (4x multisampling of the 3D layer, the one count WebGPU
  guarantees; `aa=0` or `aa=off` off) and `depth=1` (a depth buffer: polygons
  by depth, not the game's order, except skies, shadows, translucent
  layers and large floors; `docs/enhancements.md`). Both off by default,
  both in the panel's Display section, both applied live. The module's
  `set_quality(aa, depth, texfilter, shading, crease)` takes them before
  or after `start` (`texfilter` is a string: `off`, `mip`, `2`, `4`, `8` or
  `16`; anything else is off; `shading` and `crease` as below, and may be
  left out: flat, the game's angle). Temporal anti-aliasing and upscaling
  are the native player's only (`docs/play.md`); the browser has no
  setting for them;
- `shading=vertex` (smooth shading; `shading=flat` or `off`, the default,
  the original; `shading=pixel`, accepted, draws as `vertex` for now) and
  `crease=40` (degrees, 0 to 180: faces meeting at a sharper angle keep a
  hard edge; 0 smooths nothing; `crease=auto`, the default, is the game's
  own, `romset::Shading`). Both in the panel's Display section, both
  applied live (below, "Smooth shading");
- `pack=1` (the game's texture pack from `/texpack/<set>/`; `pack=0` off) and
  `texfilter=off|mip|2|4|8|16` (filtering of pack images, as in
  `docs/texture-packs.md`). Both off by default, both in the panel's Display
  section, both applied live.

**Smooth shading.** With `shading=vertex` (or Shading: Smooth in the
panel), brightness varies across curved surfaces instead of stepping from
polygon to polygon: the layer `model2::light` computes from the polygon
stream alone (`docs/usage.md`, `--shading`; `docs/play.md`). It runs on
the page, in the renderer's `present_bus` (`Renderer::set_light_rules`
gives it the game's crease angle and HUD windows at `start`), on whatever
stream it draws: the worker's, the page thread's (`geo=local`), the
chip's (`geo=chip`) and smooth motion's in-betweens alike, so the
geometry worker and its messages are unchanged. It combines with
anti-aliasing, the depth buffer and texture packs; WebGPU needs nothing
it didn't already have. With shading on, the status line says the level,
the crease angle and how many of the last frame's polygons were shaded
(`shading vertex 40°, 812/4127 polygons`). `shading_cost(runs)` (in the
page as `remodelShadingCost(runs)`) times the grouping pass on the newest
frame for a test driver: `[median ms, worst ms, polygons, lit]`.

What isn't there yet:

- **Per pixel** (`shading=pixel`) needs the exact layer: the normals and
  lighting the game used, which only the native geometrizer knows. The
  page accepts it, keeps it, and draws it as smooth; the panel and the
  status line say so (`shading pixel (as vertex)`).
- **The game's own lighting data.** In the native player the geometrizer
  will also record, per object and polygon, the light vector, focal
  lengths and normals it lit with (the side channel), which makes smooth
  shading exact. In the browser the geometrizer runs in the worker, so
  the worker will post those words with each frame for the page to
  shade with. Until that exists, the page shades from the stream with a
  guessed focal length, as the native player does today. The planned
  change: the `init` message gains `light` (and a `{type: "set", light}`
  message switches it live), which calls a `GeoEngine::set_light`; each
  `frame` reply gains `light` (a transferred `Uint32Array`, the frame's
  side channel words, `model2::light::Frame::to_words`, about 16 bytes a
  polygon) and, with smooth motion, `betweenLight` (one per in-between);
  `set_geo_frame(stream, points, between, light, between_light)` takes
  them as optional trailing arguments (old pages keep working) and hands
  them to the renderer with the stream it draws. On the page's thread
  (`geo=local`) the renderer reads them from the engine directly.
- **Finer levels** (`shading_fine`, blending the game's 64 brightness
  levels): not built; it will be a sixth, optional `set_quality` argument
  and `?shadefine=1`.

*Measured* (Chrome 157 on Linux, Vulkan, RTX A2000, interpreter build,
1413x1060, the machine shared with other work): `lastbrnx` and
`topskatr` attract mode with `shading=vertex`, alone and with `aa=4`,
`depth=1`, `pack=1&texfilter=mip`, `geo=chip` (`pixel`) and `geo=local`
(`crease=20`), draw without console errors or WebGPU validation
messages, at full speed with the worker; switching live with
`set_quality` works, and the old three-argument call turns shading off.
The grouping pass on the page took 1.0-1.5 ms a frame (median of 30-50
passes, 2,700-3,900 polygons of which 230-530 shaded), 2.7-4.9 ms with
the page thread loaded (`geo=local`, or the chip slowing the game); the
status line's draw time went from 4.3-5.4 ms flat to 5.3-6.3 ms.

**Smooth motion.** With the setting on (`?smooth=1`, or Advanced in the
panel; off by default), a display faster than the game's 57.5 Hz shows
in-between frames: the 3D geometry part way from the frame before to the
newest, the native geometrizer running the newest frame's display list
again with each object's matrix interpolated (`docs/hw/geometrizer.md`,
"Smooth motion"; the native player's `--smooth` is the same, `docs/play.md`).
The 2D layers stay on the game's frames. It adds one game frame (about
17 ms) of latency, and needs the native geometrizer: with the chip
(`geo=chip`) it is off. It works in both geometrizer modes:

- on the page's thread (`geo=local`), each animation frame draws its own
  in-between, at the fraction of a game frame since the newest frame was
  first drawn (`set_smooth` before `start`; the page then redraws every
  animation frame);
- in the worker (the default), the page tells it with each frame job how
  many in-betweens a game frame needs at the measured refresh rate (one
  per refresh it lasts, at most 4: 2 at 120 Hz, 3 at 144 Hz, none at
  60 Hz), the worker draws them after the frame (`GeoEngine::between`, at
  k / n of the way) and sends them with it, and the page draws the one
  nearest each refresh. That costs the worker n geometrizer passes a
  frame whether or not each is shown.

*Unconfirmed:* the module type-checks for WebAssembly, but the browser
build hasn't been run with it, so its cost there (a pass natively is
0.2-0.4 ms for 2C games, 1.4-1.8 ms for 2B ones) and how it looks on a
real 120 or 144 Hz display are untested. `Headless::set_geo_smooth` and
`Headless::between_len` let Node time an in-between.

**Texture packs.** With `pack` on, the page fetches `/texpack/<set>/index.txt`
(`../../texpack/<set>/` from the game page), and for a clone whose folder has
none, its parent's (`/texpack/<parent>/index.txt`; `pack_sets(set)` lists
the folders in order, as the native player looks for its directories): one
file name per line (`.png` optional), `#` comments and blank lines ignored. A browser can't list a
folder and asking for every texture the game draws would be a stream of 404s,
so only names in the index are requested; `make upscale-textures` writes one,
or `(cd DIR && ls *.png > index.txt)` for a hand-made pack. Without it the
original textures stay, with a console warning. The module asks for an
image the first time its texture is drawn (`pack_wanted`); the page has the
pack worker (`pack-worker.js`) fetch and decode it with the module's
`decode_png`, the decoder the native player uses (at most four requests at
once; a 1024x1024 PNG takes 10-50 ms), and hands over at most one decoded
image a frame (`pack_image`; `pack_missing` for one that failed, which then
keeps its original) so uploads don't pile up in one frame: a frame uploading
a 1024x1024 image with its mipmaps measured up to 13 ms of draw against about
3 ms normally. Until an image arrives the original is drawn. The renderer
keeps images up to 192 MB (the least recently drawn are dropped, and
fetched again when next drawn, likely from the HTTP cache). The status line shows
the polygons replaced, the images held and the filter, and with tile images
in the pack the tile patterns replaced and held. `set_pack(index)` takes
the index text (or `undefined` for none) before or after `start`. Tile
images (`tile_<key>.png` in the index) are fetched the same way and drawn
at the canvas's resolution, as natively (`docs/texture-packs.md`, "Tiles
live"); checked on `overrevb` (whose pack came from `overrev`'s folder):
its attract text and logo drawn from 4x tiles, up to 180 patterns a frame,
no WebGPU errors. Details: `docs/texture-packs.md`,
`docs/gpu.md`. Tested in headed Chrome on Linux (headless Chromium loses the
WebGPU device, "Testing headless" below). *Not verified:* other browsers,
phones, `RECOMP=1` builds, eviction past the budget.

**Bezels.** With the 4:3 aspect in a window wider than 4:3, the game's
bezel fills the sides: original cabinet-style artwork, one per game
(`assets/bezels/README.md`), named in `games.json` (its own, else its
parent set's) and loaded the first time it shows. It's an `<img>` behind
the canvas, which is letterboxed by `object-fit: contain`, so it shows
where the canvas draws nothing; it is as tall as the window (the picture's
height) and centred (`settings.js` `bezelRect`, placed again on every
resize). A wide view (the default aspect in a wide window, or 16:9) draws
the game there instead and shows none, nor does a window 4:3 or taller.
It never touches the game: the canvas still gets every pointer event, so
the light guns see the sides as off the screen as before, and the HUD
setting is unchanged. Turning it off or on in the panel applies on
Resume. Checked headless against a stub module (the layout at several
window sizes, the setting live, stored and from the URL; WebGPU isn't
available there), not yet with a game in a real browser.

The bar shows the speed, frames drawn per second, and milliseconds per
frame spent emulating, preparing the frame on the CPU, and submitting it
to the GPU. The sound label adds the worker's time per frame.

**Controls.** Each game's default keys and gamepad bindings come from its
manifest, as in the native player (`docs/play.md`), and the bar lists them
(as remapped).
The touch controls act as a gamepad too, so they drive any game through
its pad bindings, and they are laid out from them (`touch_layout`): a
steering area if an analog control follows the left stick; coin, start and
the shoulder buttons along the top; the face buttons and the analog
triggers as round buttons at the right, the A button big. Each input shows
once, labelled from the manifest. For `topskatr`:

| Action | Keyboard | Gamepad | Touch |
|---|---|---|---|
| Coin | C or 5 | Back | Coin |
| Start | Enter | Start | Start |
| Steer | Left / Right | Left stick or d-pad | drag sideways on the left of the screen, or Tilt |
| Slide | Down or Space | Right trigger or B | Slide (hold) |
| Jump (front) | Z or Up | A | Jump |
| Jump (tail) | X | X | Tail |
| Select | Q / E | Shoulder buttons | ◀ / ▶ |

F toggles fullscreen, H hides the bar and Esc opens the settings.

**Remapping.** In the panel's Controls section, click or tap a binding,
then press the key or gamepad button that should replace it; **+** in the
Keyboard or Gamepad column adds one, **×** removes one, **↺** puts that
input back and "Reset controls to defaults" all of them. Esc cancels
waiting. On a gamepad, move to a binding and press A, release it, then
press the new button (a button already held when it starts waiting
doesn't count until pressed again). Analog inputs have an axis row (move
a stick the way that should read as `+`: moved left or up it binds
inverted, and "invert" flips it; or press a trigger; × leaves no axis) and
`−` and `+` rows for the keys and buttons that hold it at full deflection
(Left and Right for a wheel). Light-gun games keep the mouse or a tap as
the gun ("left click", "right click" in the rows); their trigger and
reload keys are rows like the rest, and the gamepad crosshair's fire and
reload take pad buttons. Inputs the manifest leaves unbound are behind
"Show unbound inputs". A key or button on two inputs is allowed but marked
⚠ (both inputs get it), unless the manifest binds it so itself. Esc, F5,
F9, F11, F and H stay the page's: the panel says so and keeps waiting.

The rules are the native player's (`romset::bindings`, through the
module's `bindings_view`, `bindings_capture` and `bindings_edit`): the
panel only finds out what was pressed. What it stores is the game's
`controls` setting in `remodel.settings`: the same text the native
player keeps in `controls.txt`, only what differs from the manifest, one
line per input (`docs/play.md`, "Remapping"). The module ignores what it
can't parse, so stale or hand-edited storage can't break a game. The
touch controls are laid out from the manifest's pad bindings and keep
going through them, whatever the gamepad is remapped to. Scripted inputs
(`Headless`, `--input`) never use bindings.

Checked headless (Chromium over DevTools, the real module with `start`
stubbed and `navigator.getGamepads` replaced by a scripted pad): keyboard
capture, reserved keys, Esc, conflicts, ×, ↺ and reset, pad buttons, a
stick axis (inverted) and a trigger, unbound inputs, storage (per game
also with "for all games", bad stored values ignored), `set_bindings`
before `start` and on Resume, and the gun games' rows. Not yet tried in a
real browser with a game running or with a real gamepad (whose buttons
and axes a browser may number off the standard layout).

**Light guns** (`hotd`, `bel`; `crates/web/gun.js`). Over the canvas the pointer
is a crosshair: the mouse aims player 1's gun, the left button fires, the
right button reloads, and moving off the picture points off the screen
(where the game reloads). On a touch screen a tap fires at the spot
tapped, and the touch layout is just coin, start and a Reload pill. A
gamepad's sticks move a drawn crosshair for the first gun the page isn't
aiming (player 1 until the mouse or a touch aims it, then player 2); the
right trigger or A fires, the left trigger or B reloads, R reloads from
the keyboard.

Every source reports through one export, `set_gun(player, x, y,
onscreen, buttons)`: x and y are fractions (0 to 1) across and down the
game's 496x384 picture, `onscreen` false means the gun sees no screen,
buttons bit 0 is the trigger and bit 1 reload. `view_point(x, y)` turns
canvas pixels into those fractions (None, an empty array, off the picture,
including a wide view's added sides). Another pointer source, such as a
camera tracking a hand, needs only those two calls.

`bel`'s guns are mounted guns read on analog channels: they can't point
off the screen, so leaving the picture keeps the last aim, and the reload
controls (right button, left trigger or B, the touch pill, labelled
Missile) press the gun's missile button instead (`Reload::Button` in the
manifest).

The touch controls (`crates/web/touch.js`) appear on touch screens, or
anywhere with the touch setting on (`?touch=1`); while they show, the bar
starts hidden, behind the ⓘ button. Steering is analog: full lock 70 CSS pixels from where the
finger went down. Tilt steers by turning the device like a wheel (full
lock at 30 degrees; iOS asks for motion permission).

**Model 2A (`srallyc`).** Its fixed-function geometrizer
(`geo::Core::Fixed`, docs/hw/model2a.md) needs nothing special in the
browser: it has no uploaded program, so it is "booted" from its first frame
(`Geometrizer::booted`, set by `Geometrizer::new` for the core) and the
worker needs no boot job. The page posts the usual frame jobs, the worker's
`GeoEngine` is built from the board (`Core::of`) and runs the native
geometrizer whatever the `native` flag says (`Engine::set_mode` ignores it
for the fixed core); the TGP (the MB86234 interpreter, `tgp2a`) runs on the
i960 side, on the page thread, with the 2A memory map, as it does natively.
The gear lever's latch (`romset::Shifter`) is in the shared input path
(`model2::input::apply_player`), the settings panel lists the gear buttons
(Gear 1-4, Neutral, Gear +, Gear -, all bindable; the last two also on
the pad's shoulder buttons) and the touch layout gives a wheel game's
steering drag, Gas and Brake triggers, View, Gear + and Gear -. Checked
headless: the module (`Headless`, Node 26) runs the game from boot to a
race through the worker's job path with raster streams of up to 35,671
words from frame 1,500 on; and in Chromium, `geo-worker.js` loads the
module and `srallyc`'s chips over HTTP, builds the engine and answers
frame jobs without a boot job. Not checked: a real browser with WebGPU
drawing the game.

Frames 2800-4000 of a coin-start run (Race: from frame 2637), from boot,
all interpreted (`srallyc`'s code runs from work RAM, which the
recompiler doesn't reach, so a recompiled module would give the same;
docs/recompiler.md), sound boards fed as in the table above:

| set | i960 side (with the TGP) | geometrizer | sound boards |
|---|---|---|---|
| `srallyc` | 10.5 ms (worst 13.9) | 0.44 ms (worst 0.6) | 0.95 ms |

Natively the same frames cost 7.3 ms on the i960 side. The page thread has
the i960 side and the drawing (about 6 ms to spare of a 16.7 ms frame
before drawing); the decode cache for code in RAM took it from the 1.4x
real time it had to 2.3x natively.

**Testing headless.** Headless Chromium here can't keep a WebGPU canvas
alive, even for a plain JavaScript test page: the device is lost with "A
valid external Instance reference no longer exists". Test the game in a
real browser. The settings panel and the page around it don't need
WebGPU: headless Chromium driven over its DevTools protocol, with a stub
`remodel_web.js` standing in for the module (exports that record their
calls), can check the panel, storage, URL overrides, pause and the
options `start` gets; `settings.js` has no DOM and runs in Node.

**Timing without a browser.** Headless Chromium has no WebGPU here, so
the module also exports `Headless`: the machine with scripted inputs and
no page, its i960 side (`run_frame`) and geometrizer (`run_geo`) run by
separate calls, and `load` to start from a `remodel run --save-state`
file. Node can time it: import the package's `remodel_web.js`, call its
`default({ module_or_path: <the .wasm bytes> })`, build `new
Headless(set, chips, "coin-start")` from the files `rom_files(set)` names,
and time the two calls per frame with `performance.now()`. Race frames
(from a save at frame 3000 of a coin-start run, 390 frames, Node 26):

| set | i960 side (recompiled) | geometrizer |
|---|---|---|
| `overrev` (2C, TGP recompiled) | 6.8 ms | 8.1 ms |
| `overrevb` (2B, SHARC interpreted) | 9.5 ms | 33.1 ms (worst 36.9) |
| `overrevb` (2B, SHARCs recompiled) | 8.2 ms | 10.7 ms (worst 12.5); with blocks 4.8 ms (below) |

Frame by frame from boot, with the sound boards fed the frame's link
bytes as the sound worker is (`Headless::link_cycles`, `link_bytes`, then
`SoundEngine::feed`), Node 26, frames 3000-6600 of a coin-start run, all
recompiled:

| set | i960 side | geometrizer | sound boards |
|---|---|---|---|
| `sgt24h` | 5.4 ms | 14.9 ms (95th percentile 19.9, worst 23.9; 16% of frames over 16.7) | 1.5 ms |
| `overrevb` | 8.4 ms | 11.6 ms (worst 18.8) | 1.5 ms |
| `overrev` | 6.9 ms | 7.7 ms (worst 13.9) | 1.5 ms |

`sgt24h`'s geometrizer was the limit: from frame 2100 to 3000 (the start
of the race) it averaged 19-25 ms a frame, and 18.5 ms from 6000 to 6600,
so the game couldn't hold a 60 Hz display there. Natively it wasn't much
faster: 18.7 ms a frame from 2100 to 2900. Neither the sound boards nor
the i960 side come close. The mix doesn't clip: the loudest sample in the
race is 5.7 dB under full scale at the default mix with the game's +6 dB.

Since the SHARC programs run as blocks (`docs/recompiler.md`), from a
save at frame 2000 (2,070 frames, Node 26), the geometrizer takes:

| set | before | blocks |
|---|---|---|
| `sgt24h` | 16.5 ms (95th percentile 24.2, worst 46.8; 35% of frames over 17.4) | 8.1 ms (95th percentile 11.8, worst 17.4; none over) |
| `sgt24h`, race start (frames 2000-2899) | | 9.0 ms (95th percentile 11.9) |
| `overrevb` | 10.2 ms (95th percentile 11.7) | 4.8 ms (95th percentile 5.5) |

so `sgt24h` fits a frame (17.4 ms at 57.524 Hz) with room to spare, in a
worker of its own or on the page's thread beside the 5.4 ms i960 side.

Interpreted, about 1.6 times native for the SHARC (20 ms native then);
recompiled about 1.2 times as single steps (9.3 ms native), 1.3 times
with blocks (6.1 ms native for `sgt24h`).

## Ready

- **Renderer.** The GPU rasteriser (`crates/gpu`, see `docs/gpu.md`)
  uses wgpu, which runs on the browser's WebGPU. Its shader reads video
  RAM from storage buffers. WebGPU has those, but WebGL2 doesn't, so
  WebGL2 isn't a target.
- **Single thread.** The geometrizer can run inline instead of on a
  worker thread (`--single-thread`; the default on `wasm32`). The state
  hash is identical either way.
- **No clock needed.** Host timing in the board model goes through a
  stopwatch that measures nothing on `wasm32`, where
  `std::time::Instant` would panic.
- **Frame-sized steps.** `Machine::run_slice` returns at each frame
  boundary, which suits one emulated frame per `requestAnimationFrame`.

## Single-threaded speed (`make bench` run, native)

| build | geometrizer on a thread | inline |
|---|---|---|
| interpreters | 2.2x real time | 1.6x |
| recompiled (`RECOMP=1`) | 4.4x | 3.2x |

Same state hash in all four. WebAssembly usually runs at one half to two
thirds of native speed, so the recompiled build should manage roughly
1.6-2x real time in a browser. The interpreter build, the only one that
could be published (recompiled code is derived from the ROMs), would be
around 1x.

## Left to do

The native player's menu has an HTML counterpart now (the settings panel
above, with pause). Still to come:

- **Save states.** The machine is split three ways: the i960 board on the
  main thread, both sound boards in the sound worker, and (by default) the
  geometrizer in the geometry worker. A save pauses (`set_paused`), asks
  each worker for its state, and joins the replies with the main thread's
  into the format `model2::save_session_parts` writes. Each worker handles
  messages in order, as the native sound thread does, so every part
  reflects the same moment. Loading does the reverse. Saves (about 1.2 MB
  each) go in IndexedDB, with buttons in the panel. The format matches the
  native player's, so a save exported as a download should load in either,
  though that's not yet tested.
- **Games from a file.** The game list shows the sets the server holds.
  A page that doesn't serve ROMs could take a zip from a file picker
  instead (kept in IndexedDB).
- **Controller art** per game (the bezels are done, above): drawn touch
  controls laid out for each game's cabinet, which would also let the
  touch controls sit where they make sense for the game instead of the
  generic layout they get now from the pad bindings.
- Height and aspect changes applied live (reconfiguring the canvas and
  view) instead of on a restart.
- **Run-ahead** (the native player's, `docs/play.md`; `model2::runahead`).
  The second machine copies the game's whole state each frame, the
  geometrizer engine's included, which here lives in the geometry worker
  (remote mode, which `Geometrizer::copy_from` refuses). Either the game's
  engine comes back to the main thread while run-ahead is on (the
  worker then runs only the shadow's frames: post it each run's start
  state, or run the shadow's geometrizer inline too), or the worker
  keeps a shadow engine of its own and copies the game's into it on a
  message after each real frame (`Engine::copy_from` is in-process and
  cheap), the main thread copying the rest. The shadow's i960 frames run
  on the main thread, single-threaded: N more frames each frame needs a
  game with that much headroom in WebAssembly (the 2C games, recompiled).
  Its sound is never posted; drawing switches to its bus and its smooth
  motion record, and the settings panel needs the setting (and the
  game's `input_lag` hint). Copies are memory copies: about 1 ms natively.

Remapping is done (above). Order: save states, then zips from a file,
then controller art for the touch layout.
