// On-screen controls for touch screens (or any device with ?touch=1).
//
// Left part of the screen: drag sideways to steer (analog: the further from
// where the finger went down, the harder the carve), or turn on tilt to steer
// by turning the device like a wheel. Right: jump (front), jump (tail) and
// slide. Top: coin, start, select left/right, tilt, and info (status bar).
// Everything is reported through `report(steer, slide, buttons)`; the button
// bits are those of the game's `set_touch`.

/// Drag distance (CSS pixels) for full steering lock.
const RANGE = 70;
/// Tilt (degrees) for full steering lock.
const TILT_RANGE = 30;

const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

/// Show the controls if this is a touch screen; true if shown.
export function setupTouch(report, toggleBar) {
  const root = document.getElementById("touch");
  const touchScreen = matchMedia("(pointer: coarse)").matches || "ontouchstart" in window;
  if (!touchScreen && !new URLSearchParams(location.search).has("touch")) return false;
  root.hidden = false;

  const state = { steer: 0, slide: 0, buttons: 0, tilt: null };
  const send = () => report(state.tilt ?? state.steer, state.slide, state.buttons);

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
  hold(document.getElementById("t-slide"), () => (state.slide = 1), () => (state.slide = 0));

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
  const onTilt = (e) => {
    const angle = screen.orientation?.angle ?? window.orientation ?? 0;
    // Landscape: turning the device changes beta; portrait: gamma.
    const deg = angle === 90 ? e.beta : angle === 270 || angle === -90 ? -e.beta : e.gamma;
    if (deg == null) return;
    state.tilt = clamp(deg / TILT_RANGE, -1, 1);
    send();
  };
  tiltButton.addEventListener("click", async () => {
    if (state.tilt !== null) {
      removeEventListener("deviceorientation", onTilt);
      state.tilt = null;
      tiltButton.classList.remove("on");
      pad.hidden = false;
      send();
      return;
    }
    if (typeof DeviceOrientationEvent?.requestPermission === "function") {
      try {
        if ((await DeviceOrientationEvent.requestPermission()) !== "granted") return;
      } catch {
        return;
      }
    }
    state.tilt = 0;
    tiltButton.classList.add("on");
    pad.hidden = true;
    addEventListener("deviceorientation", onTilt);
  });

  document.getElementById("t-info").addEventListener("click", toggleBar);
  return true;
}
