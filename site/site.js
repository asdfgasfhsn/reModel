/* reModel promo site. Plain JS; GSAP + ScrollTrigger only drive the pinned sequence. */
(function () {
  'use strict';
  var root = document.documentElement;
  var reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  var $ = function (s, c) { return (c || document).querySelector(s); };
  var $$ = function (s, c) { return Array.prototype.slice.call((c || document).querySelectorAll(s)); };
  var clamp = function (v, a, b) { return Math.max(a, Math.min(b, v)); };
  root.classList.add('js');

  /* ---------- reveals: wipes, lines, tiles and starbursts ---------- */
  var reveal = $$('.wipe,.rise,.ln,.slapin');
  $$('.lines').forEach(function (ul) { $$('.ln', ul).forEach(function (l, i) { l.style.setProperty('--i', i); }); });
  $$('.slapin').forEach(function (s, i) { s.style.setProperty('--d', (i % 3) * 0.14 + 's'); });
  /* A scroll scan rather than IntersectionObserver: clip-path hides an element from the observer, and
     it misses elements a fast scroll jumps over. */
  var pend = [];
  if (reduce) reveal.forEach(function (t) { t.classList.add('in'); });
  else reveal.forEach(function (t) {
    pend.push(t);
  });
  function scan(vh) {
    /* reveal anything whose top has entered the viewport, or that a fast scroll jumped over */
    pend = pend.filter(function (x) {
      if (x.getBoundingClientRect().top >= vh * 1.15) return true;
      x.classList.add('in');
      /* once a wipe has run, drop its clip-path altogether so the headline can never stay clipped */
      if (x.classList.contains('wipe')) setTimeout(function () { x.classList.add('done'); }, 800);
      return false;
    });
  }

  /* ---------- scroll progress: --p on [data-p] panels, hero parallax ---------- */
  var panels = $$('[data-p]'), shards = $$('[data-par]'), hero = $('.hero'), tick = false;
  function progress() {
    tick = false;
    var vh = window.innerHeight;
    scan(vh);
    panels.forEach(function (el) {
      var r = el.getBoundingClientRect();
      if (r.bottom < -200 || r.top > vh + 200) return;
      el.style.setProperty('--p', clamp((vh - r.top) / (vh + r.height), 0, 1).toFixed(4));
    });
    var hp = clamp(window.scrollY / (hero.offsetHeight || 1), 0, 1);
    shards.forEach(function (s) { s.style.setProperty('--py', (hp * parseFloat(s.dataset.par)).toFixed(1)); });
  }
  function queue() { if (!tick) { tick = true; requestAnimationFrame(progress); } }
  if (reduce) {
    /* static panels: the hardware panel shows its faded state, how-it-works its reprint */
    panels.forEach(function (el) { el.style.setProperty('--p', el.id === 'problem' ? '0.9' : '1'); });
  } else {
    window.addEventListener('scroll', queue, { passive: true });
    window.addEventListener('resize', queue);
    progress();
  }

  /* ---------- gameplay gallery: shots/gallery/<set>.webp ---------- */
  var GALLERY = [['bel', 'Behind Enemy Lines'], ['desert', 'Desert Tank'], ['doa', 'Dead or Alive'], ['gunblade', 'Gunblade NY'], ['hotd', 'The House of the Dead'],
    ['indy500', 'Indianapolis 500'], ['lastbrnx', 'Last Bronx'], ['overrev', 'Over Rev'], ['sgt24h', 'Super GT 24h'], ['vcop2', 'Virtua Cop 2']];
  var shots = $('.shots');
  if (shots) GALLERY.forEach(function (g) {
    var f = document.createElement('figure'), im = new Image(), cap = document.createElement('figcaption');
    f.className = 'shot'; im.alt = g[1] + ' gameplay'; im.width = 1280; im.height = 720; im.fetchPriority = 'low';
    im.onerror = function () { f.remove(); };
    im.src = 'shots/gallery/' + g[0] + '.webp'; cap.textContent = g[1];
    f.appendChild(im); f.appendChild(cap); shots.appendChild(f);
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

  /* ---------- decode every image once the page is idle, so nothing pops in ---------- */
  window.addEventListener('load', function () {
    $$('img').forEach(function (i) { if (i.decode) i.decode().catch(function () {}); });
  });

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
      var def = list.filter(function (l) { return l[0] === 'smooth'; })[0] || list[0];
      pick(def, def.chip);
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

  /* ---------- the pinned sequence ---------- */
  var gameName = $('#gameName'), stage = $('#stage');
  if (gameName && stage) gameName.textContent = stage.dataset.game + ', one frame';
  if (!(window.gsap && window.ScrollTrigger) || reduce || !stage) return;
  gsap.registerPlugin(ScrollTrigger);
  root.classList.add('js-seq');

  var plate = $('#plate'), pw = $('#pw'), capbar = $('.capbar', pw);
  var L = { nat: $('#lNat'), enh: $('#lEnh'), bez: $('#lBez'), side: $('#lSide'), wide: $('#lWide'), ultra: $('#lUltra') };
  var stks = $$('.stk', pw), whip = $$('.whip i'), fign = $('#fign'), ghost = $('#ghost');
  var S = { nat: 1, enh: 0, bez: 0, bezY: 0, side: 0, wide: 0, ultra: 0, w: 4 / 3 };
  var GHOST = ['4:3', '4:3', 'BEZEL', '16:9', '16:9', '21:9'];
  var H = 300, availW = 360, cur = -1;
  var LAG = 0.56, SPAN = 6.5;

  /* a layer hides the one under it only when it has fully faded in AND its image has loaded */
  function up(img, v) { return v >= 0.999 && img.complete && img.naturalWidth > 0; }

  function size() {
    var vw = root.clientWidth, vh = window.innerHeight;
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
    var a = stks[cur >= 0 ? cur : 0];
    if (a) {
      var vw = root.clientWidth, l = pw.getBoundingClientRect().left + a.offsetLeft, sw = a.offsetWidth, dx = 0;
      a.style.translate = '';
      if (l + sw > vw - 8) dx = vw - 8 - (l + sw);
      if (l + dx < 8) dx = 8 - l;
      a.style.translate = dx + 'px 0';
    }
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
    var s = stks[i]; void s.offsetWidth; s.classList.add('on');
    if (i > 0) whip.forEach(function (el, k) {
      gsap.killTweensOf(el);
      gsap.fromTo(el, { xPercent: -110, x: 0 }, { xPercent: 110, duration: 0.5 + k * 0.06, ease: 'power2.in', delay: k * 0.03 });
    });
  }
  var pin = null;
  function build() {
    if (pin) { pin.kill(true); pin = null; }
    size();
    var narrow = root.clientWidth < 700;
    pin = ScrollTrigger.create({
      trigger: stage, start: 'top top', end: '+=' + Math.round(window.innerHeight * (narrow ? 3.9 : 5.85)),
      pin: true, scrub: true, anticipatePin: 1,
      /* step k's picture changes over [k, k+0.55); its label, sticker and caption follow once the change is done
         (LAG), and the extra half step at the end gives the last view a full step of its own */
      onUpdate: function (self) { var t = self.progress * SPAN; tl.time(t, false); apply(); setStep(Math.max(0, Math.min(5, Math.floor(t - LAG)))); }
    });
    tl.time(0, false); apply();
  }
  build();
  var rt;
  window.addEventListener('resize', function () { clearTimeout(rt); rt = setTimeout(function () { build(); ScrollTrigger.refresh(); }, 150); });
  /* once the images arrive, re-evaluate which layers may hide */
  Object.keys(L).forEach(function (k) { L[k].addEventListener('load', apply); });
  setStep(0);

  /* the shards parallax relies on scroll listeners above; refresh pin positions after fonts and images settle */
  window.addEventListener('load', function () { ScrollTrigger.refresh(); });
})();
