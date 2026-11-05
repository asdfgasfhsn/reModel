# Browser build

Remodel compiled to WebAssembly, running in a browser and drawing with
WebGPU (`crates/web`). It is playable with keyboard or gamepad. Sound
isn't in yet.

```sh
make web            # interpreter build -> out/web (index.html, pkg/)
make web RECOMP=1   # with the recompiled game code (after `make recomp`)
```

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
serve over HTTPS. The page loads the ROM files from `rom/` next to
`index.html`, one file per chip, as in the repo's `rom/`. They are your
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

- `?aspect=16:9` (default: the window's shape, 4:3 to 21:9);
- `height=1080` (default: the screen's, up to 1080);
- `hud=center` (default `edges`).

**Controls.**

| Action | Keyboard | Gamepad |
|---|---|---|
| Coin | C or 5 | Back |
| Start | Enter | Start |
| Steer | Left / Right | Left stick or d-pad |
| Slide | Down or Space | Right trigger or B |
| Jump (front) | Z or Up | A |
| Jump (tail) | X | X |
| Select | Q / E | Shoulder buttons |

F toggles fullscreen and H hides the bar.

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

- Sound: the sound and music boards running live, with audio through an
  AudioWorklet.
- A ROM picker that keeps ROMs in IndexedDB, for a page that doesn't
  serve them.
- Persistent GPU buffers: each frame still re-creates every buffer and
  uploads all of render RAM.
- Optional: WebAssembly threads for the geometrizer. These need
  cross-origin isolation headers and a nightly `build-std`.
