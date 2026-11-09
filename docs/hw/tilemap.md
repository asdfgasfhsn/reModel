# Model 2 tile layer (System 24 tilemap generator)

Research notes for drawing the 2D text/HUD/menu layer. Clean-room summary in
our own words; no source code was copied. Written 2026-11-05.

Confidence: **H** = two independent sources agree; **M** = one credible
source, consistent with others; **L** = single source, inference, or odd
behaviour in the source. **[MAME-only]** = only MAME says so, and MAME does
not run topskatr (`MACHINE_NOT_WORKING`). Check against what the game writes.

| Tag | Source |
|---|---|
| S24 | MAME master `src/mame/sega/segaic24.cpp/.h` (tilemap, sprite, mixer devices), fetched 2026-11 |
| M2 | MAME master `model2.cpp` (memory map, palette write), `model2_v.cpp` (screen update), `model2rd.ipp` (polygon colour path) |
| SYS24 | MAME master `segas24.cpp` (System 24 driver: mixer use, palette size) |
| M150 | MAME 0.150 `drivers/model2.c` (older, simpler palette handling) |
| CMD | Charles MacDonald, "Sega System 24 hardware description" v0.3 (2003-03-27), `s24tech.txt` (via web.archive.org of cgfm2.emuviews.com). A 2013 revision exists on Sega Retro ("added ABSEL override mode, line scroll description") but the site blocks fetchers, so we have not read it. |

Chips: on System 24 the tilemap generator is the **315-5292** (CMD). The
mixer is the **315-5294** and the colour encoder the **315-5242** (CMD). The
315-5195 is a System 16/24 memory mapper, not a mixer. Model 2 reuses only the
tilemap generator. Its colour output goes through Model 2's own palette and
colour-translation logic (M2, M). Supermodel's Model 3 "tile generator" is a
different, later chip: 4/8 bpp, different register layout. Do not use it as a
reference, except as loose evidence that the design family exists.

---

## 1. Character (pattern) RAM — i960 0x01080000–0x010FFFFF

| Fact | Value | Source / conf. |
|---|---|---|
| Size | 0x80000 bytes (512 KiB), fully populated on Model 2. System 24 has only 128 KiB. | S24 + M2 + CMD, **H** |
| Tile size | 8×8 pixels | S24 + CMD, **H** |
| Depth | 4 bpp, packed nibbles (no bitplanes) | S24 + CMD, **H** |
| Bytes per tile | 32 (4 bytes per row × 8 rows); rows top to bottom | S24 + CMD, **H** |
| Tile count | 0x80000 / 32 = **16384** (0x4000). Model 2 tile number mask is 0x3FFF. | S24 + M2, **H** |
| Address of tile n | 0x01080000 + n·32 + row·4 | derived, **H** |
| Mirror | +0x100000 (0x01180000) | M2, M [MAME-only] |

**Nibble order** (S24 gfx layout, M; consistent with CMD's "8 nibbles per 4
bytes" wording for sprites). Each row is two 16-bit words. In each word the
**leftmost pixel is the most significant nibble**:

| row word | bits 15–12 | 11–8 | 7–4 | 3–0 |
|---|---|---|---|---|
| word 0 (byte offset +0) | px 0 | px 1 | px 2 | px 3 |
| word 1 (byte offset +2) | px 4 | px 5 | px 6 | px 7 |

The i960 is little-endian, so a 32-bit row read at the row address gives:
px0 = bits 15–12, px1 = 11–8, px2 = 7–4, px3 = 3–0, px4 = 31–28, px5 = 27–24,
px6 = 23–20, px7 = 19–16. In byte terms: byte +1 holds px0 (high nibble) and px1,
byte +0 holds px2 and px3, byte +3 holds px4 and px5, and byte +2 holds px6 and px7.
If text comes out as garbage, swapping the two words or the bytes is the first thing to try.

Pen 0 is transparent, except on the bottom layer (see §3) (S24, H for System 24 behaviour).

## 2. Tile ("name table") RAM — i960 0x01000000–0x0100FFFF

64 KiB of 16-bit words. Mirror at 0x01100000 (M2, M); **confirmed** for the whole 1 MB
(tile RAM, registers and character RAM at +0x80000) by `lastbrnx`: its gauge routine builds the
fill patterns of the life gauge by tile number `0x8080` and up, the map entry with its priority
bit, and writes them at `0x01080000 + (entry << 5)` = `0x01181000`; the map entries then draw
tile `0x80`. Only the low 20 address bits reach the RAM, so the 2 MB window repeats it (reModel
maps `0x01100000-0x011FFFFF` onto the lower megabyte; with separate RAM there the gauges drew
empty). Offsets below are **byte
offsets** from 0x01000000. A 16-bit word at byte offset 2k is word k in MAME.

### 2.1 Layout

| Byte offset | Contents | Source / conf. |
|---|---|---|
| 0x0000–0x1FFF | map 0: plane **A**, "scroll" map | S24 + CMD, **H** |
| 0x2000–0x3FFF | map 1: plane **A**, "window"/alternate map | S24 + CMD, **H** |
| 0x4000–0x5FFF | map 2: plane **B**, scroll map | S24 + CMD, **H** |
| 0x6000–0x7FFF | map 3: plane **B**, window/alternate map | S24 + CMD, **H** |
| 0x8000–0x8FFF | line-scroll tables, 0x400 bytes (512 words) per map n at 0x8000 + 0x400·n | S24, M (CMD: "row/column scroll data") |
| 0xA000–0xA00F | scroll/control registers (§2.3). CMD says the first 16 words are used; MAME reads only 8. | S24 + CMD, M |
| 0xC000–0xCBFF | plane A window mask (§2.4) | S24, M (CMD: "window control?") |
| 0xD000–0xDBFF | plane B window mask | S24, L |
| rest | unused or unknown | CMD |

Each map is **64×64 tiles = 512×512 pixels**, row-major. Entry (col, row) is at
map_base + (row·64 + col)·2 (S24 + CMD, **H**). So there are 4 tilemaps arranged as
2 planes × 2 maps. Within a plane, the two maps are combined by the mask or by a
split; they never overlap, so each plane works as one 2D surface.

### 2.2 Entry format (16 bits)

| Bits | Meaning | Source / conf. |
|---|---|---|
| 13–0 | tile number (0–16383) | S24 (mask 0x3FFF on Model 2) + CMD, **H** |
| 14–7 | colour bank, 8 bits (256 banks × 16 colours) | S24 + CMD, **H** |
| 15 | priority: 1 = in front of 3D, 0 = behind (§3) | S24 + M2, M (CMD: "unknown") |

**The fields overlap (bits 7–13).** The hardware does not have separate
fields. It simply uses bits 14–7 as the palette bank and bits 13–0 as the
pattern address. Pixel colour index = (entry bits 14–7) × 16 + pen, which is
the same as ((entry & 0x7F80) >> 3) | pen, a 12-bit index (0x000–0xFFF). As a
result, tile n is always drawn with palette bank (n >> 7) & 0x7F, plus 128 if
bit 14 is set. Games must place their character data so that each group of 128
consecutive tiles sits next to the palette bank it uses. On System 24 (mask 0xFFF,
CMD says bits 12–13 have no effect) bits 12–14 select the colour only. On Model 2,
only bit 14 is free.
An implementer just applies both formulas to the same word.

### 2.3 Scroll / control registers (byte offsets 0xA000+) [MAME-only, M]

| Byte offset | Register |
|---|---|
| 0xA000 + 2n (n = 0..3) | **HSCROLL** of map n |
| 0xA008 + 2n (n = 0..3) | **VSCROLL** of map n |

HSCROLL bits: 0–8 = X scroll (0–511). Bit 9 = in split mode, which map is on the
left (see below). Bit 15 = use the line-scroll table instead of bits 0–8.

VSCROLL bits: 0–8 = Y scroll. **Bit 15 = map disabled** (not drawn). On map 0
and map 2 (the first map of each plane), **bits 14–13 = plane mode**, which
applies to the whole plane:

| mode | Behaviour (per MAME) |
|---|---|
| 0 | normal: the two maps of the plane are chosen per 8-pixel cell by the window mask (§2.4). Each map uses its own scroll registers. |
| 1 | horizontal split (top/bottom). Both maps use the first map's H/V scroll. Split line = (−VSCROLL) & 0x1FF. Bit 9 of (−VSCROLL) set means the first map is on top. **L**: the same value serves as both scroll and split position. |
| 2, 3 | vertical split (left/right) at x = HSCROLL & 0x1FF. HSCROLL bit 9 set means the first map is on the left. With line scroll on, each line's table entry gives both that line's scroll and its split point. Equivalently: map 0 and map 1 side by side as one 1024-pixel plane scrolled by the 10-bit value. **Confirmed** by schamp (see Checks below) |

**Scroll direction** (S24, M): for screen pixel (x, y), the map pixel is
mx = (x − HSCROLL) mod 512 and my = (y + VSCROLL) mod 512. H and V have
**opposite sign**. Both wrap at 512.

**Line scroll** (HSCROLL bit 15 set): word y of map n's table (at byte 0x8000 + 0x400·n + 2y) replaces
HSCROLL for screen line y. Its bits 0–8 are the X scroll. The table is indexed by
**screen** line (0–383), not map line. V scroll still applies normally (S24, M;
CMD 2013 adds a "line scroll description" we could not read). MAME does not
support per-column scroll.

### 2.4 Window mask (plane A at 0xC000, plane B at 0xD000) [MAME-only, M/L]

For each screen line y there are 4 words (8 bytes) at mask_base + 8y, covering
512 pixels. Word w covers screen x = 128w … 128w+127. Within a word, **bit 15 is the
leftmost 8-pixel cell** and bit 0 the rightmost. Bit = 0: the plane's first map
(0 or 2) shows in that cell. Bit = 1: the second map (1 or 3) shows. The mask is in
**screen** coordinates; it does not scroll. The used size is 384 lines × 8 bytes =
0xC00. The typical use is a fixed HUD map in the second map that does not scroll.
For a quick text viewer, it is reasonable to draw map 0 where the mask bit is 0 and
map 1 where it is 1.

### 2.5 Other registers (M2) [MAME-only]

| i960 address | MAME name | MAME behaviour |
|---|---|---|
| 0x01020000 | ABSEL | write ignored ("always 0"). CMD 2013 describes an "ABSEL override mode" we have not read. **L** |
| 0x01040000 | H sync | 3D x offset = 84 + (signed 16-bit value). The tile layer is **not** moved. |
| 0x01060000 | V sync | 3D y offset = 130 + signed value. The tile layer is not moved. |
| 0x01070000 | sync switch | ignored |

All are also mirrored at +0x100000. Log what topskatr writes here.

## 3. Layer order, mixing, transparency

The System 24 mixer (315-5294, mapped at 0x404000 on System 24) has per-layer
priority registers. Each is 3 bits; register 13 bit 0 blanks the screen (SYS24, CMD).
**Model 2 has no mixer in its memory map**, and MAME uses a **fixed order** (M2, M,
[MAME-only]). From back to front:

1. Fill with palette entry 0.
2. Plane B (maps 3 and 2), drawn **opaque**: pen 0 shows bank·16 + 0. Every B tile
   is drawn in this pass, whatever its priority bit.
3. Plane A (maps 1 and 0), priority-0 tiles only, with pen 0 transparent.
4. **3D polygons** on top of the result.
5. All maps (B first, then A), **priority-1 tiles only**, with pen 0 transparent, on
   top of the 3D.

So bit 15 = 1 means "HUD in front of 3D", and bit 15 = 0 means "background behind 3D".
Within a pass, A is in front of B. Two MAME artefacts to be aware of:
- MAME composites the tile layer in RGB and treats **pure black (RGB 0,0,0) as
  transparent**. Real hardware surely keys on pen 0 / priority, not on colour.
- MAME's own TODO says vcop2 shows a B-over-A priority bug, so the real A/B/3D
  priority logic is probably richer than this. **L**

Transparency: pen 0 of every tile is transparent, except in the bottom plane
(B), whose pen 0 is the backdrop (S24 + SYS24, H for System 24; M for Model 2).

## 4. Palette and colour translation

### 4.1 Palette RAM — 0x01800000–0x01803FFF (M2) [MAME-only, M]

0x4000 bytes = **8192 16-bit entries**.

| Entries | Bytes | Used by |
|---|---|---|
| 0x0000–0x0FFF | 0x01800000–0x01801FFF | **tiles**: index = bank·16 + pen (§2.2) |
| 0x1000–0x1FFF | 0x01802000–0x01803FFF | polygon base colours (polygon colour field + 0x1000; only 10 bits are used) |

Entry format on Model 2 is **xBGR555**: bits 4–0 R, 9–5 G, 14–10 B, bit 15
unused (MAME masks it off for polygons). This is **not** the System 24 layout:
System 24 puts each colour's LSB in bits 12–14 and uses bit 15 as shade (CMD).
On Model 2 the 5-bit values only index the translation table; they are not fed
straight to a DAC. M150 used plain pal5bit xBGR555, which is a good first
approximation (M).

### 4.2 Colour translation table — 0x01810000–0x0181BFFF (M2) [MAME-only, M/L]

0xC000 bytes. There is one table per channel: **R at +0x0000, G at +0x4000, B at
+0x8000**, 0x4000 bytes each. Each channel table has 32 rows of 0x200 bytes (256
16-bit words), one row per 5-bit input level. The column is a brightness/"luma"
index, and the **low byte of the word** is the 8-bit output level.

- **Polygons** use column = luma 0–63. Luma comes from the per-texel lumaram
  lookup (2C: 0x11400000, byte-wide) scaled by the polygon's light level.
- **Tiles** (and the direct-framebuffer test mode) use the fixed **column 0x40**,
  word 64, byte 0x80 within the row. MAME calls this the "scroll colour table".
  So tile output R = byte at 0x01810000 + R5·0x200 + 0x80. G uses base 0x01814000
  and B uses base 0x01818000. MAME re-evaluates every tile colour whenever a word at
  that column is written.
- Tiles **do** go through the translation table, but **not** through lumaram.
  Lumaram is used only by polygons.
- MAME then applies its own monitor-curve guess: out = max(0, (v − 64)·255/191).
  This is a calibration hack ("optimal for vf2"), not hardware.

If the game has not filled column 0x40 yet, fall back to pal5bit xBGR555.

## 5. Screen geometry

| Fact | Value | Source / conf. |
|---|---|---|
| Visible | **496 × 384** | M2 + SR (peripherals.md), **H** |
| Total | 656 × 424 at 16 MHz pixel clock, 57.524 Hz | M2 + AC, **H** (see peripherals.md) |
| Tile plane | 512 × 512 per map. The screen shows a 496×384 window into it at (−HSCROLL, +VSCROLL), wrapping at 512. | S24, M |
| Tile origin | Screen (0,0) = top-left of the visible area. The tile layer ignores the H/V sync registers in MAME. | M2, M |
| 3D origin | MAME shifts polygons by 84+H / 130+V. The 2B manual gives 3D screen coordinates (8,90)–(504,474), Y up. | M2, M |

With zero scroll, tile column c and row r land at screen (8c, 8r). The visible area
shows 62 × 48 tiles; columns 62–63 and rows 48–63 are off-screen.

reModel's wide views (an enhancement) extend plane B past the frame's
sides, round the 512-pixel map, on rows where its map is scrolled and
3D is on screen; unscrolled rows and plane A stop at the frame. Games
that pan a backdrop on plane B (doa's skies) scroll it, while select
screens that lay out a page on it don't (docs/games/doa.md).

---

## Biggest uncertainties

1. Nibble/word order of char RAM under i960 32-bit access. It follows from MAME's
   gfx layout on a 16-bit view; check it against a known font.
2. Model 2 A/B/3D priority: there is no mixer, and MAME's fixed order has a known bug.
   The meaning of bit 15 (front/behind 3D) is inferred from MAME's draw passes.
3. Window mask, split modes and ABSEL. These are only in MAME, and CMD's 2013
   revision is unread.
4. The colour translation column 0x40 and the gamma. Any colour error is cosmetic for
   reading game state.
5. Registers at 0xA010–0xA01F (CMD says 16 words are used) are unexplained.

---

## Checks against topskatr (empirical)

These come from rendering the game's own tile RAM (`make run SNAP=…`). They
outrank the sources above.

| Question | Result |
|---|---|
| Character format (8x8, 4bpp, leftmost pixel in each 16-bit half's top nibble, little-endian under i960 access) | **Confirmed**: boot text, logo and title render cleanly |
| Entry format, with tile and colour bank overlapping in bits 7-13 | **Confirmed**: logo and title colours are correct |
| Palette xBGR555, tiles use entries 0x000-0xFFF | **Confirmed** (plain xBGR555, no translation table) |
| Plane modes | The game uses mode 2 (left/right split) on plane A, with line scroll enabled on map 0 |
| Split mode: is the line-scroll entry also the X scroll? | **Yes** (as MAME reads it). In split mode the plane is one 1024-pixel-wide surface, map 0 then map 1, scrolled by the low 10 bits of the first map's value (line-scroll entry or HSCROLL): bits 0-8 are both maps' X scroll and the split point, bit 9 says which map is on the left. Confirmed from schamp's story captions ("REVENGE OF DR. ..."), a line longer than one map that scrolls 8 pixels a frame across both maps (the caption lines' entries run from 0x1FE0 down to 0x19F8); read as a split point only (our first reading, from topskatr), the split swept left over two still maps and cut letters out mid-word. topskatr's attract text ("INSERT COIN(S)", "CREDIT 0", lines 320-360, map 1 blank there) now scrolls out to the left and back in from the right every 200 frames, the second line a little behind the first. fvipers uses it for comb transitions (alternate lines scrolled +x and -x: the SEGA logo on plane B, the PLAYER SELECT header on plane A). topskatr's earlier "jumps sideways" probably came from looking at snapshots 20 frames apart. *Unconfirmed:* not compared with hardware footage. |
| Visible window | 496x384 starting at map (0,0) with zero scroll; attract text is centred |
