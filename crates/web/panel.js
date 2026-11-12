// The settings panel (docs/web.md, "Settings"): an HTML overlay shown
// before a game starts, and from the gear button (or Esc) while it runs.
// settings.js says what the settings are and how they're stored; this draws
// them, from the game's manifest data (the module's `bindings_view` and
// `touch_layout`) and nothing game-specific.
//
// Keyboard: Tab and the arrow keys move and change, Enter plays (or
// resumes), Esc resumes in game. Gamepad: d-pad or left stick up and down
// moves, left and right changes, A presses the focused button or toggles
// a box (else plays), Start plays, B resumes in game. Keys pressed while
// the panel is open don't reach the game.
//
// Remapping (the Controls section): click or tap a binding (or +), then
// press a key or gamepad button, or for an axis move a stick or press a
// trigger; Esc cancels. The module does the binding (`bindings_capture`,
// `bindings_edit`: the same rules as the native player) and says what it
// refuses; the panel only finds out what was pressed.

import { RESTART, audioBuffer, differ, screenHeight, windowAspect } from "./settings.js";

const el = (tag, props = {}, ...children) => {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...children);
  return e;
};

const STYLE = `
#settings { position: fixed; inset: 0; z-index: 10; display: flex; justify-content: center; align-items: flex-start;
  overflow-y: auto; overscroll-behavior: contain; -webkit-overflow-scrolling: touch; touch-action: pan-y;
  background: rgba(0, 0, 0, 0.72); backdrop-filter: blur(3px); -webkit-backdrop-filter: blur(3px);
  padding: max(12px, env(safe-area-inset-top)) max(16px, env(safe-area-inset-right))
    max(16px, env(safe-area-inset-bottom)) max(16px, env(safe-area-inset-left)); box-sizing: border-box; }
#settings[hidden] { display: none; }
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
#settings section { padding: 10px 16px 4px; }
#settings h3 { margin: 4px 0 8px; font-size: 12px; letter-spacing: 0.16em; text-transform: uppercase; color: #2fe6ef; }
#settings .row { margin: 0 0 12px; border: 0; padding: 0; min-width: 0; }
#settings .row > legend, #settings .row > .name { display: block; padding: 0; margin: 0 0 6px; font-weight: 600; font-size: 14px; }
#settings .hint { color: #8b93a3; font-size: 12px; font-weight: 400; }
#settings .link { display: inline-block; margin-left: 6px; padding: 0 6px; border-radius: 8px; background: #3a2a05;
  color: #ffd479; font-size: 11px; font-weight: 700; vertical-align: 1px; }
#settings .pills { display: flex; flex-wrap: wrap; gap: 8px; }
#settings .pills label { position: relative; }
#settings .pills input { position: absolute; opacity: 0; inset: 0; margin: 0; }
#settings .pills span { display: grid; place-items: center; min-height: 44px; padding: 6px 14px; border-radius: 22px;
  border: 2px solid #2c3a48; background: #0d141d; color: #c9d0dc; font-weight: 600; font-size: 14px; cursor: pointer; }
#settings .pills input:checked + span { border-color: #2fe6ef; background: rgba(47, 230, 239, 0.15); color: #fff; }
#settings .pills input:focus-visible + span { outline: 2px solid #fff; outline-offset: 2px; }
#settings .slider { display: flex; align-items: center; gap: 12px; }
#settings input[type=range] { flex: 1; min-width: 0; height: 44px; accent-color: #2fe6ef; margin: 0; }
#settings output { min-width: 4.5em; text-align: right; font-variant-numeric: tabular-nums; }
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
/* Play stays in reach on a small screen. */
#settings .actions { position: sticky; bottom: calc(-1 * max(16px, env(safe-area-inset-bottom)));
  padding: 10px 16px calc(14px + max(16px, env(safe-area-inset-bottom)));
  margin-bottom: calc(-1 * max(16px, env(safe-area-inset-bottom))); background: #070b12;
  border-top: 1px solid #142230; border-radius: 0 0 12px 12px; }
#settings .buttons { display: flex; flex-wrap: wrap; gap: 10px; }
#settings button { min-height: 48px; padding: 0 20px; border-radius: 24px; font: 700 16px system-ui, sans-serif; cursor: pointer;
  border: 2px solid #2c3a48; background: #0d141d; color: #f2f4f8; }
#settings button.primary { flex: 1; border-color: #2fe6ef; background: rgba(47, 230, 239, 0.18);
  box-shadow: 0 0 12px rgba(47, 230, 239, 0.35); }
#settings button:focus-visible { outline: 2px solid #fff; outline-offset: 2px; }
#settings button[hidden] { display: none; }
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
  const root = el("div", { id: "settings", hidden: true });
  root.setAttribute("role", "dialog");
  root.setAttribute("aria-modal", "true");
  root.setAttribute("aria-label", "Settings");
  const rows = {};
  const badge = (key) => (source[key] === "link" ? el("span", { className: "link", textContent: "from link" }) : "");

  // A row of pills, one per option ([value, label]).
  const choice = (key, label, options, hint) => {
    const set = el("fieldset", { className: "row" });
    const legend = el("legend", {}, label, badge(key));
    if (hint) legend.append(" ", el("span", { className: "hint", textContent: hint }));
    const pills = el("div", { className: "pills" });
    set.append(legend, pills);
    const draw = () => {
      pills.replaceChildren();
      for (const [v, text] of options()) {
        const input = el("input", { type: "radio", name: `s-${key}`, checked: v === values[key] });
        input.addEventListener("change", () => update(key, v));
        pills.append(el("label", {}, input, el("span", { textContent: text })));
      }
    };
    draw();
    rows[key] = { el: set, draw, legend };
    return set;
  };
  const onOff = (key, label, hint) => choice(key, label, () => [[true, "On"], [false, "Off"]], hint);
  const slider = (key, label) => {
    const out = el("output");
    const input = el("input", { type: "range", min: -24, max: 24, step: 1 });
    input.setAttribute("aria-label", label);
    const show = () => (out.textContent = `${values[key] > 0 ? "+" : ""}${values[key]} dB`);
    input.addEventListener("input", () => {
      update(key, Number(input.value));
      show();
    });
    const set = el("div", { className: "row" }, el("div", { className: "name" }, label, badge(key)),
      el("div", { className: "slider" }, input, out));
    const draw = () => {
      input.value = values[key];
      show();
    };
    draw();
    rows[key] = { el: set, draw };
    return set;
  };
  // An option for a value the URL gave that the panel doesn't list.
  const withValue = (key, list, label) =>
    list.some(([v]) => v === values[key]) ? list : [...list, [values[key], label(values[key])]];

  // The game's own crease angle for smooth shading, once the module says
  // (setGameCrease).
  let gameCrease = null;
  const height = screenHeight(env);
  const fit = windowAspect(env);
  const display = el("section", {}, el("h3", { textContent: "Display" }),
    choice("height", "Render height", () => withValue("height",
      [[384, "384 (native)"], [768, "768"], [1080, "1080"], ["auto", `Screen's (${height})`]], (v) => String(v))),
    choice("aspect", "Aspect", () => withValue("aspect",
      [["auto", `Fit window (${fit.toFixed(2)}:1)`], ["4:3", "4:3"], ["16:9", "16:9"]], (v) => v)),
    choice("hud", "HUD", () => [["edges", "At the edges"], ["center", "Centred"]], "in wide views"),
    onOff("bezel", "Bezel", "beside a 4:3 picture in a wider window"),
    choice("aa", "Anti-aliasing", () => [["off", "Off"], ["4", "4x"]], "smooths polygon edges"),
    onOff("depth", "Depth buffer", "polygons that cross or are sorted wrong come out right; off draws in the game's order"),
    choice("shading", "Shading", () => [["flat", "Flat"], ["vertex", "Smooth"], ["pixel", "Per pixel"]],
      "flat is the original; smooth blends the game's own lighting across curved surfaces, worked out at each " +
      "corner; per pixel works it out for every pixel (with the chip geometrizer it draws as smooth)"),
    choice("crease", "Crease angle", () => withValue("crease",
      [["auto", gameCrease === null ? "The game's" : `The game's (${gameCrease}°)`], [30, "30°"], [40, "40°"], [60, "60°"]],
      (v) => `${v}°`), "with smooth shading: faces meeting at a sharper angle keep a hard edge"),
    onOff("shadefine", "Blend brightness levels",
      "with smooth shading: no banding on large smooth surfaces; off keeps the game's 64 levels"),
    onOff("shadebalance", "Balance brightness",
      "with smooth shading: each smooth surface as bright as the game's; off, bumpy ones come out brighter"),
    onOff("pack", "Texture pack", "replacement textures from the server's texpack folder for this game, if it has one"),
    choice("texfilter", "Pack filtering", () => [["off", "Off"], ["mip", "Mipmaps"], ["2", "2x"], ["4", "4x"],
      ["8", "8x"], ["16", "16x"]], "mipmaps, then anisotropic filtering; texture pack images only"),
    onOff("materials", "Material maps",
      "with the texture pack: its normal maps, bumps and grooves lit by the game's own lights (needs the " +
      "native geometrizer; no effect with the chip)"),
    choice("relief", "Material relief", () => withValue("relief",
      [["1", "1"], ["2", "2"], ["4", "4"], ["8", "8"], ["off", "Faithful"]], (v) => `Custom (${v})`),
      "with material maps: how strongly they shade (2 by default); faithful keeps the game's own lighting sum, subtler"));
  const autoBuffer = audioBuffer("auto", env).target * 1000;
  const sound = el("section", {}, el("h3", { textContent: "Sound" }),
    onOff("sound", "Sound"), slider("sfx", "Effects"), slider("music", "Music"),
    choice("buffer", "Buffer", () => withValue("buffer",
      [["auto", `Auto (${autoBuffer} ms)`], [70, "70 ms"], [150, "150 ms"], [250, "250 ms"]], (v) => `${v} ms`),
    "more rides out uneven frames without bending the pitch, but lags more"));
  // Controls: filled in once the game's module is loaded (setControls).
  let table = el("p", { className: "hint", textContent: "Loading the game's controls…" });
  const controls = el("section", {}, el("h3", { textContent: "Controls" }), table);
  const touchRows = el("div");
  if (env.touchScreen || source.touch === "link") {
    touchRows.append(onOff("touch", "Touch controls"));
  }
  controls.append(touchRows);
  const advanced = el("section", {}, el("h3", { textContent: "Advanced" }),
    choice("geo", "Geometrizer", () => [
      ["worker", "In a worker"],
      ["local", "On the page's thread"],
      ["chip", "In a worker, on the game's own program on its chip (the reference)"],
    ]),
    onOff("smooth", "Smooth motion",
      "in-between frames on displays faster than 60 Hz, a frame later; not with the chip"));

  const forAll = el("input", { type: "checkbox", checked: !!opts.forAll });
  const skip = el("input", { type: "checkbox", checked: !!opts.skip });
  const play = el("button", { type: "button", className: "primary", textContent: "Play" });
  const restart = el("button", { type: "button", textContent: "Restart to apply", hidden: true });
  const reset = el("button", { type: "button", textContent: "Defaults" });
  const note = el("p", { className: "hint", hidden: true,
    textContent: "Render height, aspect, sound on or off, the sound buffer, the geometrizer and smooth motion " +
      "apply on a restart." });
  const status = el("p", { className: "status" });
  const footer = el("footer", {},
    el("label", { className: "check" }, forAll, "Use these for all games"),
    el("label", { className: "check" }, skip, "Don't show this again for this game"),
    note);
  const actions = el("div", { className: "actions" }, el("div", { className: "buttons" }, reset, restart, play), status);
  const subtitle = el("p", { textContent: "Settings are kept in this browser. The ⚙︎ button opens them again." });
  const card = el("div", { className: "card" },
    el("header", {}, el("a", { className: "back", href: opts.back, textContent: "← Games" }),
      el("h2", { textContent: opts.title }), subtitle),
    display, sound, controls, advanced, footer, actions);
  root.append(card);
  document.body.append(root);

  // In game: the settings the running game started with.
  let running = null;
  let resolveShow = null;
  let mode = "start";

  function update(key, v) {
    values[key] = v;
    if (source[key] === "link") {
      changed.add(key);
      source[key] = "panel";
      rows[key]?.el.querySelector(".link")?.remove();
    }
    refresh();
  }
  function refresh() {
    const off = !values.sound;
    for (const k of ["sfx", "music", "buffer"]) rows[k].el.style.opacity = off ? 0.5 : "";
    const needRestart = running && differ(values, running, RESTART).length > 0;
    restart.hidden = !needRestart;
    note.hidden = mode !== "game";
    if (rows.tilt) rows.tilt.el.hidden = !values.touch;
    // Smooth motion needs the native geometrizer.
    rows.smooth.el.style.opacity = values.geo === "chip" ? 0.5 : "";
    // The crease angle only matters with smooth shading.
    rows.crease.el.style.opacity = values.shading === "flat" ? 0.5 : "";
    rows.shadefine.el.style.opacity = values.shading === "flat" ? 0.5 : "";
    rows.shadebalance.el.style.opacity = values.shading === "flat" ? 0.5 : "";
    // Material maps need the texture pack (and the native geometrizer).
    rows.materials.el.style.opacity = !values.pack || values.geo === "chip" ? 0.5 : "";
    rows.relief.el.style.opacity = !values.materials || !values.pack || values.geo === "chip" ? 0.5 : "";
  }
  reset.addEventListener("click", () => {
    for (const k of Object.keys(opts.base)) {
      if (k === "touch") continue; // the device's choice; Defaults shouldn't hide the controls
      if (k === "controls") continue; // the Controls section has its own reset
      update(k, opts.base[k]);
      rows[k]?.draw();
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
    r({ action, values: { ...values }, changed: new Set(changed), forAll: forAll.checked, skip: skip.checked });
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
    if (e.key === "Enter") {
      // The focused button (Defaults, Restart, Play), else Play.
      e.preventDefault();
      const f = document.activeElement;
      (f instanceof HTMLButtonElement && card.contains(f) ? f : play).click();
    } else if (e.key === "Escape" && mode === "game") {
      e.preventDefault();
      finish("resume");
    }
  }

  // Gamepad: polled while open; a button counts when pressed after the
  // panel opened (one held then must be released first).
  const focusables = () => [...card.querySelectorAll("input, button")].filter((e) => {
    if (e.closest("[hidden]") || e.hidden) return false;
    // One stop per row of pills: its checked one.
    return e.type !== "radio" || e.checked;
  });
  const moveFocus = (d) => {
    const list = focusables();
    const i = list.indexOf(document.activeElement);
    const next = list[i < 0 ? 0 : (i + d + list.length) % list.length];
    next?.focus();
    next?.scrollIntoView({ block: "nearest" });
  };
  const change = (d) => {
    const a = document.activeElement;
    if (a?.type === "radio") {
      const group = [...card.querySelectorAll(`input[name="${a.name}"]`)];
      const next = group[(group.indexOf(a) + d + group.length) % group.length];
      next.checked = true;
      next.dispatchEvent(new Event("change"));
      next.focus();
    } else if (a?.type === "range") {
      a.value = Number(a.value) + d;
      a.dispatchEvent(new Event("input"));
    }
  };
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
      a: b(0), b: b(1), start: b(9),
      up: b(12) || y < -0.6, down: b(13) || y > 0.6, left: b(14) || x < -0.6, right: b(15) || x > 0.6,
    };
    if (held === null) held = now;
    const pressed = (k) => now[k] && !held[k];
    // Directions repeat while held.
    const repeat = (k) => pressed(k) || (now[k] && t >= repeatAt);
    if (pressed("start")) play.click();
    else if (pressed("b") && mode === "game") finish("resume");
    else if (pressed("a")) {
      const f = document.activeElement;
      if (f instanceof HTMLButtonElement && card.contains(f)) f.click();
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
    if (capture.accepts !== "axis") return;
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
      for (const r of Object.values(rows)) r.draw();
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
    /// The game's own crease angle (degrees, the module's `game_crease`),
    /// for the crease row's first option.
    setGameCrease(degrees) {
      gameCrease = degrees;
      rows.crease.draw();
    },
    /// A line under the buttons (loading progress, errors).
    setStatus(text, error = false) {
      status.textContent = text;
      status.style.color = error ? "#f66" : "";
    },
  };
}
