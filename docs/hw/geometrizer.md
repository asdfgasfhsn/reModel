# Model 2 geometrizer and 3D rasteriser (2C-CRX, Top Skater)

Research notes for building (a) a display-list decoder/dumper and (b) a software
rasteriser. Everything here is a clean-room summary in our own words. Written 2026-11-05.

Confidence: **H** = two independent sources agree (for example MAME and the game's own
code or data); **M** = one credible source that is consistent with the rest; **L** =
single source, inference, or a guess the source admits to. **[MAME-only]** = only MAME
says so. MAME does not run topskatr (`MACHINE_NOT_WORKING`), so check those claims
against the game.

| Tag | Source |
|---|---|
| M2V | MAME master `src/mame/sega/model2_v.cpp`: geometry HLE, raster input, clipping, z-sort, projection, screen mix (fetched 2026-11-05) |
| M2RD | MAME master `model2rd.ipp`: scanline/texel/colour path |
| M2 | MAME master `model2.cpp` / `model2.h`: memory map, geo ports, vblank hook, ROM regions |
| TSG | **topskatr's own geometrizer program**: 4,096 TGPx4 words that the i960 uploads from program ROM 0x3FE40. Disassembled with `remodel copro-disasm` (see §1.4) |
| TSI | topskatr i960 code (`out/topskatr/program.lst`) |
| TR | Port-write trace of one gameplay frame, `out/topskatr/geo-4500.txt` (1,566 writes) |
| ROM | Our decode of the polygon ROM (.17/.21) and texture ROM (.27/.25) at addresses taken from TR |

MAME's own sources, as its header lists them: a disassembly of a 2B game's geometrizer
code, the Model 2B-CRX manual, and ElSemi's Direct3D geometrizer. We found no public copy
of the manual or ElSemi's notes. Supermodel emulates Model 3 (Real3D), a different
pipeline, so it is not used here.

---

## 0. Architecture (read first)

```
i960 --geo ports--> buffer RAM display list --(list DMA, jumps)--> GEOMETRIZER (2nd MB86235,
      0x800000-                0x900000-0x91FFFF                    program uploaded by the i960)
      0x807FFF                                                                 | FO0: 24-bit floats + command words
                                                                               v
                                                  Z-sort/clip + rasteriser (fixed function) --> framebuffer --> mixer with tiles
```

- On 2B and 2C the geometrizer is a DSP that runs **code uploaded by the game**. MAME says so
  (M2V header, "Model 2B and 2C hardware upload the geometry code to a DSP"), but it still runs
  its own HLE of a 2B program for every board. **H**
- **topskatr uploads its own geometrizer program** (TSI 0x47EC0-0x47F00). It sets bit 31 of
  0x00980008, writes 0x2000 dwords from ROM 0x3FE40 to 0x00804000, then clears bit 31. The
  result is a 4,096-word MB86235 program. It is not the copro program (that one comes from
  ROM 0x2262C), and it decodes cleanly with no invalid words. **H**
  - So 2C has a second TGPx4 after all. This bears on MD-8 in `mame-defects.md`: MAME counts
    these words and discards them.
  - Implementation consequence: Remodel can run TSG on a second instance of our MB86235
    interpreter (LLE). Then only the fixed-function raster side (§4-§6) has to be written
    from MAME's description.
- MAME notes that "Top Skater seems to use a slightly different geometry code that has a
  secondary transformation matrix". TSG confirms this and shows what is different (§2.3,
  §3.2). **H**

---

## 1. How the i960 hands over a display list

### 1.1 Geo port registers (i960 addresses)

| Address | R/W | Meaning | Source / conf. |
|---|---|---|---|
| 0x00800000 + fn·0x10 (offsets < 0x1000) | W | **Command port.** A write with value bit 31 = 0 to an offset whose low nibble is 0 appends one word to the list: `(value & 0x000FFFFF) \| (fn << 23)`, where fn = (offset >> 4) & 0x3F. | M2, **H** (TR decodes cleanly) |
| same, value bit 31 = 1 | W | Appends a **jump** word: `(value & 0x800FFFFF) \| (fn << 23)`. When the list DMA sees bit 31, it continues at buffer byte offset `word & 0x1FFFF`. | M2, M [MAME-only]; topskatr doesn't use it |
| offset bits 10-11 (0x400/0x800/0xC00) | W | **Eye mode** (centre select 1-3). MAME puts these bits into list-word bits 29-30, but **only when fn == 0x01**. | M2 (hack comment cites Sega Rally); see §9 |
| fn·0x10 + 4, + 8 | W | MAME's dead debug code calls these "command length" (data & 0x3F) and "data length" (data & 0x7F). MAME ignores the writes. | M2, L |
| 0x00801008 | W | **Write pointer**: buffer RAM byte offset (data & 0xFFFFF) where appends go. | M2 + TSI, **H** |
| 0x00802008 | R | Returns the current write pointer. | M2, M |
| 0x00803008 | R/W | **Read pointer**: byte offset where the geometrizer starts the next frame. | M2 + TSI, **H** |
| 0x00804000-0x00807FFF | W | **Data append**: any offset appends the dword at the write pointer, then adds 4. topskatr uses `stq` (4 words to +0..+C), and these land in order. With 0x00980008 bit 31 set, the same port is the geometrizer program upload. | M2 + TR, **H** |
| 0x00980008 | W | Geometrizer control. Bit 31 0→1 starts an upload; 1→0 ends it ("boot geo"). | M2 + TSI, **H** |
| 0x0098000C | R/W | Video control. Write bit 0 = 30 Hz (geometrizer starts only on even frames). Read bit 2 = frame parity (toggles every frame in 60 Hz render mode). | M2 + TSI, M |
| 0x10000000 | R/W (16-bit) | Render mode. Bit 0 = test mode (host access to the framebuffer, no 3D), bit 2 = 60 Hz (else 30 Hz), bit 14 unknown. topskatr writes **0x4004**; one routine sets and clears bit 0. | M2 + TSI, M |
| 0x0181C000 | W (byte) | Master Z-clip. 0xFF = disabled. topskatr writes 0xFF. | M2 + TSI, M |
| 0x10400000 | R | Polygon count (MAME: only Sky Target uses it). | M2, L |

**Command word check (TSG, H):** for every command word, the geometrizer compares fn =
(word >> 23) & 0x1F with (word >> 8) & 0x1F and with word & 0x1F. If they differ, it
falls into an error loop that toggles its PDR output bits. That is why the game always
writes the value `fn | fn << 8` (for example `0x0B0B` to 0x008000B0). A decoder should
flag words that fail this check.

### 1.2 Frame protocol and double buffering (TSI + TR, H for the game's side)

- At init (TSI 0x47E60), the game fills all 32K words of buffer RAM with `0x07800F0F`, a
  valid END command (MAME's reset default has the same value). It uploads the geometrizer
  program, then (0x48110) writes END at buffer offsets 0x00000 and 0x10000.
- **Two lists**, at buffer byte offsets **0x00000** and **0x10000** (i960 0x00900000 and
  0x00910000), 64 KB each. Index `b` is kept at RAM 0x583180.
- Each frame (TSI 0x481B8):
  1. The game builds the list at `b << 16` and ends it with END (fn 0x0F).
  2. It writes the read pointer: `b << 16`, the list just finished. If flag 0x5CDDCC is set,
     it writes the other buffer instead (presumably to re-show the previous list).
  3. It busy-waits until bit 2 of 0x0098000C changes, i.e. a frame boundary.
  4. It flips `b` and writes `b << 16` to the write pointer.
- **When processing starts:** MAME runs the whole list at the start of vblank, from the read
  pointer, if 60 Hz mode is on or the frame number is even (M2 `screen_vblank` → `geo_parse`;
  **M** [MAME-only], but consistent with the game's protocol). The rendered image appears
  when the screen is next updated, so 3D lags input by about one frame. In TSG, handlers
  poll flag input F0 and abort to the end-of-frame path when it is set. That looks like a
  hardware "frame time over" signal (L).
- **List DMA (L):** something in hardware walks buffer RAM from the read pointer, follows
  bit-31 jump words, and feeds the geometrizer's input FIFO. TSG only ever reads FI and has
  no jump handling, so jumps must be resolved before the FIFO. MAME also stops after 0x8000
  commands or at the end of buffer RAM.

### 1.3 Buffer RAM

128 KB at i960 0x00900000-0x0091FFFF, mirrored through 0x0097FFFF (M2, **H**; already in
`bus.rs`). The copro (1st TGPx4) also sees it at ext 0x400000 (see `mb86235.md` §3.3).

### 1.4 Getting the program and listing

`remodel copro-disasm FILE` disassembles any 32 KB image. **Caution:** it writes the
listing to `out/topskatr/copro.lst`, overwriting the copro listing. Extract bytes
0x3FE40-0x47E3F of the joined program ROM (low half from `.15`). The handler addresses
below refer to that listing.

---

## 2. Command set (the list as the geometrizer reads it)

Each command is one command word (fn in bits 23-27; bits 28-30 carry fn bit 5 and the eye
mode; bit 31 = jump), followed by operands. "TS" is the handler in topskatr's program;
"MAME" is MAME's HLE. "f" is an IEEE single.

| fn | Name | Operands after the command word | Output to rasteriser | TS | MAME / notes |
|---|---|---|---|---|---|
| 0x00 | NOP | none | the command | 0x008 | same, **H** |
| 0x01 | Object data (standard format) | tpa, tha, oba, obc (4) | polygon stream (§4.2) | 0x00B | same; model format §3.1, **H** |
| 0x02 | Direct data | tpa, tha, then 6 f (P0, P1); then repeated: attr, luma, dist, 3 f [+3 f if quad]; ends with attr & 3 = 0 | raster cmd 1 + values >> 8 | 0x1A0 | same (MAME sends cmd 2−1 = 1), **H** |
| 0x03 | Window data | 6 words: start, end, centre 0-3, each X in bits 27-16 and Y in bits 11-0 | repacks each as `0x00XXXYYY` | 0x1D4 | same, **H** (TR) |
| 0x04 | Texture/point RAM write | addr, count, count words | passed through | 0x1FD | same, **H** |
| 0x05 | Polygon RAM write | addr, count, count words | none | 0x206 | TS: addr must be < 0x01000000 and count < 0x8000, else error; words go to geometrizer ext address `addr`. MAME: bit 24 selects fast RAM, else slow RAM, both 32K words. M |
| 0x06 | Texture (lighting) parameters | index, count, then count × (param, coef f) | none | 0x214 | param: bits 0-7 diffuse, 8-15 ambient, 16-23 specular scale, 24-31 specular control; coef = LOD distance coefficient. 32 entries. MAME uses index >> 2; TS uses the index as is (TR always sends 0). M |
| 0x07 | Mode | 1 word | none | 0x22B | bit 0 = specular on; bit 1 = no normals stored (computed). topskatr: 3. **H** |
| 0x08 | Z-sort mode | 1 f | cmd + (f >> 8) | 0x233 | same, **H** (topskatr 8.0156) |
| 0x09 | Focal distance | 2 f (focus x, y) | none | 0x238 | same, **H** (topskatr 600/600, 250/250) |
| 0x0A | Light vector | 3 f | none | 0x23C | same, **H** |
| 0x0B | Matrix | 12 f: 3×3 rotation, then translation | none | 0x241 | same, **H** |
| 0x0C | Translation | 3 f | none | 0x250 | same, **H** |
| 0x0D | Data memory push | addr, count | count words from ext `addr`, raw | 0x255 | MAME: unsupported, logs only. TS implements a raw copy. M |
| 0x0E | Test | 32 words (1 << i), nblocks, then nblocks × (addr, count, checksum) | none | 0x262 | FIFO test plus polygon-ROM checksums (sums of the even/odd 16-bit halves). **H** |
| 0x0F | End | none | END marker | 0x2CA | same, **H** |
| 0x10 | Dummy | 1 word (discarded) | none | 0x2B2 | same, **H** |
| 0x11 | **TS:** object data, skinned compact format | tpa, tha, oba, obc | polygon stream | 0x379 | MAME: alias of 0x01 with the 10-word format, **wrong for topskatr** (§3.2). **H** |
| 0x14 | Log data write | addr, count, count words (4 bytes each) | cmd, addr, count·4, then one byte per word | 0x2B5 | same, **H** |
| 0x16 | LOD | 1 f | none | 0x2C7 | same, **H** (topskatr 65536.0) |
| 0x1A | **TS: invalid** | — | — | error | MAME: light vector |
| 0x1B | **TS:** secondary matrix | 12 f: rotation to the matrix-1 slot, translation to the T1 slot | none | 0x480 | MAME: overwrites the primary matrix. **Wrong for topskatr**, **H** |
| 0x1C | **TS:** copy matrix 0 to matrix 1 | none | none | 0x488 | MAME: translation |
| 0x12, 0x13, 0x15, 0x17-0x19, 0x1D-0x1F | **TS: invalid** | — | — | error (0x2E9 → 0x377 hang) | MAME aliases them to 0x02, 0x03, 0x05, 0x07-0x09, plus code upload (0x1D), code jump (0x1E) and end (0x1F) |

Commands topskatr sends in TR: 0x16, 0x07, 0x08, 0x03, 0x09, 0x06, 0x0A, 0x0B, 0x1B, 0x11
(at offset 0x110, and at 0x510, i.e. eye 1), and 0x0F. In TSI it also writes fn 0x04, 0x05
and 0x14. A typical frame: LOD, mode, z-sort, window, focus, parameters, then per object a
matrix (0x0B, plus 0x1B for skinned parts) and object data (0x11), then END. **H**

### 2.1 Object-data operands
- **tpa**: texture point (UV list) address, in 16-bit units. Bit 23 set means texture
  point RAM (64K × u16, written by raster cmd 4), otherwise the texture ROM region (§5.1).
  It is sent to the rasteriser masked to 24 bits. **H**
- **tha**: texture header address, in the same space. **H**
- **oba**: model address in geometrizer ext space, in words. Addresses from 0x800000 up are
  the **polygon ROM** (word index = oba & 0xFFFFF for topskatr's 1M words). Lower addresses
  are polygon RAM written by fn 0x05. TS checks `oba & 0xFFFF8000`. MAME: bit 24 = fast RAM,
  bit 23 = ROM, else slow RAM. M
- **obc**: polygon count. 0 means "until terminator" (MAME treats 0 as 0xFFFFF). topskatr
  always sends 0. **H**

### 2.2 Matrix layout (M2V + TSG, H)
12 floats: m0-m8 are the rotation and m9-m11 the translation. A point transforms as
x' = x·m0 + y·m3 + z·m6 + m9, y' = x·m1 + y·m4 + z·m7 + m10, z' = x·m2 + y·m5 + z·m8 + m11.
In other words, m0-m2 is the image of the X axis, and so on. Normals use the rotation only.

### 2.3 Secondary matrix (TSG, H)
TSG keeps two rotation slots (PR offsets 0 and 9) and two translations (B-RAM 0x30 and
0x33). fn 0x0B writes slot 0, fn 0x1B writes slot 1, and fn 0x1C copies 0 to 1. In the
fn 0x11 path, **bit 0 of each vertex's X word** picks the slot: TSG computes `(x & 1) · 9`
and `(x & 1) · 3` as the slot offsets. Vertices with that bit set sit at limb joints
(object 0x80177B: the flagged vertices are the x ≈ 4.5 ring, the rest are at x ≈ 3.95).
This is two-bone skinning of the skater's body.

---

## 3. Polygon model format (polygons ROM, 32-bit LE words; `.17` = low half)

All coordinates and normals are **IEEE single floats** in model space (M2V + ROM, **H**).
A model is two start vertices followed by a chain of polygon records.

### 3.1 Standard format (fn 0x01; MAME's only format) — M2V, H for other games
```
P0 (x,y,z)  P1 (x,y,z)                       6 words: the first edge
repeat:  attr | normal (x,y,z) | Pa (x,y,z) | Pb (x,y,z)    10 words per polygon,
                                                             always (a triangle pads Pb)
until attr & 3 == 0, or obc polygons
```
With mode bit 1 set, the normal words are present but skipped, and the normal is computed
from the vertices (cross product, normalised).

### 3.2 topskatr compact format (fn 0x11) — ROM + TSG, **H**
```
P0 (3 words)  P1 (3 words)
repeat:  attr | Pa (3) [| Pb (3) if quad]    7 words per quad, 4 per triangle; no normal words
terminator: a word with low 2 bits = 0
```
Verified on three objects from TR (0x81BAA0, 0x846CCC sky dome of radius about 12,500,
0x802442 skater part): the meshes are coherent and terminate cleanly. Normals are always
computed (TSG uses FRSQ to normalise). Bit 0 of each X word is the matrix select (§2.3).
It is a mantissa LSB, so using the value unmasked costs only 1 ulp.

### 3.3 Attribute word (M2V; field use confirmed by TSG masks 3, 0x3FFFF, 0x7C0000, 27-bit shift)
| Bits | Meaning | conf. |
|---|---|---|
| 0-1 | 0 = end of model; 1 = quad; 2 = triangle (MAME tests bit 0 for quad); 3 = quad? | H |
| 2-7 | unused (always 0 in the ROM samples) | M |
| 8-9 | **Link type**: how the next polygon's "previous edge" is formed (§3.4). Type 0 polygons are never drawn (culled), so they only advance the strip. | M [MAME-only] |
| 10-11 | Z-sort key: 0 = previous polygon's z, 1 = min z, 2 = max z, 3 = error (MAME: 1e10) | M |
| 12-16 | Texture header step, signed 5-bit: after this polygon, tha += step·4 (one header = 4 u16) | M |
| 17 | Double-sided (no backface cull) | M |
| 0-17 | sent to the rasteriser as is (`& 0x3FFFF`) | H |
| 18-22 | Lighting parameter index (fn 0x06 entry) | H |
| 23-24 | In topskatr data these copy the link bits 8-9 (geometrizer bookkeeping?) | L (observation) |
| 27-31 | Distance-coefficient index (fn 0x06 coef) | H |

### 3.4 Vertex linking (M2V, M [MAME-only]; the ROM meshes look right with it)
The rasteriser holds the previous edge (P0', P1'). Each polygon adds Pa, and Pb for a quad.
For a triangle, Pb := Pa. The polygon drawn is **[P1', P0', Pa, Pb]**, or [P1', P0', Pa] for
a triangle. The texture points (§5.2) are assigned in that order. Then the edge for the
next polygon is:

| link | new P0' | new P1' |
|---|---|---|
| 0 or 2 | Pa | Pb |
| 1 | P0' (kept) | Pa |
| 3 | Pb | P1' (kept) |

The tpa pointer advances by 2·nverts u16 after every polygon, drawn or not. A dumper should
print each record with its resolved 3-4 vertices.

---

## 4. Transform, lighting, projection, clipping, sort

### 4.1 Geometrizer per vertex and per polygon (M2V; the TS transform matches in TSG)
1. v' = M·v (+T). **H**
2. Lighting uses the view-space normal n and the polygon's new vertex Pa:
   - dotl = n·L, dotp = n·Pa.
   - Front face if dotp ≥ 0, else back face (adds 0x100 to the luma).
   - Diffuse term: 0 if dotl·dotp < 0, else |dotl|.
   - luma = clamp(diffuse_term·diffuse + ambient [+ specular], 0, 255).
   - Specular: s = 2·dotl·n.z − L.z, clamped ≥ 0. It is squared 1-3 times depending on the
     control value (0 = off), then scaled by the specular scale.
   - **M [MAME-only]**; the TS code differs in detail and should be checked by running TSG.
3. LOD distance = coef[attr >> 27] · |dotp| · lod. **M**
4. Focus: x ·= fx, y ·= fy (z unchanged), for every vertex. **H** (TSG: FMUL by A[0x33], A[0x34], and z by 1.0)
5. Output to the rasteriser as **24-bit floats: IEEE bits >> 8** (sign, 8-bit exponent,
   15-bit mantissa). TSG does an `LSR #8` before each FO0 write. **H**

### 4.2 Rasteriser input stream (M2V `model2_3d_push`; TSG framing)
- **Command words:** TSG sends `0xFF000000 | (list word >> 23)`: bits 0-4 = fn, bit 5 =
  fn bit 5, bits 6-7 = eye. MAME's HLE sends `word >> 23` without the 0xFF. The raster
  reads cmd = bits 0-3, "reverse" = bit 4 (stored, unused), centre select = bits 6-7. Data
  words always have top byte 0. **M**
- **Polygon data** (cmd 1) buffer: [0] tpa, [1] tha, [2-4] P0', [5-7] P1', then per
  polygon: [8] attr, [9] luma word (luma 8 bits at 15-22, backface at bit 23), [10]
  distance, [11-13] Pa, [14-16] Pb (quads only). After each polygon, slots 2-7 are
  relinked and parsing continues at [8]. MAME ends on attr & 3 == 0, but **TSG sends no
  terminator** (it returns to dispatch silently), so our rasteriser must also end on a
  command word (top byte 0xFF). **M** (inferred from TSG)
- **Window** (cmd 3): 6 words `0x00XXXYYY` with 12-bit signed fields: viewport start,
  viewport end, centres 0-3. Each window command **increments the window number**. **H** for the format
- cmd 4: addr, count, data. addr bit 23 → texture point RAM (u16[64K]), else log RAM
  (u8[32K]). cmd 8: z-adjust. cmd 0 / END: no-op / end of frame.

### 4.3 Culling and clipping (M2V, M [MAME-only])
A polygon is culled if:
- it is not double-sided and has the backface bit set;
- its link type is 0;
- master Z-clip ≠ 0xFF and 1/min_z > zclip (odd test, kept as is);
- or max_z < 0.

Survivors are clipped (Sutherland-Hodgman) against **4 side planes through the eye**,
derived from the window and the selected centre:
- left: c_x − vp_x0; right: vp_x1 − c_x; top: vp_y1 − c_y; bottom: c_y − vp_y0;
- each plane's normal is (±1, 0, d)/hypot(1, d), or the same in y.

There is **no near or far plane**. Clipping can produce up to 8 vertices. u and v are
interpolated linearly in view space.

### 4.4 Projection to screen (M2V; topskatr values from TSI)
- sx = xoff + c_x + x/z
- sy = (384 − c_y) + yoff − y/z

Here xoff = 84 + (int16)H_sync and yoff = 130 + (int16)V_sync. topskatr writes H = 0xFFAC
(−84) and V = 0xFFFE (−2) (TSI 0xA8F40), so **xoff = 0, yoff = 128** (**H** for the
values written, **M** for the formula). The window is (0,128)-(498,512), centre (248,320),
so screen centre = (248,192) and the viewport covers the whole 496×384 screen.
Window-space Y runs upward: sy = 512 − wy.

### 4.5 Z-sort, not a z-buffer (M2V; M2V's header cites the manual)
- The polygon's z (per attr bits 10-11) has the z-adjust exponent subtracted, then converts
  to a **16-bit 4.12 pseudo-float** key:
  - exponent+1 in the top 4 bits and 12 mantissa bits, rounded;
  - denormals for small values; 0 if negative; 0xFFFF on overflow.
  - The manual reportedly says 1.4.8 internally (**L**).
- Each key has a bucket. Each new polygon goes to the head of its bucket's list.
- **Order:** windows from last to first; within a window, keys from small (near) to large
  (far); within a bucket, newest first. MAME draws **front to back with a coverage
  ("fill") mask**: a pixel is written only if no earlier polygon covered it. This equals a
  back-to-front painter, with later windows always on top. **M** (the window rule is
  reported from Sega Rally's name entry)
- Translucency is not blending (§5.4), so painter order is enough. Zero Gunner needs the
  "previous z" start value to be 1e10. MAME aborts at 32,768 polygons per frame.

---

## 5. Texturing and colour (M2V/M2RD [MAME-only] unless noted)

### 5.1 Where things live on 2C
| Data | Location | Source / conf. |
|---|---|---|
| **Texel sheets** | Texture RAM bank 0 at i960 0x11000000 and bank 1 at 0x11200000 (MAME maps 2 MB each; the texel addressing uses 1 MB). **The i960 copies them from the main data ROM** (0x02000000+, chips .11/.12/.9/.10; TSI 0x3F9E0 sets the source to 0x2000000 + n·1 MB and the destinations, then copies incrementally across frames). | M2 + TSI, **M** |
| Texture **point lists (UVs) and headers** | The "textures" ROM region = **.27/.25** (`.27` = low half, 8 MB, read as u16), or texture point RAM (tpa/tha bit 23). | M2V + ROM, **M** (headers and UVs decode sensibly, §9) |
| Luma tables | 32 KB (256 × 128 bytes) at 0x11400000; byte data on the low half of 16-bit words. topskatr writes linear ramps 0..64 into tables 0-1 and copies 0x2080 bytes from ROM 0xB7D84 to table 32 onward (TSI 0x47F38). | M2 + TSI, **M** |
| Colour translation | 0x01810000-0x0181BFFF: three 16 KB tables (R at +0, G at +0x4000, B at +0x8000). Each is 32 rows (5-bit colour component) × 256 u16, low byte used. Column 0x40 = tile layer, columns 0-63 = 3D luma. | M2, M |
| Polygon colours | Palette RAM entries **0x1000-0x13FF** (xBGR555), at i960 0x01802000+ | M2RD, M |

**Sheet addressing.** Logically 2048×1024 texels, 4 bpp. Each u16 holds a 2×2 block:
| Bits | Texel |
|---|---|
| 15-12 | (x even, y even) |
| 11-8 | (x odd, y even) |
| 7-4 | (x even, y odd) |
| 3-0 | (x odd, y odd) |

The word index is (y/2)·512 + x/2. For x ≥ 1024, use x − 1024 and y ^ 1024 (the RAM is
1024 wide × 2048 tall). Within a 32-bit RAM word, even word indices are the low half.

### 5.2 Texture points
4 or 3 pairs per polygon, as u16 **13.3 fixed point**, stored **V first, then U**, for
vertices [P1', P0', Pa, Pb] in that order. **M**

### 5.3 Texture header (4 × u16 at tha)
| Word | Bits | Meaning | conf. |
|---|---|---|---|
| h0 | 0-2 / 3-5 | width = 32 << n, height = 32 << n | M |
| h0 | 6 / 7 | smooth wrap X / Y (only affects filtering at the seam; coordinates always repeat because sizes are powers of two) | M |
| h0 | 8 / 9 | mirror X / Y (mirrored repeat) | M |
| h0 | 10-11 / 12 | microtexture min-LOD / microtexture enable | L |
| h0 | 13 | translucent (texel 15 = transparent) | M |
| h0 | 14 | textured (else flat) | M |
| h0 | 15 | checker: draw every other pixel (x ^ y parity), used as a screen-door translucency | M |
| h1 | 0-7 | luma table number (× 128 bytes) | M |
| h2 | 0-5 / 6-10 | X / 32, Y / 32 position in the sheet | M |
| h2 | 12 | swap banks (level 0 from bank 1) | M |
| h2 | 13 / 14-15 | microtexture X (×128) / Y (×128), 128×128 in the other bank | L |
| h3 | 6-15 | colour: palette entry 0x1000 + n | M |
| h1 8-15, h2 11, h3 0-5 | | unknown | — |

**Mipmaps.** Level k sits at ((x − 2048) >> k) & 2047, ((y − 1024) >> k) & 1023, in bank
(k & 1) (bank-swapped if h2 bit 12). Level 1 is therefore at (1024 + x/2, 512 + y/2) in
the other bank, and so on down to 2×2.

**LOD per pixel.** The polygon's lod = 128·(exponent of distance − 127) + logram[the 15
mantissa bits]. The log table is uploaded by fn 0x14. The pixel's mip value is
2·log2(z)·128 − lod; the level is that >> 7, with fractional blending between levels.
Negative values blend in the microtexture. **L** (MAME derived this itself)

### 5.4 Pixel colour
- **Textured:**
  1. Fetch the texel bilinearly (MAME's choice; the hardware filter is unconfirmed, **L**):
     t = 8-bit filtered value.
  2. If translucent: texel 15 is transparent. Transparent texels borrow a neighbour's value,
     and a pixel with less than 50 % coverage is discarded. No alpha blending.
  3. lum = lumaram[table·128 + t/2] · polygon_luma / 256, capped at 63.
  4. RGB = (xlatR[c.r][lum], xlatG[c.g][lum], xlatB[c.b][lum]), where c = the 5-bit
     components of the palette colour.
- **Flat:** lum = polygon_luma >> 2; same lookup. Flat translucent polygons are not drawn
  at all in MAME (**L**).
- **Gamma:** MAME then applies a guessed gamma (MH-3: out = max(0, (v − 64)·255/191)). It is
  calibration, not hardware.

---

## 6. Final output and mixing

- **Framebuffer:** two banks at 0x11600000 / 0x11680000, 16-bit, 512 wide (MAME says
  xGGGGGRRRRRBBBBB, 512×400, but decodes R = bits 5-9, G = 10-14, B = 0-4 through the tile
  colour table). The host only uses them in test mode. Whether pixels are stored before or
  after translation is unknown. **L** [MAME-only]
- **Mixing order, back to front** (MAME screen update; **M** [MAME-only]):
  1. palette pen 0;
  2. tile layers B (opaque) then A, **low-priority** half;
  3. the 3D image;
  4. all tile layers, **high-priority** half (see `tilemap.md` for the priority bit).

  MAME treats 3D pixels of exact RGB 0 as transparent, which is a MAME artefact. Use a
  coverage mask instead.
- **Coordinates:** 3D covers the 496×384 visible area. Tiles ignore the sync registers;
  only 3D is offset (§4.4). If no new frame came from the geometrizer, MAME shows the
  previous 3D image again.

---

## 7. Implementation notes

**(a) Display-list decoder/dumper**
1. Model the geo ports: write pointer, read pointer, appends, and the `fn | fn << 8` check.
   Optionally keep the eye bits for any fn (§9).
2. At each frame boundary, walk buffer RAM from the read pointer: follow bit-31 jumps, stop
   at END, and cap at 0x8000 commands.
3. Decode operands per the §2 table. For 0x01/0x11, resolve the model (§3.1 or §3.2),
   printing linked vertices, attr fields, texture header and UV pairs (§5).
4. Validate against TSG. Run the uploaded program on a second MB86235 interpreter, feed it
   the list words as FI, and diff its FO0 stream against the HLE output. TR plus
   `make run --geo-trace` give test vectors.

**(b) Software rasteriser**

Take the FO0 stream (from TSG or an HLE) through §4.2 → §4.3 cull/clip → §4.5 buckets →
§4.4 project → scan-convert with perspective-correct u/z, v/z, 1/z → §5.4 shade → coverage
mask → mix (§6).

- Start with point-sampled level-0 textures and no mipmaps. That is enough to see the game.
- Buffer RAM writes from the geo ports must actually land in RAM. Currently `bus.rs` only
  counts them, and `buffer-4500.bin` is still all `0x07800F0F`.

---

## 8. TODO, hack and guess markers in MAME

- Z-sort "not entirely understood": 1.4.8 vs 4.12 (M2V header).
- Older games need small viewport/centre offsets, "coordinate system worked out from the
  2B spec" (M2V).
- Data memory push (0x0D), code upload (0x1D) and code jump (0x1E) are unsupported stubs.
- The eye-mode hack applies only to fn 1 (M2 `geo_w`).
- Flat translucent polygons are skipped. The Virtua Striker luma cap is 0x3F (a hack). The
  gamma table is a calibration guess (MH-3).
- Buffer RAM reset fill: "HW can probably parse this at will somehow" (M2). topskatr does
  this fill itself.
- The geometrizer program upload is counted and ignored. Every board runs MAME's 2B-derived
  HLE, with a special "mesh" mode only for Last Bronx (0x1A-0x1C).
- Exceeding 32,768 polygons is fatal; an unknown raster command is fatal.

## 9. Candidate MAME defects (for `mame-defects.md`; not added there)

1. **fn 0x11 / 0x1B / 0x1C differ on topskatr** (*Confirmed* by TSG + ROM). MAME parses
   0x11 as the 10-word standard format and 0x1B as an overwrite of the primary matrix.
   topskatr's geometrizer uses the compact skinned format and a secondary matrix (§2.3,
   §3.2). This is enough to garble all of topskatr's 3D in MAME.
2. **Eye mode only for fn 1** (*Suspected*). topskatr sends object data as fn 0x11 at port
   offset 0x510 (eye 1), right after a window whose centre 1 differs: (48,167) vs (248,320).
   MAME drops the eye bits, so these objects would be drawn at centre 0. TSG passes word
   bits 28-30 straight to the raster command, so the interface probably maps address bits
   4-11 to word bits 23-30 for every fn.
3. **fn 0x06 index** (*Suspected*): MAME uses the index >> 2, TSG the index unscaled.
   Moot while the game sends 0.
4. **Geometrizer upload ignored on 2B/2C** (*Known limitation*): the root of item 1.

---

## Checks against topskatr (empirical)

| Question | Result |
|---|---|
| Does TR fit MAME's port packing (fn << 23, low 20 bits kept)? | Yes. All 1,566 writes decode into well-formed commands with the expected operand counts. |
| Floats on the list and in the model ROM? | IEEE single (matrices are orthonormal and coordinates coherent). |
| Raster input encoding | float >> 8 (TSG `LSR #8` before FO0) |
| Command dispatch | TSG jump table at A-RAM 0x40-0x5F (TSG init 0x2EF-0x30E); 0x2E9 = invalid |
| Model format of fn 0x11 | Compact, no normals, 7/4 words; matrix select in the X LSB |
| Texture headers at TR's tha (e.g. ROM16 0x1AE88, 0x2001C0) | Decode with MAME's layout: 32-512 texel sizes, positions on 32-texel boundaries, colours 0x040-0x1F0 |
| Double buffer | List halves at 0x00000 / 0x10000; read pointer = the list just finished; flip on the parity bit |
| 3D screen offsets | H = −84, V = −2 → xoff 0, yoff 128 |

## Top uncertainties

1. The raster-side semantics (linking, culling, z-key, clip planes, LOD, colour formula) are
   all [MAME-only]. There is no hardware document we can read; the 2B-CRX manual is not
   public.
2. How the list DMA feeds the geometrizer FIFO (jumps, timing, F0 abort), and whether
   processing really starts at vblank.
3. Whether the rasteriser ends polygon data on the 0xFF command marker (TSG sends no
   terminator).
4. Texture filtering (bilinear?), translucency rules, and mipmap/microtexture details.
5. Eye/centre-select mapping for functions other than 0x01.
6. TS lighting and backface rule: read the TSG fn 0x11 handler (0x3AD-0x47F) or run it.

## Remodel status (running TSG)

Remodel runs the uploaded geometrizer program on a second MB86235 interpreter (`crates/model2/src/geo.rs`):
- The geo ports build the list in buffer RAM. Command words are `(v & 0xFFFFF) | fn << 23 | eye << 29`, and data appends at the write pointer.
- At each vblank, F0 is pulsed and the list at the read pointer is streamed into the input FIFO, kept full to depth 8, until the program goes idle.
- Result for a topskatr gameplay frame: a 1,644-word list becomes 33,376 rasteriser words (z-sort, 5 windows and 87 object blocks), with no command-check errors.
