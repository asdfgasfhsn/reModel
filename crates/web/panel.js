// The settings panel (docs/web.md, "Settings"): an HTML overlay shown
// before a game starts, and from the gear button (or Esc) while it runs.
// settings.js says what the settings are and how they're stored; this draws
// them, from the game's manifest data (the module's `controls_table` and
// `touch_layout`) and nothing game-specific.
//
// Keyboard: Tab and the arrow keys move and change, Enter plays (or
// resumes), Esc resumes in game. Gamepad: d-pad or left stick up and down
// moves, left and right changes, A presses the focused button or toggles
// a box (else plays), Start plays, B resumes in game. Keys pressed while
// the panel is open don't reach the game.

import { RESTART, differ, screenHeight, windowAspect } from "./settings.js";

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

  const height = screenHeight(env);
  const fit = windowAspect(env);
  const display = el("section", {}, el("h3", { textContent: "Display" }),
    choice("height", "Render height", () => withValue("height",
      [[384, "384 (native)"], [768, "768"], [1080, "1080"], ["auto", `Screen's (${height})`]], (v) => String(v))),
    choice("aspect", "Aspect", () => withValue("aspect",
      [["auto", `Fit window (${fit.toFixed(2)}:1)`], ["4:3", "4:3"], ["16:9", "16:9"]], (v) => v)),
    choice("hud", "HUD", () => [["edges", "At the edges"], ["center", "Centred"]], "in wide views"));
  const sound = el("section", {}, el("h3", { textContent: "Sound" }),
    onOff("sound", "Sound"), slider("sfx", "Effects"), slider("music", "Music"));
  // Controls: read-only, filled in once the game's module is loaded.
  const table = el("p", { className: "hint", textContent: "Loading the game's controls…" });
  const controls = el("section", {}, el("h3", { textContent: "Controls" }), table);
  const touchRows = el("div");
  if (env.touchScreen || source.touch === "link") {
    touchRows.append(onOff("touch", "Touch controls"));
  }
  controls.append(touchRows);
  const advanced = el("section", {}, el("h3", { textContent: "Advanced" }),
    choice("geo", "Geometrizer", () => [["worker", "In a worker"], ["local", "On the page's thread"]]));

  const forAll = el("input", { type: "checkbox", checked: !!opts.forAll });
  const skip = el("input", { type: "checkbox", checked: !!opts.skip });
  const play = el("button", { type: "button", className: "primary", textContent: "Play" });
  const restart = el("button", { type: "button", textContent: "Restart to apply", hidden: true });
  const reset = el("button", { type: "button", textContent: "Defaults" });
  const note = el("p", { className: "hint", hidden: true,
    textContent: "Render height, aspect, sound on or off and the geometrizer apply on a restart." });
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
    for (const k of ["sfx", "music"]) rows[k].el.style.opacity = off ? 0.5 : "";
    const needRestart = running && differ(values, running, RESTART).length > 0;
    restart.hidden = !needRestart;
    note.hidden = mode !== "game";
    if (rows.tilt) rows.tilt.el.hidden = !values.touch;
  }
  reset.addEventListener("click", () => {
    for (const k of Object.keys(opts.base)) {
      if (k === "touch") continue; // the device's choice; Defaults shouldn't hide the controls
      update(k, opts.base[k]);
      rows[k]?.draw();
    }
  });

  const finish = (action) => {
    if (!resolveShow) return;
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
    /// The game's controls: `table` from `controls_table`, `layout` from
    /// `touch_layout` (a touch column, and tilt where it steers).
    setControls(list, layout) {
      const touchOf = (row) => {
        if (row.gun) return row.label.startsWith("Aim") ? "tap the picture" : "";
        const items = [...(layout.top ?? []), ...(layout.actions ?? [])];
        const hit = items.find((i) => i.label === row.label || i.label.startsWith(row.label + " "));
        if (hit) return hit.label;
        return layout.steer && row.pad.includes("left stick ↔") ? "drag on the left, or Tilt" : "";
      };
      const touch = env.touchScreen || values.touch;
      const head = el("tr", {}, el("th", { textContent: "" }), el("th", { textContent: "Keyboard" }),
        el("th", { textContent: "Gamepad" }));
      if (touch) head.append(el("th", { textContent: "Touch" }));
      const body = list.map((r) => {
        const tr = el("tr", {}, el("td", { textContent: r.label }),
          el("td", {}, ...r.keys.map((k) => (k.includes(" ") ? k + " " : el("kbd", { textContent: k })))),
          el("td", { textContent: r.pad.join(", ") }));
        if (touch) tr.append(el("td", { textContent: touchOf(r) }));
        return tr;
      });
      table.replaceWith(el("table", {}, el("thead", {}, head), el("tbody", {}, ...body)));
      if (layout.steer && (env.touchScreen || source.touch === "link")) {
        touchRows.append(onOff("tilt", "Tilt steering", "turn the device like a wheel"));
        refresh();
      }
    },
    /// A line under the buttons (loading progress, errors).
    setStatus(text, error = false) {
      status.textContent = text;
      status.style.color = error ? "#f66" : "";
    },
  };
}
