/* reModel website. Plain JS, shared by every page; each part runs only where its elements are.
   Nothing here runs on every scroll frame except the sequence (one rect read, then writes). */
(function () {
  'use strict';
  var root = document.documentElement;
  var reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  var $ = function (s, c) { return (c || document).querySelector(s); };
  var $$ = function (s, c) { return Array.prototype.slice.call((c || document).querySelectorAll(s)); };
  var clamp = function (v, a, b) { return Math.max(a, Math.min(b, v)); };
  root.classList.add('js');

  /* ---------- the header: mark the current page ---------- */
  var here = location.pathname.split('/').pop() || 'index.html';
  $$('.nav a').forEach(function (a) {
    var h = a.getAttribute('href');
    if (h === here || (h === './' && here === 'index.html')) a.setAttribute('aria-current', 'page');
  });

  /* ---------- reveals: wipes, lines, tiles and starbursts ----------
     An IntersectionObserver, not a scan on every scroll. A wipe is clipped, so its parent is watched.
     When one element reveals, every element before it in the page reveals too, so a fast scroll that
     jumps over something never leaves it hidden. */
  var pend = $$('.wipe,.rise,.ln,.slapin');
  $$('.lines').forEach(function (ul) { $$('.ln', ul).forEach(function (l, i) { l.style.setProperty('--i', i); }); });
  $$('.slapin').forEach(function (s, i) { s.style.setProperty('--d', (i % 3) * 0.14 + 's'); });
  function show(x) {
    x.classList.add('in');
    /* once a wipe has run, drop its clip-path altogether so the headline can never stay clipped */
    if (x.classList.contains('wipe')) setTimeout(function () { x.classList.add('done'); }, 800);
  }
  function revealUpTo(x) { var i = pend.indexOf(x); if (i >= 0) pend.splice(0, i + 1).forEach(show); }
  if (reduce || !('IntersectionObserver' in window)) pend.splice(0).forEach(show);
  else {
    var watched = new Map();
    var io = new IntersectionObserver(function (es) {
      es.forEach(function (e) {
        if (!(e.isIntersecting || e.boundingClientRect.bottom < 0)) return;
        (watched.get(e.target) || []).forEach(revealUpTo);
        io.unobserve(e.target);
      });
    }, { rootMargin: '0px 0px 12% 0px' });
    pend.forEach(function (x) {
      var t = x.classList.contains('wipe') ? x.parentElement : x;
      if (!watched.has(t)) { watched.set(t, []); io.observe(t); }
      watched.get(t).push(x);
    });
  }

  /* ---------- panels that change once as they come into view (sun-fade, reprint) ---------- */
  var panels = $$('[data-seen]');
  if (reduce || !('IntersectionObserver' in window)) panels.forEach(function (p) { p.classList.add('seen'); });
  else {
    var pio = new IntersectionObserver(function (es) {
      es.forEach(function (e) { if (e.isIntersecting) { e.target.classList.add('seen'); pio.unobserve(e.target); } });
    }, { threshold: 0.3 });
    panels.forEach(function (p) { pio.observe(p); });
  }

  /* ---------- the ticker runs only while it is on screen ---------- */
  var band = $('.tickband');
  if (band && !reduce && 'IntersectionObserver' in window) {
    new IntersectionObserver(function (es) { band.classList.toggle('run', es[0].isIntersecting); }).observe(band);
  }

  /* ---------- gameplay gallery: shots/gallery/<set>.webp ---------- */
  var GALLERY = [['bel', 'Behind Enemy Lines'], ['desert', 'Desert Tank'], ['doa', 'Dead or Alive'], ['gunblade', 'Gunblade NY'], ['hotd', 'The House of the Dead'],
    ['indy500', 'Indianapolis 500'], ['lastbrnx', 'Last Bronx'], ['overrev', 'Over Rev'], ['sgt24h', 'Super GT 24h'], ['vcop2', 'Virtua Cop 2']];
  $$('.shots').forEach(function (shots) {
    var only = shots.dataset.only ? shots.dataset.only.split(' ') : null;
    GALLERY.forEach(function (g) {
      if (only && only.indexOf(g[0]) < 0) return;
      var f = document.createElement('figure'), im = new Image(), cap = document.createElement('figcaption');
      f.className = 'shot'; im.alt = g[1] + ' gameplay'; im.width = 1280; im.height = 720; im.loading = 'lazy'; im.decoding = 'async';
      im.onerror = function () { f.remove(); };
      im.src = 'shots/gallery/' + g[0] + '.webp'; cap.textContent = g[1];
      f.appendChild(im); f.appendChild(cap); shots.appendChild(f);
    });
  });

  /* ---------- poster tilt ---------- */
  if (!reduce && window.matchMedia('(hover:hover)').matches) {
    $$('.card').forEach(function (c) {
      c.addEventListener('pointermove', function (e) {
        var r = c.getBoundingClientRect(), x = (e.clientX - r.left) / r.width - 0.5, y = (e.clientY - r.top) / r.height - 0.5;
        c.style.transform = 'rotateY(' + (x * 16) + 'deg) rotateX(' + (-y * 14) + 'deg) translateZ(14px) rotate(-1.5deg)';
      });
      c.addEventListener('pointerleave', function () { c.style.transform = ''; });
    });
  }

  /* ---------- picture styles: before/after wipe ---------- */
  /* [file name in shots/looks/, label, note]; styles whose image is missing drop out */
  var LOOKS = [
    ['original', 'Arcade original', 'The default: the picture as the arcade drew it.'],
    ['sharp', 'Sharp', 'Anti-aliased edges (MSAA 4x) and mipmapped textures.'],
    ['enhanced', 'Smooth modern', 'Set per game: temporal anti-aliasing, smooth shading, smooth motion and run-ahead where they suit it. The recommended style.'],
    ['crt', 'CRT', 'A scanline for each of the game\'s 384 lines at any output size, with a mask, glow and vignette.'],
    ['comic', 'Comic', 'Ink outlines, toon lighting and halftone shadows.']
  ];
  (function looks() {
    var cmp = $('#cmp'), a = $('#cmpA'), b = $('#cmpB'), h = $('#cmpH'), chips = $('#chips'), note = $('#chipNote'), lab = $('#cmpLab');
    if (!cmp) return;
    var loaded = {}, pending = LOOKS.length;
    LOOKS.forEach(function (l) {
      var im = new Image();
      im.onload = function () { loaded[l[0]] = true; done(); };
      im.onerror = done;
      im.src = 'shots/looks/' + l[0] + '.webp';
    });
    function done() { if (--pending === 0) build(); }
    function build() {
      var list = LOOKS.filter(function (l) { return loaded[l[0]] && l[0] !== 'original'; });
      if (!loaded.original || !list.length) { cmp.hidden = true; chips.hidden = true; note.hidden = true; return; }
      a.src = 'shots/looks/original.webp';
      list.forEach(function (l) {
        var c = document.createElement('button');
        c.type = 'button'; c.className = 'chip'; c.textContent = l[1];
        c.addEventListener('click', function () { pick(l, c); });
        chips.appendChild(c); l.chip = c;
      });
      pick(list[0], list[0].chip);
    }
    function pick(l, c) {
      b.src = 'shots/looks/' + l[0] + '.webp'; lab.textContent = l[1].toUpperCase();
      note.textContent = l[2];
      $$('.chip', chips).forEach(function (x) { x.setAttribute('aria-pressed', x === c ? 'true' : 'false'); });
    }
    function set(v) { v = clamp(v, 0, 100); cmp.style.setProperty('--x', v + '%'); h.setAttribute('aria-valuenow', Math.round(v)); }
    function at(e) { var r = cmp.getBoundingClientRect(); set((e.clientX - r.left) / r.width * 100); }
    var down = false;
    cmp.addEventListener('pointerdown', function (e) { down = true; try { cmp.setPointerCapture(e.pointerId); } catch (x) {} at(e); });
    cmp.addEventListener('pointermove', function (e) { if (down) at(e); });
    cmp.addEventListener('pointerup', function () { down = false; });
    cmp.addEventListener('pointercancel', function () { down = false; });
    h.addEventListener('keydown', function (e) {
      var v = parseFloat(h.getAttribute('aria-valuenow')), k = e.key, d = e.shiftKey ? 15 : 5;
      if (k === 'ArrowLeft' || k === 'ArrowDown') set(v - d);
      else if (k === 'ArrowRight' || k === 'ArrowUp') set(v + d);
      else if (k === 'Home') set(0); else if (k === 'End') set(100); else return;
      e.preventDefault();
    });
  })();

  /* ---------- the sequence ----------
     The stage is position: sticky inside a tall track (both sized in svh, so a phone's toolbar
     showing or hiding moves nothing). The track's scroll progress drives a paused GSAP timeline.
     Each frame reads one rect, then only writes. */
  var gameName = $('#gameName'), stage = $('#stage'), track = $('.seq-track');
  if (gameName && stage) gameName.textContent = stage.dataset.game + ', one frame';
  if (!window.gsap || reduce || !stage || !track) return;
  root.classList.add('js-seq');

  var plate = $('#plate'), pw = $('#pw'), capbar = $('.capbar', pw);
  var L = { nat: $('#lNat'), enh: $('#lEnh'), bez: $('#lBez'), side: $('#lSide'), wide: $('#lWide'), ultra: $('#lUltra') };
  var stks = $$('.stk', pw), whip = $$('.whip i'), fign = $('#fign'), ghost = $('#ghost');
  var S = { nat: 1, enh: 0, bez: 0, bezY: 0, side: 0, wide: 0, ultra: 0, w: 4 / 3 };
  var GHOST = ['4:3', '4:3', 'BEZEL', '16:9', '16:9', '21:9'];
  var H = 300, availW = 360, cur = -1, lastW = 0;
  /* step k's picture changes over [k, k+0.55); its label, sticker and caption follow once the change is
     done (LAG), and the extra half step at the end gives the last view a full step of its own */
  var LAG = 0.56, SPAN = 6.5;

  /* a layer hides the one under it only when it has fully faded in AND its image has loaded */
  function up(img, v) { return v >= 0.999 && img.complete && img.naturalWidth > 0; }

  function size() {
    var vw = root.clientWidth, vh = stage.clientHeight;
    lastW = vw;
    availW = vw - 2 * 16 - 24;
    H = Math.min(vh * (vw < 700 ? 0.4 : 0.54), availW * 0.75);
  }
  function apply() {
    var W = S.w * H, sc = Math.min(1, availW / W), h = H * sc, w = W * sc;
    plate.style.setProperty('--w', w + 'px'); plate.style.setProperty('--h', h + 'px');
    capbar.style.setProperty('--w', w + 'px');
    L.nat.style.opacity = up(L.enh, S.enh) ? 0 : S.nat;
    L.enh.style.opacity = up(L.wide, S.wide) ? 0 : S.enh;
    L.bez.style.opacity = S.bez;
    L.bez.style.transform = 'translateX(-50%) translateY(' + (S.bezY * h) + 'px) rotate(' + (S.bezY * 7) + 'deg)';
    L.side.style.opacity = up(L.wide, S.wide) ? 0 : S.side;
    L.wide.style.opacity = up(L.ultra, S.ultra) ? 0 : S.wide;
    L.ultra.style.opacity = S.ultra;
  }
  /* keep the step's sticker on screen; measured only when the step or the width changes */
  function fitSticker() {
    var a = stks[cur >= 0 ? cur : 0];
    if (!a) return;
    a.style.translate = '';
    var vw = root.clientWidth, l = pw.getBoundingClientRect().left + a.offsetLeft, sw = a.offsetWidth, dx = 0;
    if (l + sw > vw - 8) dx = vw - 8 - (l + sw);
    if (l + dx < 8) dx = 8 - l;
    a.style.translate = dx + 'px 0';
  }
  var tl = gsap.timeline({ paused: true, defaults: { ease: 'power2.inOut', duration: 0.45 } });
  tl.to(S, { duration: 0.0001 }, 0);
  tl.to(S, { enh: 1 }, 1);
  tl.to(S, { bez: 1, w: 16 / 9 }, 2);
  tl.to(S, { bezY: 1.15, bez: 0, side: 1, duration: 0.55, ease: 'power3.in' }, 3);
  tl.to(S, { wide: 1 }, 4);
  tl.to(S, { ultra: 1, w: 21 / 9, duration: 0.55 }, 5);
  tl.to(S, { duration: 0.5 }, 5.55);

  function setStep(i) {
    if (i === cur) return;
    cur = i;
    stage.setAttribute('data-step', i); fign.textContent = i + 1; ghost.textContent = GHOST[i];
    stks.forEach(function (s) { s.classList.remove('on'); });
    stks[i].classList.add('on');
    if (i > 0) whip.forEach(function (el, k) {
      gsap.killTweensOf(el);
      gsap.fromTo(el, { xPercent: -110, x: 0 }, { xPercent: 110, duration: 0.5 + k * 0.06, ease: 'power2.in', delay: k * 0.03 });
    });
    fitSticker();
  }
  var queued = false;
  function update() {
    queued = false;
    var r = track.getBoundingClientRect(), len = r.height - stage.offsetHeight;
    var t = clamp(-r.top / (len || 1), 0, 1) * SPAN;
    tl.time(t, false); apply();
    setStep(clamp(Math.floor(t - LAG), 0, 5));
  }
  function queue() { if (!queued) { queued = true; requestAnimationFrame(update); } }
  size(); update();
  window.addEventListener('scroll', queue, { passive: true });
  /* a resize that only changes the height (a phone's toolbar) changes nothing here */
  window.addEventListener('resize', function () { if (root.clientWidth !== lastW) { size(); queue(); fitSticker(); } });
  /* once the images arrive, re-evaluate which layers may hide */
  Object.keys(L).forEach(function (k) { L[k].addEventListener('load', queue); });
})();
