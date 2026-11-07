// On-screen controls for touch screens (or any device with the touch setting
// on, `?touch=1`). They
// act as a gamepad, so they play every game by its pad bindings; the page
// passes the game's layout (`touch_layout` in lib.rs).
//
// Left part of the screen: drag sideways to steer (analog: the further from
// where the finger went down, the harder the turn), or turn on tilt to steer
// by turning the device like a wheel. Right: the face buttons and triggers
// the game uses. Top: coin, start and the shoulder buttons, tilt, and info
// (status bar). Gun games have no steering or round buttons (taps on the
// picture shoot, gun.js) but a reload pill, which holds `gun.reload`.
// Everything else is reported through
// `report(steer, rightTrigger, leftTrigger, buttons)`, buttons as bits by
// standard gamepad index.

/// Drag distance (CSS pixels) for full steering lock.
const RANGE = 70;
/// Tilt (degrees) for full steering lock.
const TILT_RANGE = 30;

const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

/// Whether this is a touch screen (the touch controls' default).
export const touchScreen = () => matchMedia("(pointer: coarse)").matches || "ontouchstart" in window;

/// Ask for motion events (iOS asks the player; elsewhere there's nothing to
/// ask). Call it from a tap or key press: iOS refuses otherwise. Resolves
/// true if tilt can work.
export function tiltPermission() {
  const DOE = globalThis.DeviceOrientationEvent;
  if (typeof DOE?.requestPermission !== "function") return Promise.resolve(true);
  try {
    return DOE.requestPermission().then((r) => r === "granted", () => false);
  } catch {
    return Promise.resolve(false);
  }
}

/// Set up the controls, shown if `show`. `layout`: {steer, gun, top:
/// [{label, bit} or {label, gun}], actions: [{label, bit} or {label,
/// trigger}]}. `gun`: gun.js's handle, for gun games. Returns
/// {show(on), tilt(on, permission), shown}: `permission` a promise from
/// `tiltPermission`, asked for in the gesture that turned tilt on.
export function setupTouch(layout, report, toggleBar, gun, show) {
  const root = document.getElementById("touch");
  root.hidden = !show;

  const state = { steer: 0, right: 0, left: 0, buttons: 0, tilt: null };
  const send = () => report(state.tilt ?? state.steer, state.right, state.left, state.buttons);

  // The game's buttons: small pills along the top (before tilt and info),
  // round ones at the right, the last (the pad's A, usually) big at the
  // bottom right.
  const make = (cls, c) => {
    const el = document.createElement("span");
    el.className = cls;
    el.textContent = c.label;
    if ("bit" in c) el.dataset.bit = c.bit;
    else if ("gun" in c) el.dataset.gun = c.gun;
    else el.dataset.trigger = c.trigger;
    return el;
  };
  const top = document.getElementById("t-top");
  for (const c of layout.top ?? []) top.insertBefore(make("t-pill", c), document.getElementById("t-tilt"));
  const actions = document.getElementById("t-actions");
  const list = layout.actions ?? [];
  list.forEach((c, i) => {
    if (i === list.length - 1 && list.length % 2) actions.append(document.createElement("span"));
    actions.append(make(i === list.length - 1 ? "t-btn big" : "t-btn", c));
  });

  // Hold-to-press buttons: each finger is tracked on the button it went
  // down on, so several can be held at once.
  const hold = (el, on, off) => {
    el.addEventListener("pointerdown", (e) => {
      e.preventDefault();
      el.setPointerCapture(e.pointerId);
      el.classList.add("on");
      on();
      send();
    });
    for (const t of ["pointerup", "pointercancel", "lostpointercapture"]) {
      el.addEventListener(t, () => {
        if (!el.classList.contains("on")) return;
        el.classList.remove("on");
        off();
        send();
      });
    }
  };
  for (const el of root.querySelectorAll("[data-bit]")) {
    const bit = 1 << Number(el.dataset.bit);
    hold(el, () => (state.buttons |= bit), () => (state.buttons &= ~bit));
  }
  for (const el of root.querySelectorAll('[data-gun="reload"]')) {
    hold(el, () => gun?.reload(true), () => gun?.reload(false));
  }
  for (const el of root.querySelectorAll("[data-trigger]")) {
    const side = el.dataset.trigger;
    hold(el, () => (state[side] = 1), () => (state[side] = 0));
  }

  // Steering: relative to where the finger went down.
  const pad = document.getElementById("t-steer");
  const knob = document.getElementById("t-knob");
  let finger = null;
  let x0 = 0;
  const centre = () => {
    finger = null;
    state.steer = 0;
    knob.classList.remove("on");
    send();
  };
  pad.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    if (finger !== null) return;
    finger = e.pointerId;
    x0 = e.clientX;
    pad.setPointerCapture(finger);
    const r = pad.getBoundingClientRect();
    knob.style.left = `${e.clientX - r.left}px`;
    knob.style.top = `${e.clientY - r.top}px`;
    knob.style.setProperty("--dx", "0px");
    knob.classList.add("on");
  });
  pad.addEventListener("pointermove", (e) => {
    if (e.pointerId !== finger) return;
    state.steer = clamp((e.clientX - x0) / RANGE, -1, 1);
    knob.style.setProperty("--dx", `${state.steer * RANGE}px`);
    send();
  });
  for (const t of ["pointerup", "pointercancel"]) {
    pad.addEventListener(t, (e) => e.pointerId === finger && centre());
  }

  // Tilt: turn the device like a steering wheel. iOS asks for permission.
  const tiltButton = document.getElementById("t-tilt");
  if (!layout.steer) {
    pad.hidden = true;
    tiltButton.hidden = true;
  }
  const onTilt = (e) => {
    const angle = screen.orientation?.angle ?? window.orientation ?? 0;
    // Landscape: turning the device changes beta; portrait: gamma.
    const deg = angle === 90 ? e.beta : angle === 270 || angle === -90 ? -e.beta : e.gamma;
    if (deg == null) return;
    state.tilt = clamp(deg / TILT_RANGE, -1, 1);
    send();
  };
  const tiltOff = () => {
    removeEventListener("deviceorientation", onTilt);
    state.tilt = null;
    tiltButton.classList.remove("on");
    pad.hidden = false;
    send();
  };
  const tiltOn = async (permission) => {
    if (!layout.steer || state.tilt !== null || !(await permission)) return;
    state.tilt = 0;
    tiltButton.classList.add("on");
    pad.hidden = true;
    addEventListener("deviceorientation", onTilt);
  };
  tiltButton.addEventListener("click", () => (state.tilt !== null ? tiltOff() : tiltOn(tiltPermission())));

  document.getElementById("t-info").addEventListener("click", toggleBar);
  return {
    show(on) {
      root.hidden = !on;
      if (!on) {
        // Let go of anything held.
        Object.assign(state, { steer: 0, right: 0, left: 0, buttons: 0 });
        for (const e of root.querySelectorAll(".on:not(#t-tilt)")) e.classList.remove("on");
        knob.classList.remove("on");
        finger = null;
        send();
      }
    },
    tilt(on, permission) {
      if (on) tiltOn(permission);
      else if (state.tilt !== null) tiltOff();
    },
    get shown() {
      return !root.hidden;
    },
  };
}
