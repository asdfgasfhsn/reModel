// A game page's settings (docs/web.md, "Settings"): what they are, how they
// are read from the URL and from localStorage, and how they merge. No DOM
// here (panel.js draws the panel), so Node can test it.
//
// Precedence, lowest first: the built-in defaults, the defaults for all
// games (stored), this game's own (stored), then the page's URL parameters
// (named as the settings are).
// A game stores only the settings that differ from the defaults under it,
// so changing the defaults for all games reaches every game that hasn't
// set its own. The remapped controls (`controls`) are only ever a game's
// own: what they bind depends on the game.

/// The default mix in dB, sound board and music board:
/// `model2::audio::DEFAULT_MIX` (the game's own level is added to both).
export const MIX = [8, -8];

/// Every setting, in panel order.
export const KEYS = ["height", "aspect", "hud", "bezel", "aa", "depth", "shading", "crease", "shadefine", "shadebalance", "pack", "texfilter", "sound", "sfx",
  "music", "buffer", "touch", "tilt", "geo", "smooth", "controls"];
/// Settings kept per game only, never as the defaults for all games:
/// `controls` is the game's remapped keys and pad bindings, as the module
/// keeps them (`romset::bindings`: one `slot = bindings` line per change
/// from the manifest; "" for none). The module parses it and ignores what
/// it can't, so here it only has to be a string of sensible size.
export const PER_GAME = ["controls"];
/// Settings a running game can't change: they apply on a restart.
export const RESTART = ["height", "aspect", "sound", "buffer", "geo", "smooth"];

/// The localStorage key: `{defaults: {...}, games: {set: {...}}, skip:
/// {set: true}}` (skip: don't show the panel before that game starts).
export const STORE = "remodel.settings";

/// The audio queue (ms) by default: the desktop's, and a phone's or
/// tablet's, whose animation frames come less evenly (docs/web.md, "Audio
/// rate").
export const BUFFER = { desktop: 70, phone: 150 };

/// Built-in defaults. `touch` follows the device (`env.touchScreen`);
/// `buffer` "auto" too, when the game starts (`effective`).
export function builtin(env = {}) {
  return {
    height: "auto",
    aspect: "auto",
    hud: "edges",
    bezel: true,
    aa: "off",
    depth: false,
    shading: "flat",
    crease: "auto",
    shadefine: false,
    shadebalance: true,
    pack: false,
    texfilter: "off",
    sound: true,
    sfx: MIX[0],
    music: MIX[1],
    buffer: "auto",
    touch: !!env.touchScreen,
    tilt: false,
    geo: "worker",
    smooth: false,
    controls: "",
  };
}

const ratio = (s) => {
  const m = /^(\d+(?:\.\d+)?):(\d+(?:\.\d+)?)$/.exec(String(s));
  return m && Number(m[2]) > 0 ? Number(m[1]) / Number(m[2]) : NaN;
};

/// `v` as a valid value for setting `key`, or undefined.
export function clean(key, v) {
  switch (key) {
    case "height": {
      if (v === "auto") return v;
      const n = Math.round(Number(v) / 2) * 2;
      return Number.isFinite(n) && n >= 96 && n <= 4320 ? n : undefined;
    }
    case "aspect": {
      if (v === "auto") return v;
      const r = ratio(v);
      return r >= 0.5 && r <= 4 ? String(v) : undefined;
    }
    case "hud":
      return v === "edges" || v === "center" ? v : undefined;
    // Multisampling: WebGPU guarantees 4x only.
    case "aa":
      return v === "off" || v === "4" ? v : v === 0 || v === false ? "off" : v === 4 ? "4" : undefined;
    // Filtering of texture pack images: off, mipmaps, or mipmaps with
    // anisotropic filtering up to 2 to 16 times (gpu::TexFilter).
    case "texfilter":
      if (v === 0 || v === false || v === "0") return "off";
      if (v === 1 || v === "1") return "mip";
      return ["off", "mip", "2", "4", "8", "16"].includes(String(v)) ? String(v) : undefined;
    // Smooth shading (model2::light): flat (the original), per vertex, or
    // per pixel (drawn per vertex with the chip geometrizer).
    case "shading":
      if (v === "off" || v === false || v === "0") return "flat";
      return ["flat", "vertex", "pixel"].includes(v) ? v : undefined;
    // Its crease angle in degrees, or "auto": the game's own.
    case "crease": {
      if (v === "auto") return v;
      const n = typeof v === "number" ? v : typeof v === "string" && /^\d+$/.test(v.trim()) ? Number(v) : NaN;
      return Number.isInteger(n) && n >= 0 && n <= 180 ? n : undefined;
    }
    // Smooth shading's finer levels (gpu::Quality::shading_fine): lit
    // pixels blend two colour columns instead of the game's 64 levels.
    case "shadefine":
    // Smooth shading's brightness balance (gpu::Quality::shading_balance):
    // each smooth surface as bright as the game's; off, bumpy ones brighter.
    case "shadebalance":
    case "depth":
    case "pack":
    case "bezel":
    case "sound":
    case "touch":
    case "tilt":
    case "smooth":
      return typeof v === "boolean" ? v : undefined;
    case "sfx":
    case "music": {
      const n = typeof v === "number" ? v : typeof v === "string" && v.trim() !== "" ? Number(v) : NaN;
      return Number.isFinite(n) ? Math.min(24, Math.max(-40, n)) : undefined;
    }
    case "buffer": {
      if (v === "auto") return v;
      const n = typeof v === "number" ? v : typeof v === "string" && v.trim() !== "" ? Number(v) : NaN;
      return Number.isFinite(n) ? Math.min(1000, Math.max(20, Math.round(n))) : undefined;
    }
    case "geo":
      // "native" was the opt-in before the native geometrizer became the
      // default; "sharc" the chip's name before 2C had one too.
      if (v === "native") return "worker";
      if (v === "sharc" || v === "tgp") return "chip";
      return v === "worker" || v === "local" || v === "chip" ? v : undefined;
    case "controls":
      return typeof v === "string" && v.length <= 16384 ? v : undefined;
  }
  return undefined;
}

/// Only the valid settings in `o`.
function cleanAll(o) {
  const out = {};
  if (o && typeof o === "object") {
    for (const k of KEYS) {
      const v = clean(k, o[k]);
      if (v !== undefined) out[k] = v;
    }
  }
  return out;
}

/// The settings a page's URL parameters give (`?height=1080&sfx=8`...):
/// `height`, `aspect=16:9`, `hud=center`, `bezel=0`, `sound=0`, `sfx` and
/// `music` in dB, `buffer=150` (the audio queue in ms, or `auto`), `touch`
/// (`touch` or `touch=1` on, `touch=0` off), `tilt=1`,
/// `geo=local` (the geometrizer on the page's thread) or `geo=chip` (in its
/// worker, but the game's own program on its chip, the 2B SHARC or 2C TGP,
/// even where the native implementation has it; `geo=sharc` and `geo=tgp`
/// say the same; `geo=native` is the default, kept for old links), `smooth=1`
/// (smooth motion: in-between frames on displays faster than the game),
/// `aa=4` (4x multisampling; `aa=0` or `aa=off` off), `depth=1` (a depth
/// buffer for the 3D layer), `shading=flat|vertex|pixel` (smooth shading;
/// `pixel` draws as `vertex` with `geo=chip`) with `crease=auto|DEGREES`
/// and `shadefine=1` (finer levels: no banding on smooth surfaces) and
/// `shadebalance=0` (no brightness balance per smooth surface), `pack=1`
/// (the game's texture pack, from /texpack/<set>/) and
/// `texfilter=off|mip|2|4|8|16` (filtering of pack images).
export function fromParams(params) {
  const raw = {};
  const flag = (k) => params.get(k) !== "0" && params.get(k) !== "off" && params.get(k) !== "false";
  if (params.has("height")) raw.height = params.get("height");
  if (params.has("aspect")) raw.aspect = params.get("aspect");
  if (params.has("hud")) raw.hud = params.get("hud");
  if (params.has("bezel")) raw.bezel = flag("bezel");
  if (params.has("aa")) raw.aa = flag("aa") ? "4" : "off";
  if (params.has("depth")) raw.depth = flag("depth");
  if (params.has("shading")) raw.shading = params.get("shading");
  if (params.has("crease")) raw.crease = params.get("crease");
  if (params.has("shadefine")) raw.shadefine = flag("shadefine");
  if (params.has("shadebalance")) raw.shadebalance = flag("shadebalance");
  if (params.has("pack")) raw.pack = flag("pack");
  if (params.has("texfilter")) raw.texfilter = params.get("texfilter");
  if (params.has("sound")) raw.sound = flag("sound");
  if (params.has("sfx")) raw.sfx = params.get("sfx");
  if (params.has("music")) raw.music = params.get("music");
  if (params.has("buffer")) raw.buffer = params.get("buffer");
  if (params.has("touch")) raw.touch = flag("touch");
  if (params.has("tilt")) raw.tilt = flag("tilt");
  if (params.has("smooth")) raw.smooth = flag("smooth");
  if (params.has("geo")) raw.geo = ["local", "chip", "sharc", "tgp"].includes(params.get("geo")) ? params.get("geo") : "worker";
  return cleanAll(raw);
}

/// Read the stored settings from `storage` (localStorage, or null): never
/// throws, and anything unreadable counts as nothing stored.
export function readStore(storage) {
  const store = { defaults: {}, games: {}, skip: {} };
  try {
    const s = JSON.parse(storage?.getItem(STORE) ?? "null");
    if (s && typeof s === "object") {
      store.defaults = cleanAll(s.defaults);
      for (const k of PER_GAME) delete store.defaults[k];
      for (const [g, o] of Object.entries(s.games ?? {})) {
        const c = cleanAll(o);
        if (Object.keys(c).length) store.games[g] = c;
      }
      for (const [g, v] of Object.entries(s.skip ?? {})) if (v === true) store.skip[g] = true;
    }
  } catch {}
  return store;
}

/// Store `store`; false if it couldn't be (no storage, full, blocked).
export function writeStore(storage, store) {
  try {
    storage.setItem(STORE, JSON.stringify(store));
    return true;
  } catch {
    return false;
  }
}

/// The settings for `game`: `{values, source}`, `source[key]` saying where
/// each came from: "default", "all" (the stored defaults), "game" or
/// "link" (the URL).
export function resolve(base, store, game, url = {}) {
  const values = { ...base };
  const source = Object.fromEntries(KEYS.map((k) => [k, "default"]));
  for (const [layer, name] of [[store.defaults, "all"], [store.games[game] ?? {}, "game"], [url, "link"]]) {
    for (const k of KEYS) {
      if (k in layer) {
        values[k] = layer[k];
        source[k] = name;
      }
    }
  }
  return { values, source };
}

/// The store after the panel's Play (or Resume): `values` saved for `game`,
/// or as the defaults for all games with `forAll` (removing this game's own
/// copies of them); the keys in `keep` (URL overrides left as they were)
/// aren't saved. `skip`: don't show the panel before this game again.
/// Returns a new store; `store` is unchanged.
export function commit(base, store, game, values, { forAll = false, skip = false, keep = [] } = {}) {
  const out = {
    defaults: { ...store.defaults },
    games: Object.fromEntries(Object.entries(store.games).map(([g, o]) => [g, { ...o }])),
    skip: { ...store.skip },
  };
  const own = out.games[game] ?? {};
  for (const k of KEYS) {
    if (keep.includes(k)) continue;
    const v = clean(k, values[k]);
    if (v === undefined) continue;
    if (forAll && !PER_GAME.includes(k)) {
      if (v === base[k]) delete out.defaults[k];
      else out.defaults[k] = v;
      delete own[k];
    } else {
      const under = k in out.defaults ? out.defaults[k] : base[k];
      if (v === under) delete own[k];
      else own[k] = v;
    }
  }
  if (Object.keys(own).length) out.games[game] = own;
  else delete out.games[game];
  if (skip) out.skip[game] = true;
  else delete out.skip[game];
  return out;
}

/// The screen's render height: the window's height in device pixels, even,
/// up to 1080.
export function screenHeight(env) {
  return Math.min(1080, Math.round((env.innerHeight * env.devicePixelRatio) / 2) * 2);
}

/// The window's shape, kept between 4:3 and 21:9.
export function windowAspect(env) {
  return Math.min(Math.max(env.innerWidth / env.innerHeight, 4 / 3), 21 / 9);
}

/// The audio queue (seconds) for the buffer setting `buffer` on this
/// device (`env.phone`: a phone or tablet), and the worklet's priming and
/// most to suit: priming a little under the target, as on the desktop
/// (60 ms for 70), and at most 250 ms or 8/3 of the target.
export function audioBuffer(buffer, env = {}) {
  const ms = buffer === "auto" ? (env.phone ? BUFFER.phone : BUFFER.desktop) : buffer;
  const target = ms / 1000;
  return { target, prime: (target * 6) / 7, max: Math.max(0.25, (target * 8) / 3) };
}

/// What the page starts the game with: `{height, aspect (width / height),
/// hudEdges, bezel, sound, mix: [sfx, music], audio: {target, prime, max},
/// touch, tilt, geoLocal, geoNative, smooth, aa (samples, 0 or 4), depth,
/// shading ("flat", "vertex", "pixel"), crease (degrees, or undefined: the
/// game's), shadingFine, shadingBalance, pack, texFilter ("off", "mip", "2" ... "16")}`
/// (smooth motion needs the
/// native geometrizer: off with the chip).
export function effective(values, env) {
  return {
    height: values.height === "auto" ? screenHeight(env) : values.height,
    aspect: values.aspect === "auto" ? windowAspect(env) : ratio(values.aspect),
    hudEdges: values.hud !== "center",
    bezel: values.bezel,
    sound: values.sound,
    mix: [values.sfx, values.music],
    audio: audioBuffer(values.buffer, env),
    touch: values.touch,
    tilt: values.tilt,
    geoLocal: values.geo === "local",
    geoNative: values.geo !== "chip",
    smooth: values.smooth && values.geo !== "chip",
    aa: values.aa === "4" ? 4 : 0,
    depth: values.depth,
    shading: values.shading,
    crease: values.crease === "auto" ? undefined : values.crease,
    shadingFine: values.shadefine,
    shadingBalance: values.shadebalance,
    pack: values.pack,
    texFilter: values.texfilter,
  };
}

/// The settings among `keys` whose values differ between `a` and `b`.
export function differ(a, b, keys = KEYS) {
  return keys.filter((k) => a[k] !== b[k]);
}


/// Where the game's bezel goes (docs/web.md, "Bezels"): `{left, top, width,
/// height}` in CSS pixels, or null when it doesn't show. It shows with the
/// bezel setting `on` beside a 4:3 picture (`aspect`, the game's width /
/// height) in a window wider than that; a wide view draws the game there
/// instead, and a window 4:3 or taller has no room. The art (`artWidth` x
/// `artHeight`, its middle 4:3 plain) is as tall as the picture, which
/// fills the window's height, and centred on it.
export function bezelRect(env, aspect, on, artWidth, artHeight) {
  const { innerWidth: w, innerHeight: h } = env;
  if (!on || !(artHeight > 0) || Math.abs(aspect - 4 / 3) > 0.005 || w <= (h * 4) / 3 + 1) return null;
  const width = (h * artWidth) / artHeight;
  return { left: (w - width) / 2, top: 0, width, height: h };
}
