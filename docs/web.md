# Browser build

Remodel compiled to WebAssembly, running in a browser and drawing with
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

**Pacing.** The page measures the display's refresh rate from its
animation frames. At about 60 Hz the game runs exactly one frame per
refresh; at about 120 Hz, one per two. That is 4.3% fast at 60 Hz, but
every refresh shows a new frame. The sound worker resamples the audio to
match. At other rates, frames follow a steady 57.524 Hz clock. Drift
against the sound card is corrected by stretching the audio by at most
0.5%, steering the worklet's queue (reported every 23 ms or so) towards
70 ms. The native window paces the same way (`--exact` there forces
clock pacing).

```sh
make web            # interpreter build -> out/web (index.html, the logo, pkg/)
make web RECOMP=1   # with the recompiled game code (after `make recomp`)
```

**Three cores.** The page's thread runs the main CPU, the copro and the
drawing. The geometrizer runs in `crates/web/geo-worker.js`: at each
vblank the game posts the frame's job (a copy of buffer RAM, the read
pointer), and at boot the uploaded program. The worker runs the jobs in
order (`model2::geo::Engine::run_job`) and posts back each frame's raster
stream and texture point RAM. The page draws the newest one, so polygons
can lag the 2D layers by about a frame. The game never waits for the
geometrizer: nothing the i960 reads depends on it, so results are the same
as with the engine on the page's thread. `?geo=local` keeps it there.

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
serve over HTTPS. The page loads the ROM files from `rom/<set>/` next
to `index.html` (`rom/topskatr/` unless `?game=` says otherwise), one file
per chip, as in the repo's `rom/topskatr/`. They are your
dump: keep the server private to your network. One setup is a file server
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

**Page options.** These are URL parameters:

- `?game=skisuprg` (default `topskatr`): the set to play;
- `aspect=16:9` (default: the window's shape, 4:3 to 21:9);
- `height=1080` (default: the screen's, up to 1080);
- `hud=center` (default `edges`);
- `sound=0` (don't run the sound boards at all);
- `sfx=8&music=-8` (the mix in dB: sound board, music board; these are
  the defaults);
- `geo=local` (run the geometrizer on the page's thread).

The bar shows the speed, frames drawn per second, and milliseconds per
frame spent emulating, preparing the frame on the CPU, and submitting it
to the GPU. The sound label adds the worker's time per frame.

**Controls.** Each game's keys and gamepad bindings come from its
manifest, as in the native player (`docs/play.md`); for games other than
`topskatr` the bar lists them. The touch controls act as a gamepad (the
stick, the right trigger and six buttons), so they drive any game through
its pad bindings, but their labels are `topskatr`'s. For `topskatr`:

| Action | Keyboard | Gamepad | Touch |
|---|---|---|---|
| Coin | C or 5 | Back | Coin |
| Start | Enter | Start | Start |
| Steer | Left / Right | Left stick or d-pad | drag sideways on the left of the screen, or Tilt |
| Slide | Down or Space | Right trigger or B | Slide (hold) |
| Jump (front) | Z or Up | A | Jump |
| Jump (tail) | X | X | Tail |
| Select | Q / E | Shoulder buttons | ◀ / ▶ |

F toggles fullscreen and H hides the bar.

The touch controls (`crates/web/touch.js`) appear on touch screens, or
anywhere with `?touch=1`; on touch screens the bar starts hidden, behind
the ⓘ button. Steering is analog: full lock 70 CSS pixels from where the
finger went down. Tilt steers by turning the device like a wheel (full
lock at 30 degrees; iOS asks for motion permission).

**Testing headless.** Headless Chromium here can't keep a WebGPU canvas
alive, even for a plain JavaScript test page: the device is lost with "A
valid external Instance reference no longer exists". Test in a real
browser.

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

The native player's menu, save states and remembered settings
(`docs/play.md`) aren't in the browser build yet. The plan:

- **Menu: an HTML overlay, not egui.** egui runs on WebGPU, but an
  overlay in `index.html` keeps the WebAssembly small, and its sliders and
  text fields work with touch and the on-screen keyboard, where egui's
  text input is weak. Esc opens it; touch screens get a menu button next
  to ⓘ. It drives small `App` exports, like `set_touch` and
  `set_audio_level` do now: pause, mix, HUD placement, exact speed.
- **Settings** kept in `localStorage`. URL parameters still override
  them, so links like `?sfx=8&music=-8` keep working.
- **Save states.** The machine is split three ways: the i960 board on the
  main thread, both sound boards in the sound worker, and (by default) the
  geometrizer in the geometry worker. A save pauses, asks each worker for
  its state, and joins the replies with the main thread's into the format
  `model2::save_session_parts` writes. Each worker handles messages in
  order, as the native sound thread does, so every part reflects the
  same moment. Loading does the reverse. Saves (about 1.2 MB each) go in
  IndexedDB. The format matches the native player's, so a save exported
  as a download should load in either, though that's not yet tested.
- **Game picker.** The page fetches ROMs from its server, so it would
  list the sets the server holds, or take a zip from a file picker (kept
  in IndexedDB) for a page that doesn't serve ROMs. Only useful once a
  second game is supported.

Order: the overlay with settings and pause first, then save states,
then the picker.
