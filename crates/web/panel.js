// The settings panel (docs/web.md, "Settings"): an HTML overlay shown
// before a game starts, and from the gear button (or Esc) while it runs.
// settings.js says what the settings are and how they're stored; this draws
// them, from the game's manifest data (the module's `bindings_view`,
// `touch_layout` and `game_recommended`) and nothing game-specific.
//
// The settings come in tabs (Display, Picture, Motion, Sound, Controls,
// More), each row "label ◂ value ▸". The Picture tab starts with the preset
// (Original, Sharp, Enhanced, Custom); only Custom lists the picture rows.
// Rows that do nothing as things are set are left out, not greyed. One help
// line at the foot says what the row in focus (or under the pointer) does,
// and "Applies to" there says where the changes are kept.
//
// Keyboard: Tab moves between controls, the arrow keys move (up and down)
// and change (left and right) a row, Space steps on, Q and E switch tabs,
// Enter plays (or resumes), Esc resumes in game. Gamepad: d-pad or left
// stick up and down moves, left and right changes, LB and RB switch tabs, A
// steps a row on, presses the focused button or toggles a box (else plays),
// Start plays, B resumes in game. Keys pressed while the panel is open
// don't reach the game.
//
// Remapping (the Controls tab): click or tap a binding (or +), then
// press a key or gamepad button, or for an axis move a stick or press a
// trigger; Esc cancels. The module does the binding (`bindings_capture`,
// `bindings_edit`: the same rules as the native player) and says what it
// refuses; the panel only finds out what was pressed.

import { PICTURE, PRESETS, RECOMMENDED, RESTART, audioBuffer, differ, presetValues, screenHeight, settle,
  windowAspect } from "./settings.js";

const el = (tag, props = {}, ...children) => {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...children);
  return e;
};

/// The settings tabs: [id, label].
const TABS = [["display", "Display"], ["picture", "Picture"], ["motion", "Motion"], ["sound", "Sound"],
  ["controls", "Controls"], ["more", "More"]];

const PRESET_LABELS = { original: "Original", sharp: "Sharp", enhanced: "Enhanced", custom: "Custom" };

const STYLE = `
#settings { position: fixed; inset: 0; z-index: 10; display: flex; justify-content: center; align-items: flex-start;
  overflow-y: auto; overscroll-behavior: contain; -webkit-overflow-scrolling: touch; touch-action: pan-y;
  background: rgba(0, 0, 0, 0.72); backdrop-filter: blur(3px); -webkit-backdrop-filter: blur(3px);
  padding: max(12px, env(safe-area-inset-top)) max(16px, env(safe-area-inset-right))
    max(16px, env(safe-area-inset-bottom)) max(16px, env(safe-area-inset-left)); box-sizing: border-box; }
#settings[hidden], #settings [hidden] { display: none !important; }
#settings * { box-sizing: border-box; }
#settings .card { width: 100%; max-width: 560px; margin: auto 0; background: #070b12; color: #f2f4f8;
  border: 1px solid #163a44; border-radius: 12px; box-shadow: 0 0 24px rgba(47, 230, 239, 0.18);
  font: 15px/1.45 system-ui, -apple-system, "Segoe UI", sans-serif; }
#settings header { padding: 14px 16px 10px; border-bottom: 3px solid #2477ff; }
#settings .back { float: right; margin: -2px 0 0 12px; padding: 8px 12px; border-radius: 16px; color: #2fe6ef;
  text-decoration: none; font-size: 14px; }
#settings .back:focus-visible { outline: 2px solid #fff; }
#settings h2 { margin: 0; font-size: 19px; font-weight: 800; font-style: italic; }
#settings header p { margin: 2px 0 0; color: #8b93a3; font-size: 13px; }
#settings .tabs { display: flex; gap: 2px; padding: 8px 12px 0; border-bottom: 1px solid #142230; overflow-x: auto; }
#settings button.tab { flex: none; min-height: 40px; padding: 0 14px; border: 0; border-bottom: 3px solid transparent;
  border-radius: 8px 8px 0 0; background: none; color: #8b93a3; font: 600 14px system-ui, sans-serif; cursor: pointer; }
#settings button.tab[aria-selected=true] { color: #fff; border-bottom-color: #2fe6ef; background: rgba(47, 230, 239, 0.08); }
#settings button.tab:focus-visible { outline: 2px solid #fff; outline-offset: -2px; }
#settings .pane { padding: 10px 16px 4px; min-height: 210px; }
#settings .row { margin: 0 0 4px; border: 0; padding: 0; min-width: 0; }
#settings .step { display: flex; align-items: center; gap: 4px; min-height: 44px; padding: 0 4px 0 8px; border-radius: 8px;
  cursor: pointer; }
#settings .step:hover { background: rgba(255, 255, 255, 0.05); }
#settings .step:focus-visible { outline: 2px solid #2fe6ef; outline-offset: -2px; background: rgba(47, 230, 239, 0.08); }
#settings .step .name { flex: 1; font-weight: 600; font-size: 14px; }
#settings .step .value { min-width: 9.5em; text-align: center; font-size: 14px; font-variant-numeric: tabular-nums; }
#settings .step button.arrow { flex: none; min-height: 40px; width: 36px; padding: 0; border: 0; border-radius: 8px;
  background: none; color: #2fe6ef; font: 700 16px system-ui, sans-serif; cursor: pointer; }
#settings .step button.arrow:disabled { color: #3a4656; cursor: default; }
#settings .hint { color: #8b93a3; font-size: 12px; font-weight: 400; }
#settings .link { display: inline-block; margin-left: 6px; padding: 0 6px; border-radius: 8px; background: #3a2a05;
  color: #ffd479; font-size: 11px; font-weight: 700; vertical-align: 1px; }
#settings .check { display: flex; align-items: center; gap: 10px; min-height: 44px; cursor: pointer; }
#settings .check input { width: 22px; height: 22px; margin: 0; accent-color: #2fe6ef; flex: none; }
#settings table { width: 100%; border-collapse: collapse; font-size: 13px; margin-bottom: 12px; }
#settings th { text-align: left; color: #8b93a3; font-weight: 600; padding: 4px 6px 4px 0; }
#settings td { padding: 5px 6px 5px 0; border-top: 1px solid #142230; vertical-align: top; overflow-wrap: anywhere; }
#settings td:first-child { font-weight: 600; white-space: nowrap; }
#settings kbd { display: inline-block; min-width: 1.6em; padding: 0 4px; margin: 0 2px 2px 0; border: 1px solid #2c3a48;
  border-bottom-width: 2px; border-radius: 4px; font: 12px/1.5 ui-monospace, monospace; text-align: center;
  background: #0d141d; color: #f2f4f8; }
#settings .bind { display: inline-flex; align-items: center; margin: 0 4px 4px 0; border: 1px solid #2c3a48;
  border-radius: 6px; background: #0d141d; }
#settings .bind button, #settings .add, #settings .mini { min-height: 32px; padding: 0 8px; border-radius: 6px; border: 0;
  background: none; font: 600 13px system-ui, sans-serif; color: #f2f4f8; }
#settings .bind .x { color: #8b93a3; padding: 0 6px; }
#settings .add, #settings .mini { border: 1px dashed #2c3a48; margin: 0 4px 4px 0; color: #8b93a3; }
#settings .bind.wait { border-color: #ffd479; background: #3a2a05; }
#settings .bind.wait button { color: #ffd479; }
#settings tr.changed td:first-child::after { content: " •"; color: #2fe6ef; }
#settings .warn { color: #ffb347; font-size: 12px; font-weight: 600; }
#settings tr.conflict td { background: rgba(255, 179, 71, 0.07); }
#settings .msg { min-height: 1.3em; margin: 0 0 8px; font-size: 13px; color: #ffd479; }
#settings .ctl-actions { display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 12px; }
#settings .ctl-actions button { min-height: 40px; font-size: 14px; padding: 0 14px; }
#settings footer { padding: 6px 16px 8px; border-top: 1px solid #142230; }
#settings .helpline { min-height: 3.9em; margin: 4px 8px 6px; color: #8b93a3; font-size: 13px; }
/* Play stays in reach on a small screen. */
#settings .actions { position: sticky; bottom: calc(-1 * max(16px, env(safe-area-inset-bottom)));
  padding: 10px 16px calc(14px + max(16px, env(safe-area-inset-bottom)));
  margin-bottom: calc(-1 * max(16px, env(safe-area-inset-bottom))); background: #070b12;
  border-top: 1px solid #142230; border-radius: 0 0 12px 12px; }
#settings .buttons { display: flex; flex-wrap: wrap; gap: 10px; }
#settings button.big { min-height: 48px; padding: 0 20px; border-radius: 24px; font: 700 16px system-ui, sans-serif; cursor: pointer;
  border: 2px solid #2c3a48; background: #0d141d; color: #f2f4f8; }
#settings button.primary { flex: 1; border-color: #2fe6ef; background: rgba(47, 230, 239, 0.18);
  box-shadow: 0 0 12px rgba(47, 230, 239, 0.35); }
#settings button:focus-visible { outline: 2px solid #fff; outline-offset: 2px; }
#settings .status { min-height: 1.4em; margin: 6px 0 0; color: #9ab; font-size: 13px; font-variant-numeric: tabular-nums; }
`;

/// Build the panel (hidden). `opts`: {title, back (the game list's URL), env
/// (window size, touchScreen), base (built-in defaults), values, source
/// (settings.js `resolve`), forAll, skip, onGesture(action, values) (called
/// in the tap or key press that closed the panel, before the promise
/// resolves)}. Returns its controller.
export function createPanel(opts) {
  const { env } = opts;
  document.head.append(el("style", { textContent: STYLE }));
  let values = { ...opts.values };
  const source = { ...opts.source };
  // URL overrides the player has since changed in the panel.
  const changed = new Set();
  // What the game's manifest recommends for Enhanced (setRecommended).
  let rec = RECOMMENDED;
  // Where Play keeps the changes: all games, or this one.
  let forAllValue = !!opts.forAll;
  const root = el("div", { id: "settings", hidden: true });
  root.setAttribute("role", "dialog");
  root.setAttribute("aria-modal", "true");
  root.setAttribute("aria-label", "Settings");
  const rows = {};
  const badge = (key) => (source[key] === "link" ? el("span", { className: "link", textContent: "from link" }) : "");
  // The one help line, for the row in focus or under the pointer.
  const helpLine = el("p", { className: "helpline" });
  helpLine.setAttribute("aria-live", "polite");
  const showHelp = (row) => (helpLine.textContent = row?.dataset?.help ?? "");

  // A "label ◂ value ▸" row. `options()` lists [value, text]; `get` and `set`
  // read and change the value (the setting `key`'s by default). Left and
  // right step to the previous and next choice; a click on the name or the
  // value, Space or A steps on, round to the first after the last.
  const step = (key, label, options, help, { get = () => values[key], set = (v) => update(key, v) } = {}) => {
    const row = el("div", { className: "row step", tabIndex: 0 });
    row.setAttribute("role", "spinbutton");
    row.setAttribute("aria-label", label);
    row.dataset.help = help ?? "";
    const prev = el("button", { type: "button", className: "arrow", textContent: "◂", tabIndex: -1 });
    const next = el("button", { type: "button", className: "arrow", textContent: "▸", tabIndex: -1 });
    prev.setAttribute("aria-label", `${label}: previous`);
    next.setAttribute("aria-label", `${label}: next`);
    const value = el("span", { className: "value" });
    row.append(el("span", { className: "name" }, label, badge(key)), prev, value, next);
    const index = () => Math.max(0, options().findIndex(([v]) => v === get()));
    const move = (d, wrap = false) => {
      const list = options();
      const i = index();
      const j = wrap ? (i + d + list.length) % list.length : Math.min(list.length - 1, Math.max(0, i + d));
      if (j !== i) set(list[j][0]);
    };
    prev.addEventListener("click", (e) => {
      e.stopPropagation();
      move(-1);
    });
    next.addEventListener("click", (e) => {
      e.stopPropagation();
      move(1);
    });
    row.addEventListener("click", () => move(1, true));
    row._move = move;
    const draw = () => {
      const list = options();
      const i = index();
      value.textContent = list[i]?.[1] ?? String(get());
      row.setAttribute("aria-valuetext", value.textContent);
      prev.disabled = i <= 0;
      next.disabled = i >= list.length - 1;
    };
    draw();
    rows[key] = { el: row, draw };
    return row;
  };
  const onOff = (key, label, help) => step(key, label, () => [[false, "Off"], [true, "On"]], help);
  const db = (key, label, help) => step(key, label, () => withValue(key,
    Array.from({ length: 49 }, (_, i) => [i - 24, `${i - 24 > 0 ? "+" : ""}${i - 24} dB`]),
    (v) => `${v > 0 ? "+" : ""}${v} dB`), help);
  // An option for a value the URL gave that the panel doesn't list.
  const withValue = (key, list, label) =>
    list.some(([v]) => v === values[key]) ? list : [...list, [values[key], label(values[key])]];

  // The game's own crease angle for smooth shading, once the module says
  // (setGameCrease).
  let gameCrease = null;
  const height = screenHeight(env);
  const fit = windowAspect(env);
  const pane = (...rowsOfPane) => el("section", { className: "pane", role: "tabpanel" }, ...rowsOfPane);
  const display = pane(
    step("height", "Render height", () => withValue("height",
      [[384, "384 (native)"], [768, "768"], [1080, "1080"], ["auto", `Screen's (${height})`]], (v) => String(v)),
    "How many lines the game is drawn with. More is sharper and costs more; applies on a restart."),
    step("aspect", "Aspect", () => withValue("aspect",
      [["auto", `Fit window (${fit.toFixed(2)}:1)`], ["4:3", "4:3"], ["16:9", "16:9"]], (v) => v),
    "Fit window draws the game wider than its 4:3 frame to fill the window. 4:3 keeps the original shape, with " +
      "the bezel beside it in a wider window. Applies on a restart."),
    step("hud", "HUD position", () => [["edges", "At the edges"], ["center", "Centred"]],
      "In wide views: out at the edges, or together in the middle as in the original."),
    onOff("bezel", "Bezel", "The cabinet's artwork beside a 4:3 picture in a wider window."));
  const picture = pane(
    step("preset", "Preset", () => PRESETS.map((p) => [p, PRESET_LABELS[p]]),
      "Original: the arcade look. Sharp: 4x anti-aliasing and mipmapped filtering. Enhanced: what suits this game, " +
      "with smooth motion. Custom: choose each setting."),
    step("aa", "Anti-aliasing", () => [["off", "Off"], ["4", "4x"]], "Smooths polygon edges."),
    onOff("depth", "Depth buffer", "Polygons that cross sort right. Off draws them in the game's order."),
    step("shading", "Shading", () => [["flat", "Flat"], ["vertex", "Smooth"], ["pixel", "Per pixel"]],
      "Flat is the original. Smooth blends the game's own lighting across curved surfaces, worked out at each " +
      "corner; per pixel works it out for every pixel (with the chip geometrizer it draws as smooth)."),
    step("crease", "Crease angle", () => withValue("crease",
      [["auto", gameCrease === null ? "The game's" : `The game's (${gameCrease}°)`], [30, "30°"], [40, "40°"],
        [60, "60°"]], (v) => `${v}°`),
    "Faces meeting at a sharper angle than this keep a hard edge: boxes and limbs stay faceted, bodies smooth."),
    onOff("shadefine", "Blend brightness levels",
      "No banding on large smooth surfaces. Off keeps the game's 64 levels."),
    onOff("shadebalance", "Brightness balance",
      "Each smooth surface as bright as the game's. Off, bumpy ones come out brighter."),
    onOff("pack", "Texture pack", "Replacement textures from the server's texpack folder for this game, if it has one."),
    onOff("materials", "Material maps",
      "The pack's normal maps: bumps and grooves lit by the game's own lights. Needs the native geometrizer."),
    step("relief", "Material relief", () => withValue("relief",
      [["1", "1"], ["2", "2"], ["4", "4"], ["8", "8"], ["off", "Faithful"]], (v) => `Custom (${v})`),
    "How strongly the bumps shade, 2 by default. Faithful keeps the game's own lighting sum: subtler."),
    step("bloom", "Bloom", () => withValue("bloom",
      [["off", "Off"], ["25", "Light"], ["50", "Medium"], ["100", "Strong"]], (v) => `${v}%`),
    "Bright parts of the 3D picture glow. The HUD is left sharp."));
  const motion = pane(
    onOff("smooth", "Smooth motion", "Draws in-between frames on displays faster than 60 Hz, a frame later."),
    el("p", { className: "hint", textContent: "Exact arcade speed and run-ahead are the native player's " +
      "(docs/play.md)." }));
  const sound = pane(
    onOff("sound", "Sound", "Off starts the game without sound; applies on a restart."),
    db("sfx", "Effects", "The level of the sound board: effects and announcer."),
    db("music", "Music", "The level of the music board."));
  // Controls: filled in once the game's module is loaded (setControls).
  let table = el("p", { className: "hint", textContent: "Loading the game's controls…" });
  const touchRows = el("div");
  if (env.touchScreen || source.touch === "link") {
    touchRows.append(onOff("touch", "Touch controls", "On-screen buttons and sticks for the game's controls."));
  }
  const controls = pane(table, touchRows);
  const autoBuffer = audioBuffer("auto", env).target * 1000;
  const more = pane(
    step("buffer", "Sound buffer", () => withValue("buffer",
      [["auto", `Auto (${autoBuffer} ms)`], [70, "70 ms"], [150, "150 ms"], [250, "250 ms"]], (v) => `${v} ms`),
    "More rides out uneven frames without bending the pitch, but lags more. Applies on a restart."),
    step("geo", "Geometrizer", () => [
      ["worker", "In a worker"],
      ["local", "On the page's thread"],
      ["chip", "The game's own chip (the reference)"],
    ], "Where the game's 3D geometry is worked out. The chip runs the game's own program, slowly, and has no " +
      "smooth motion or lighting options. Applies on a restart."),
    step("texfilter", "Texture filtering", () => [["off", "Off"], ["mip", "Mipmaps"], ["2", "2x"], ["4", "4x"],
      ["8", "8x"], ["16", "16x"]], "Of the texture pack's images: mipmaps, then anisotropic filtering."));
  const panes = { display, picture, motion, sound, controls, more };

  const tabButtons = {};
  const tabs = el("div", { className: "tabs" });
  tabs.setAttribute("role", "tablist");
  for (const [id, label] of TABS) {
    const b = el("button", { type: "button", className: "tab", textContent: label });
    b.setAttribute("role", "tab");
    b.addEventListener("click", () => selectTab(id));
    tabButtons[id] = b;
    tabs.append(b);
  }
  let tab = "display";
  function selectTab(id) {
    tab = id;
    for (const [k] of TABS) {
      panes[k].hidden = k !== id;
      tabButtons[k].setAttribute("aria-selected", String(k === id));
    }
    showHelp(null);
  }
  const tabBy = (d) => selectTab(TABS[(TABS.findIndex(([k]) => k === tab) + d + TABS.length) % TABS.length][0]);

  const skip = el("input", { type: "checkbox", checked: !!opts.skip });
  const play = el("button", { type: "button", className: "big primary", textContent: "Play" });
  const restart = el("button", { type: "button", className: "big", textContent: "Restart to apply", hidden: true });
  const reset = el("button", { type: "button", className: "big", textContent: "Defaults" });
  const note = el("p", { className: "hint", hidden: true,
    textContent: "Render height, aspect, sound on or off, the sound buffer, the geometrizer and smooth motion " +
      "apply on a restart." });
  const status = el("p", { className: "status" });
  const applies = step("forAll", "Applies to", () => [[false, "This game"], [true, "All games"]],
    "Where changes on these tabs are kept in this browser: for this game only, or as the settings of every game " +
      "that has none of its own.", { get: () => forAllValue, set: (v) => { forAllValue = v; rows.forAll.draw(); } });
  const footer = el("footer", {}, applies, helpLine,
    el("label", { className: "check" }, skip, "Don't show this again for this game"), note);
  const actions = el("div", { className: "actions" }, el("div", { className: "buttons" }, reset, restart, play), status);
  const subtitle = el("p", { textContent: "Settings are kept in this browser. The ⚙︎ button opens them again." });
  // The build's version (the page's meta tag, filled in by tools/web.sh).
  const version = document.querySelector('meta[name="remodel-version"]')?.content;
  const about = el("p", { className: "hint", textContent: version && !version.startsWith("@") ? `reModel ${version}` : "" });
  const card = el("div", { className: "card" },
    el("header", {}, el("a", { className: "back", href: opts.back, textContent: "← Games" }),
      el("h2", { textContent: opts.title }), subtitle),
    tabs, display, picture, motion, sound, controls, more, footer, actions, about);
  root.append(card);
  document.body.append(root);
  selectTab(tab);
  // The help line follows the focus and the pointer.
  const rowOf = (e) => e.target?.closest?.(".step, .check");
  card.addEventListener("focusin", (e) => showHelp(rowOf(e)));
  card.addEventListener("pointerover", (e) => rowOf(e) && showHelp(rowOf(e)));

  // In game: the settings the running game started with.
  let running = null;
  let resolveShow = null;
  let mode = "start";

  // A URL override changed in the panel is the player's from then on.
  function touch(key) {
    if (source[key] === "link") {
      changed.add(key);
      source[key] = "panel";
      rows[key]?.el.querySelector(".link")?.remove();
    }
  }
  function update(key, v) {
    values[key] = v;
    touch(key);
    if (key === "preset") {
      // Choosing a preset sets its rows.
      const set = presetValues(v, rec);
      Object.assign(values, set);
      for (const k of Object.keys(set)) touch(k);
    } else if (PICTURE.includes(key)) {
      // Changing a picture row makes a preset Custom, with the rows as they are.
      values = settle(values, rec);
      touch("preset");
    }
    refresh();
  }
  // Show the rows that apply to what's set, and redraw them all.
  function refresh() {
    const show = (k, on) => rows[k] && (rows[k].el.hidden = !on);
    const custom = values.preset === "custom";
    const chip = values.geo === "chip";
    const shaded = values.shading !== "flat";
    for (const k of ["aa", "depth", "shading", "pack", "bloom"]) show(k, custom);
    show("crease", custom && shaded);
    show("shadefine", custom && shaded && values.crease !== 0);
    show("shadebalance", custom && shaded && values.crease !== 0);
    show("materials", custom && values.pack && !chip);
    show("relief", custom && values.pack && values.materials && !chip);
    show("texfilter", values.pack);
    show("smooth", !chip);
    for (const k of ["sfx", "music", "buffer"]) show(k, values.sound);
    show("hud", values.aspect !== "4:3");
    show("bezel", values.aspect === "4:3");
    show("tilt", values.touch);
    for (const r of Object.values(rows)) r.draw();
    const needRestart = running && differ(values, running, RESTART).length > 0;
    restart.hidden = !needRestart;
    note.hidden = mode !== "game";
  }
  reset.addEventListener("click", () => {
    for (const k of Object.keys(opts.base)) {
      if (k === "touch") continue; // the device's choice; Defaults shouldn't hide the controls
      if (k === "controls") continue; // the Controls section has its own reset
      update(k, opts.base[k]);
    }
  });

  const finish = (action) => {
    if (!resolveShow) return;
    capture = null;
    const r = resolveShow;
    resolveShow = null;
    root.hidden = true;
    removeEventListener("keydown", onKey, true);
    // Still in the tap or key press (when there was one): what needs a
    // gesture (audio, iOS motion permission) is asked for here.
    opts.onGesture?.(action, { ...values });
    r({ action, values: { ...values }, changed: new Set(changed), forAll: forAllValue, skip: skip.checked });
  };
  play.addEventListener("click", () => finish(mode === "game" ? "resume" : "play"));
  restart.addEventListener("click", () => finish("restart"));
  root.addEventListener("click", (e) => {
    // A tap outside the card resumes a running game.
    if (e.target === root && mode === "game") finish("resume");
  });

  // Keys go to the panel, not the game (keyup still does, so a key held
  // when it opened is released).
  function onKey(e) {
    e.stopPropagation();
    if (capture) {
      // Remapping: this key binds (Esc cancels).
      e.preventDefault();
      if (e.repeat) return;
      if (e.code === "Escape") endCapture();
      else captured("key", e.code);
      return;
    }
    const a = document.activeElement;
    const onRow = a?.classList?.contains("step") && card.contains(a);
    const plain = !e.ctrlKey && !e.metaKey && !e.altKey;
    if (e.key === "Enter") {
      // The focused button (Defaults, Restart, Play), else Play.
      e.preventDefault();
      (a instanceof HTMLButtonElement && card.contains(a) ? a : play).click();
    } else if (e.key === "Escape" && mode === "game") {
      e.preventDefault();
      finish("resume");
    } else if (onRow && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      e.preventDefault();
      a._move(e.key === "ArrowLeft" ? -1 : 1);
    } else if (onRow && e.key === " ") {
      e.preventDefault();
      a._move(1, true);
    } else if ((e.key === "ArrowUp" || e.key === "ArrowDown") && card.contains(a)) {
      e.preventDefault();
      moveFocus(e.key === "ArrowUp" ? -1 : 1);
    } else if (plain && !e.shiftKey && (e.key === "q" || e.key === "e") && !capture) {
      e.preventDefault();
      tabBy(e.key === "q" ? -1 : 1);
    }
  }

  // Gamepad: polled while open; a button counts when pressed after the
  // panel opened (one held then must be released first).
  const focusables = () => [...card.querySelectorAll(".step, button, input")].filter((e) =>
    !e.closest("[hidden]") && !e.hidden && !e.disabled && !e.classList.contains("arrow") &&
    e.getAttribute("role") !== "tab");
  const moveFocus = (d) => {
    const list = focusables();
    const i = list.indexOf(document.activeElement);
    const next = list[i < 0 ? 0 : (i + d + list.length) % list.length];
    next?.focus();
    next?.scrollIntoView({ block: "nearest" });
  };
  const change = (d) => document.activeElement?._move?.(d);
  let held = null;
  let repeatAt = 0;
  function poll(t) {
    if (!resolveShow) return;
    requestAnimationFrame(poll);
    const pad = [...(navigator.getGamepads?.() ?? [])].find((p) => p);
    if (!pad) return;
    const b = (i) => pad.buttons[i]?.pressed;
    if (capture) {
      padCapture(pad);
      held = null;
      return;
    }
    const x = pad.axes[0] ?? 0;
    const y = pad.axes[1] ?? 0;
    const now = {
      a: b(0), b: b(1), start: b(9), lb: b(4), rb: b(5),
      up: b(12) || y < -0.6, down: b(13) || y > 0.6, left: b(14) || x < -0.6, right: b(15) || x > 0.6,
    };
    if (held === null) held = now;
    const pressed = (k) => now[k] && !held[k];
    // Directions repeat while held.
    const repeat = (k) => pressed(k) || (now[k] && t >= repeatAt);
    if (pressed("start")) play.click();
    else if (pressed("b") && mode === "game") finish("resume");
    else if (pressed("lb")) tabBy(-1);
    else if (pressed("rb")) tabBy(1);
    else if (pressed("a")) {
      const f = document.activeElement;
      if (f?.classList?.contains("step")) f._move(1, true);
      else if (f instanceof HTMLButtonElement && card.contains(f)) f.click();
      else if (f?.type === "checkbox") f.click();
      else play.click();
    } else {
      for (const [k, act] of [["up", () => moveFocus(-1)], ["down", () => moveFocus(1)],
        ["left", () => change(-1)], ["right", () => change(1)]]) {
        if (repeat(k)) {
          act();
          repeatAt = t + (pressed(k) ? 400 : 120);
        }
      }
    }
    held = now;
  }

  // Remapping. `capture`: {slot, index (-1 adds), accepts} while waiting
  // for a key, button or stick; `base` the pad as it was when it started
  // (what's already held or off centre doesn't count).
  let binder = null;
  let touchLayout = {};
  let capture = null;
  let showUnbound = false;
  const msg = el("p", { className: "msg", role: "status" });
  msg.setAttribute("aria-live", "polite");
  const say = (text) => (msg.textContent = text);
  function startCapture(row, index) {
    capture = { slot: row.slot, index, accepts: row.accepts, label: row.label, base: null };
    say(row.accepts === "axis"
      ? `${row.label}: move a stick the way that reads +, or press a trigger… (Esc cancels)`
      : `${row.label}: press a key or gamepad button… (Esc cancels)`);
    drawControls();
  }
  function endCapture(text = "") {
    capture = null;
    say(text);
    drawControls();
  }
  function setOverrides(o) {
    update("controls", o);
    drawControls();
  }
  // What was pressed: the module binds it, or says why not (then keep
  // waiting, for another key).
  function captured(kind, value) {
    const r = binder.capture(values.controls, capture.slot, capture.index, kind, value);
    if (r.error) {
      say(r.error);
      return;
    }
    const label = capture.label;
    capture = null;
    setOverrides(r.overrides);
    say(`${label}: bound.`);
  }
  function padCapture(pad) {
    const now = { buttons: pad.buttons.map((x) => x.pressed || x.value > 0.5), axes: [...pad.axes] };
    const base = capture.base;
    capture.base = now;
    if (!base) return;
    // Standard buttons only (16 and on are the guide button and extras).
    for (let i = 0; i < Math.min(16, now.buttons.length); i++) {
      if (now.buttons[i] && !base.buttons[i]) return captured("button", String(i));
    }
    // A button's list takes a stick pushed a way as a lever's direction; a
    // pad's crosshair (accepts "pad") only buttons.
    if (capture.accepts !== "axis" && capture.accepts !== "inputs") return;
    for (let i = 0; i < Math.min(4, now.axes.length); i++) {
      const v = now.axes[i];
      if (Math.abs(v) > 0.6 && Math.abs(base.axes[i] ?? 0) < 0.3) return captured("axis", `${i}${v < 0 ? "-" : "+"}`);
      // Still near centre: a move from here counts.
      if (Math.abs(v) >= 0.3) now.axes[i] = base.axes[i];
    }
  }
  function drawControls() {
    if (!binder) return;
    const view = binder.view(values.controls);
    const touch = env.touchScreen || values.touch;
    const touchOf = (row) => {
      if (row.accepts === "none") return touchLayout.gun && row.label.startsWith("Aim") ? "tap the picture" : "";
      if (!row.input) return "";
      const items = [...(touchLayout.top ?? []), ...(touchLayout.actions ?? [])];
      if (row.accepts === "axis") {
        const t = items.find((i) => i.trigger && (i.label === row.input || i.label.startsWith(row.input + " ")));
        if (t) return t.label;
        return touchLayout.steer && row.axis.text === "left stick ↔" && row.default ? "drag on the left, or Tilt" : "";
      }
      if (row.label !== row.input) return "";
      return items.find((i) => i.label === row.input && !i.trigger)?.label ?? "";
    };
    const waiting = (row, i) => capture && capture.slot === row.slot && capture.index === i;
    const chip = (row, text, i, removable = true) => {
      const pick = el("button", { type: "button", textContent: waiting(row, i) ? "press…" : text });
      pick.title = "Change: click, then press the new key or button";
      pick.addEventListener("click", () => startCapture(row, i));
      const box = el("span", { className: "bind" + (waiting(row, i) ? " wait" : "") }, pick);
      if (removable) {
        const x = el("button", { type: "button", className: "x", textContent: "×" });
        x.setAttribute("aria-label", `Remove ${text} from ${row.label}`);
        x.addEventListener("click", () => {
          setOverrides(binder.edit(values.controls, "remove", row.slot, i));
          say(`${row.label}: ${text} removed.`);
        });
        box.append(x);
      }
      return box;
    };
    const add = (row) => {
      const b = el("button", { type: "button", className: "add", textContent: waiting(row, -1) ? "press…" : "+" });
      b.setAttribute("aria-label", `Add a binding to ${row.label}`);
      b.addEventListener("click", () => startCapture(row, -1));
      return b;
    };
    const head = el("tr", {}, el("th", { textContent: "" }), el("th", { textContent: "Keyboard" }),
      el("th", { textContent: "Gamepad" }));
    if (touch) head.append(el("th", { textContent: "Touch" }));
    const body = [];
    let hidden = 0;
    for (const row of view.rows) {
      if (row.unbound && !showUnbound && !waiting(row, -1)) {
        hidden++;
        continue;
      }
      const keys = el("td");
      const pad = el("td");
      if (row.fixed) keys.append(row.fixed + " ");
      if (row.accepts === "axis") {
        keys.append(el("span", { className: "hint", textContent: "keys: − and + below" }));
        const none = !row.axis.text;
        pad.append(chip(row, none ? "none" : row.axis.text, -1, !none));
        if (!none) {
          const inv = el("button", { type: "button", className: "mini",
            textContent: row.axis.invert ? "inverted ✓" : "invert" });
          inv.setAttribute("aria-pressed", String(row.axis.invert));
          inv.addEventListener("click", () => setOverrides(binder.edit(values.controls, "invert", row.slot, 0)));
          pad.append(inv);
        }
      } else if (row.accepts !== "none") {
        row.items.forEach((it, i) => (it.token.startsWith("pad:") ? pad : keys).append(chip(row, it.text, i)));
        if (row.accepts === "inputs") keys.append(add(row));
        pad.append(add(row));
      }
      const name = el("td", { textContent: row.label });
      if (row.conflicts?.length) name.append(el("div", { className: "warn", textContent: "⚠ " + row.conflicts.join("; ") }));
      if (row.slot && !row.default) {
        const reset = el("button", { type: "button", className: "mini", textContent: "↺" });
        reset.title = "Back to this input's defaults";
        reset.setAttribute("aria-label", `Reset ${row.label}`);
        reset.addEventListener("click", () => setOverrides(binder.edit(values.controls, "reset", row.slot, 0)));
        name.append(" ", reset);
      }
      const tr = el("tr", { className: [row.slot && !row.default ? "changed" : "",
        row.conflicts?.length ? "conflict" : ""].join(" ").trim() }, name, keys, pad);
      if (touch) tr.append(el("td", { textContent: touchOf(row) }));
      body.push(tr);
    }
    const focused = document.activeElement?.getAttribute?.("aria-label") ?? document.activeElement?.textContent;
    const next = el("div", {});
    if (view.conflicts) {
      next.append(el("p", { className: "warn",
        textContent: `⚠ ${view.conflicts === 1 ? "A key or button is" : `${view.conflicts} keys or buttons are`} ` +
          "bound to two inputs: both get it." }));
    }
    next.append(el("table", {}, el("thead", {}, head), el("tbody", {}, ...body)), msg);
    const actions = el("div", { className: "ctl-actions" });
    if (hidden || showUnbound) {
      const more = el("button", { type: "button",
        textContent: showUnbound ? "Hide unbound inputs" : `Show ${hidden} unbound input${hidden === 1 ? "" : "s"}` });
      more.addEventListener("click", () => {
        showUnbound = !showUnbound;
        drawControls();
      });
      actions.append(more);
    }
    const resetAll = el("button", { type: "button", textContent: "Reset controls to defaults" });
    resetAll.disabled = !values.controls;
    resetAll.addEventListener("click", () => {
      capture = null;
      setOverrides(binder.edit(values.controls, "reset-all", "", 0));
      say("Controls back to the game's defaults.");
    });
    actions.append(resetAll);
    next.append(actions, el("p", { className: "hint",
      textContent: "Click a binding to change it, × to remove it, + to add one. Esc, F5, F9, F11, F and H stay the " +
        "player's own. Kept for this game only." }));
    next.id = "controls-table";
    table.replaceWith(next);
    table = next;
    // Keep the focus where it was across the redraw (gamepad and keyboard
    // navigation).
    if (focused && card.contains(document.activeElement) === false) {
      const same = [...next.querySelectorAll("button")].find((b) =>
        (b.getAttribute("aria-label") ?? b.textContent) === focused);
      same?.focus({ preventScroll: true });
    }
  }

  return {
    /// Show it: `mode` "start" (before the game, Play) or "game" (while it
    /// runs, Resume, and Restart when needed; `current` the settings it's
    /// running with). Resolves with {action: "play" | "resume" | "restart",
    /// values, changed (URL overrides changed), forAll, skip}.
    show(m, current = null) {
      mode = m;
      running = current;
      play.textContent = m === "game" ? "Resume" : "Play";
      refresh();
      root.hidden = false;
      addEventListener("keydown", onKey, true);
      held = null;
      requestAnimationFrame(poll);
      play.focus({ preventScroll: true });
      return new Promise((r) => (resolveShow = r));
    },
    get open() {
      return resolveShow !== null;
    },
    /// The game's controls, editable: `binder` reaches the module
    /// (`view(overrides)`, `capture(overrides, slot, index, kind, value)`,
    /// `edit(overrides, op, slot, index)`; see game.html), `layout` is
    /// `touch_layout` (a touch column, and tilt where it steers).
    setControls(b, layout) {
      binder = b;
      touchLayout = layout;
      drawControls();
      if (layout.steer && (env.touchScreen || source.touch === "link")) {
        touchRows.append(onOff("tilt", "Tilt steering", "turn the device like a wheel"));
        refresh();
      }
    },
    /// The settings as they stand in the panel (a copy).
    values() {
      return { ...values };
    },
    /// The game's own crease angle (degrees, the module's `game_crease`),
    /// for the crease row's first option.
    setGameCrease(degrees) {
      gameCrease = degrees;
      rows.crease.draw();
    },
    /// What the game's manifest recommends for the Enhanced preset (the
    /// module's `game_recommended`, `{depth, shading}`). Rows still as the
    /// preset set them for the default follow, and a preset whose rows no
    /// longer match is Custom.
    setRecommended(r) {
      const was = presetValues(values.preset, rec);
      rec = r;
      const now = presetValues(values.preset, rec);
      for (const k of Object.keys(now)) {
        if (values[k] === was[k]) values[k] = now[k];
      }
      values = settle(values, rec);
      refresh();
    },
    /// A line under the buttons (loading progress, errors).
    setStatus(text, error = false) {
      status.textContent = text;
      status.style.color = error ? "#f66" : "";
    },
  };
}
