// Light guns on the page (gun games only: `touch_layout(game).gun`). The
// mouse aims player 1's gun over the canvas: the left button fires, the
// right reloads, and leaving the picture points off the screen. On a touch
// screen a tap fires at the spot tapped. A gamepad aims through Rust (its
// sticks move a cursor, drawn here).
//
// Everything goes through `setGun(player, x, y, onscreen, buttons)` (the
// module's `set_gun`): x and y are fractions of the game's 496x384 view,
// buttons bit 0 the trigger and bit 1 reload. Any other pointer source (a
// tracked hand, say) can call it the same way; `viewPoint` (the module's
// `view_point`) turns canvas pixels into those fractions.

/// How long a tap holds the trigger (ms): long enough for a frame or two.
const TAP_MS = 80;

/// Set up the canvas for player 1's gun; returns `{reload(on)}` for an
/// on-screen reload button, and `{cursor(xy)}` to draw the pad's cursor
/// (canvas pixels, or empty to hide it).
export function setupGun(canvas, viewPoint, setGun) {
  canvas.style.cursor = "crosshair";
  const state = { at: null, buttons: 0, reload: false };
  const send = () => {
    const b = state.buttons | (state.reload ? 2 : 0);
    if (state.at) setGun(0, state.at[0], state.at[1], true, b);
    else setGun(0, 0, 0, false, b);
  };
  // Client coordinates to view fractions, through the canvas's
  // object-fit: contain letterboxing; null off the picture.
  const locate = (e) => {
    const r = canvas.getBoundingClientRect();
    const s = Math.min(r.width / canvas.width, r.height / canvas.height);
    const x = (e.clientX - r.left - (r.width - canvas.width * s) / 2) / s;
    const y = (e.clientY - r.top - (r.height - canvas.height * s) / 2) / s;
    const p = viewPoint(x, y);
    return p.length ? [p[0], p[1]] : null;
  };
  const mouseButtons = (e) => (e.buttons & 1 ? 1 : 0) | (e.buttons & 2 ? 2 : 0);
  canvas.addEventListener("contextmenu", (e) => e.preventDefault());
  canvas.addEventListener("pointermove", (e) => {
    if (e.pointerType !== "mouse") return;
    state.at = locate(e);
    state.buttons = mouseButtons(e);
    send();
  });
  canvas.addEventListener("pointerleave", (e) => {
    if (e.pointerType !== "mouse") return;
    state.at = null;
    state.buttons = 0;
    send();
  });
  canvas.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    state.at = locate(e);
    if (e.pointerType === "mouse") {
      state.buttons = mouseButtons(e);
    } else {
      // A tap: aim there and pull the trigger briefly (a tap off the
      // picture shoots off screen, which reloads).
      state.buttons = 1;
      setTimeout(() => {
        state.buttons = 0;
        send();
      }, TAP_MS);
    }
    send();
  });
  canvas.addEventListener("pointerup", (e) => {
    if (e.pointerType !== "mouse") return;
    state.buttons = mouseButtons(e);
    send();
  });

  // The gamepad's cursor.
  const mark = document.createElement("div");
  mark.id = "crosshair";
  mark.hidden = true;
  document.body.append(mark);
  return {
    reload(on) {
      state.reload = on;
      send();
    },
    cursor(xy) {
      mark.hidden = !xy.length;
      if (!xy.length) return;
      const r = canvas.getBoundingClientRect();
      const s = Math.min(r.width / canvas.width, r.height / canvas.height);
      mark.style.left = `${r.left + (r.width - canvas.width * s) / 2 + xy[0] * s}px`;
      mark.style.top = `${r.top + (r.height - canvas.height * s) / 2 + xy[1] * s}px`;
    },
  };
}
