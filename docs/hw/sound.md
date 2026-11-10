# Model 2C sound: 68EC000 + SCSP, and the DSB2 MPEG board

Research notes for sound emulation in `topskatr` (Model 2C-CRX). Everything
here is paraphrased; no emulator code was copied. Read
`docs/hw/peripherals.md` §3 first: it covers the i960-side UART, the baud
rate and the i960 IRQ wiring, and this file does not repeat them.

Confidence: **H** = Sega/Yamaha manual or datasheet, or the game's own ROM
shows it directly; **M** = several emulators agree (MAME, Supermodel,
Mednafen), or one emulator plus ROM evidence; **L** = one source, an
inference, or a guess. **[MAME-only]** = only MAME says so, and MAME does not
run topskatr (`MACHINE_NOT_WORKING`). **ROM** = from reading topskatr's own
code or data in this repo (addresses given). The *facts* such code shows are
H. What they *mean* is graded on its own.

Sources (short tags used below):

| Tag | Source |
|---|---|
| SCSP | Sega SCSP User's Manual (1997), English HTML. Index: https://www.infochunk.com/saturn/segahtml_en/hard/scsp/index.htm. Pages cited by file name, e.g. `p04_11` = https://www.infochunk.com/saturn/segahtml_en/hard/scsp/hon/p04_11.htm. Many tables are images and are missing from the text, so the EG rate tables, the pitch formula and the DSP diagrams aren't machine-readable. |
| MAME | MAME master, fetched 2026-11-05: `src/mame/sega/model2.cpp` (sound map and config, 2C PCB note), `src/mame/sega/model3.cpp` (DSB/DSB2 PCB notes), `src/mame/sega/dsb2.cpp`, `src/devices/sound/scsp.cpp`, `scspdsp.cpp`. https://github.com/mamedev/mame |
| SM | Supermodel master: `Src/Model3/SoundBoard.cpp`, `Src/Model3/DSB.cpp`, `Src/Sound/SCSP.cpp`, `Src/Sound/SCSPDSP.cpp`. https://github.com/trzy/Supermodel |
| MDF | Mednafen Saturn module `ss/scsp.inc` (via the libretro mirror https://github.com/libretro/beetle-saturn-libretro, `mednafen/ss/scsp.inc`). This is the most hardware-tested SCSP core: its author ran test programs on a real Saturn. GPL, so it is read for facts only. |
| ROM | topskatr ROMs in `rom/`: sound program `mpr-19759.31`, DSB2 program `mpr-19760.2s`, MPEG data `mpr-19747..19750`, and i960 program (`out/topskatr/program.lst` from `make analyze`). |
| BLOG | "The Digital Sound Board 2 demystified" (2018). https://gamemaster14.blogspot.com/2018/01/the-digital-sound-board-2-demystified.html |

---

## 0. Summary

| Item | Value | Conf. |
|---|---|---|
| Sound CPU | MC68EC000 at 11.2896 MHz (45.1584 MHz / 4) | M (MAME clock; PCB part MC68EC000FN12) |
| SCSPs on 2C | **one** Sega 315-5687 (YMF292-F) at 22.5792 MHz, so 44.1 kHz output | M (MAME 2C PCB note lists one 315-5687; Model 3 has two) |
| Sound RAM | 512 KB at 68k `0x000000`, the SCSP's whole memory | M (MAME); ROM sets MEM4MB=1 |
| Sound program | `mpr-19759.31`, 512 KB, at 68k `0x600000` | H (ROM: reset PC `0x600120`) |
| Sample ROM | `mpr-19745.32` + `mpr-19746.34`, 8 MB, at 68k `0x800000–0xFFFFFF`, **68k-only**. Model 2A `srallyc` (and `vf2`): four 2 MB chips `.31 .32 .36 .37` back to back, the order fixed by the sound program's sample table (loops repeat after their ends, chips' data ends at their slots' last samples; docs/games/srallyc.md, vf2.md) | M (srallyc, vf2: H, from the ROMs and by running them) |
| SCSP regs | 68k `0x100000–0x100EE3` | H (SCSP) + ROM (`lea $100000,a5`) |
| Board latch | 68k `0x400001` (byte): bits 3-0 look like LEDs; bank bits unused by topskatr | ROM + L |
| Sound driver | ASCII string "Ver3.2.8\|96 10/28\|CRX_drv\|byHiroF" at ROM `0x600100` | H (ROM) |
| 68k IRQs (topskatr) | MIDI-in = **level 3**, timer A = level 2, timer B = level 1 | H (ROM: SCILV/SCIEB values + vector table) |
| Host link | i960 sends fixed **3-byte MIDI-style messages** (`An dd dd`, `Bn dd dd`), plus one `0xFF` at init | H (ROM) |
| DSB2 CPU | MC68EC000 at 11 MHz (33 MHz / 3); 128 KB program ROM; 128 KB RAM at `0xF00000` | M (MAME PCB note, SM); MAME's device uses 8 MHz |
| MPEG decoder | **Sega 315-6028A** (QFP100, 12.288 MHz; NEC DSP suspected, unpopulated uPD77016 footprint beside it) | L (MAME PCB note for Model 3 DSB2 837-12273; the topskatr board isn't documented) |
| MPEG data | MPEG-1 Layer II, **32 kHz, 128 kbit/s, stereo**, stored as a raw byte stream (no byte swap) | H (ROM: 26,650 chained frames) |
| DSB2 output | own DACs (2x NEC uPD63210), analog out to the cabinet amp | M (PCB note, BLOG) |

---

## 1. Sound board memory map (68EC000 side)

### Map (MAME, M; cross-checked with ROM where noted)

| 68k address | Size | What | Notes |
|---|---|---|---|
| `0x000000–0x07FFFF` | 512 KB | Sound RAM (SCSP DRAM) | Shared with the SCSP. The SCSP sees it at its own `0x00000–0x7FFFF`. |
| `0x100000–0x100EE3` | | SCSP registers | Slots `0x000–0x3FF`, common `0x400–0x42F`, sound stack `0x600–0x67F`, DSP `0x700–0xEE3` (SCSP p04_10). MAME maps `0x100000–0x100FFF`. |
| `0x400000–0x400001` | byte at `0x400001` | Board control latch (write) | See below. |
| `0x600000–0x67FFFF` | 512 KB | Program ROM `mpr-19759.31` | ROM: SSP=`0x00002000`, PC=`0x00600120`. All vectors point into `0x6xxxxx`. |
| `0x800000–0xFFFFFF` | 8 MB | Sample ROM (`.32` then `.34`) | MAME splits this into `0x800000` (fixed first 2 MB), `0xA00000` (4 MB bank) and `0xE00000` (2 MB bank). With only 8 MB of samples the banks never move, so the map is linear: 68k address − `0x800000` = sample ROM offset. |

Byte order (MAME `ROM_LOAD16_WORD_SWAP`; H from ROM): `mpr-19759.31`,
`mpr-19745.32`, `mpr-19746.34` and the DSB2 program `mpr-19760.2s` are
stored **little-endian per 16-bit word**. Swap each byte pair to get the
68k's big-endian view. Check: the swapped `.31` starts `00 00 20 00 00 60
01 20` (SSP `0x2000`, PC `0x600120`). The MPEG ROMs are **not** swapped
(§3.5).

### Reset and vectors
- MAME's reset handler copies the first 16 bytes of the program ROM to RAM
  `0x000000` so that the 68k can fetch SSP/PC from RAM. That is an emulator
  device, **[MAME-only]**. Real hardware must provide the reset vector some
  other way, for example by mirroring ROM at 0 during reset. Unknown.
- ROM: the driver itself (`0x600354`) copies 256 bytes (64 vectors) from
  ROM `0x600000` to RAM `0x000000`. This happens *after* it clears RAM
  `0x00000–0x6FFFF` (`0x600190` loop), so from then on all exceptions vector
  through RAM. H.
- SCSP p01_20: the SCSP design expects the 68k reset vector in sound RAM at
  0 (Saturn convention). H for Saturn. It suggests that Model 2 may also
  need *something* (ROM overlay or boot copy) at reset.
- **Emulation choice:** at reset, load SSP/PC from the ROM image (as if
  ROM were at 0), then let the driver install its own table. This is
  equivalent to MAME's copy.
- ROM (H): between reset and the vector copy the driver reads RAM only at
  `0xC04–0xC0F` (`0x600174`): a warm-restart marker, the ASCII
  `"ErroRSet"` at `0xC08`, which its `0xFF` handler writes before jumping
  back to `0x600120` (§2.10). Nothing reads `0x0–0xF`, so how the real
  board supplies the reset vector does not matter once the CPU is running.
  `crates/sound` boots it this way (first unmasked interrupt at 0.40 s).

### Board control latch `0x400001` (write, byte)
- ROM: written at init with 0, then with `0x08, 0x0C, 0x0E, 0x0F` as the
  RAM-clear loop progresses (`0x6001AC`). Later writes are a rotating 4-bit
  pattern ORed with the ROM config byte at `0x608068` (`0x00` in
  topskatr), written from the MIDI ISR (`0x600586`) and elsewhere. This
  looks like **4 LEDs on bits 3-0** that blink on MIDI traffic (L).
- MAME: bit 5 (`0x20`) selects the sample bank on boards with more than 8 MB
  of samples. Supermodel (Model 3): bit 4. topskatr never sets bits 4-7
  (they come from the `0x00` config byte), so **banking can be ignored for
  topskatr** (H for topskatr's writes; the meaning of the bank bits is M/L).

### How the SCSP reaches samples in ROM
- The SCSP's sample address SA is **20 bits** (byte address within 1 MB of
  sound memory; SCSP p04_21, p04_29). It cannot address an 8 MB ROM. H.
- In MAME and Supermodel the SCSP's memory space is **sound RAM only** (MAME
  `scsp_map` = `0x00000–0x7FFFF` RAM; SM reads waveforms only from its RAM
  buffer). Both play Model 2/3 sample-ROM games with correct sound, so the
  **68k must copy sample data from ROM into sound RAM** (preload or
  streaming). M.
- ROM: the driver selects slots via MSLC (`move.b ...,$408(a5)` at
  `0x6019DC`, `0x601A7A`, ...) and tests CA bits (`tst.b $409(a5)` at
  `0x601A22`, `0x601A4A`). That is the slot-position monitor a streaming
  driver uses to decide when to refill a RAM buffer (L, an inference). The
  ROM also contains several 32-bit constants in `0x800000–0xCE0000`
  (sample ROM addresses).
- **Emulation choice:** map ROM only into the 68k. Give the SCSP the 512 KB
  RAM only, with SA masked to `0x7FFFF` (MEM4MB=1, 512 KB; see §2.2).
- **Confirmed by running the driver** (`crates/sound`, H for what the code
  does): every sound is **streamed**. A voice table at RAM `0x1000` (32
  entries × `0x20`) holds a ROM source pointer, a RAM buffer pointer and a
  remaining length. The main loop (`0x6019DC–0x601B5E`, about 93% of the
  68k's time) selects each streaming slot with MSLC, waits a few
  instructions, tests CA bit 0 (`tst.b $409(a5)`, bit 7 = sample offset bit
  12, i.e. which half of the buffer is playing) and copies the other half
  in 512-byte `movem.l` chunks from `0x800000+`. Buffers are 8 KB each at
  RAM `0x60000–0x6FFFF` (SA `0x6xxxx`), played as **8-bit PCM** (PCM8B=1),
  LPCTL=1, LSA=0, LEA=`0x1FFF`, OCT=−1 FNS=0 (**22.05 kHz**). Stereo
  sounds use two slots, one with DIPAN `0x1F` (left only) and one with
  `0x0F` (right only). Typical voice: AR=`0x1F`, D1R=D2R=0, DL=0, RR=`0x1F`
  (some `0x0C`), KRS=0, TL `0x0D–0x3F`, DISDL 6-7, and `+0x14` either
  `0x0004` (ISEL 0, IMXL 4: send to the DSP at −18 dB) or `0x0008` (ISEL 1,
  IMXL 0: dry). No LFO, no FM, no DMA seen.

### Interrupt wiring
- The SCSP drives the 68k IPL0-2 lines directly. Interrupts are
  **autovectored**, levels 1-7, and the level per source comes from
  SCILV0-2 (SCSP p04_2c, H). On Model 2, MAME wires the SCSP's IRQ output
  straight to the 68k input lines (M).
- ROM (H). The driver writes regs `0x418–0x42A` from a table at ROM
  `0x60025E`:

  | Reg | Value | Meaning |
  |---|---|---|
  | `0x418` TACTL/TIMA | `0x00CE` | timer A: ÷1, start `0xCE` |
  | `0x41A` TBCTL/TIMB | `0x03C6` | timer B: ÷8, start `0xC6` |
  | `0x41C` TCCTL/TIMC | `0x0100` | timer C: ÷2, start `0x00` (not enabled) |
  | `0x41E` SCIEB | `0x00C8` | enable bit 3 (MIDI in), 6 (timer A), 7 (timer B) |
  | `0x420` SCIPD | `0x0000` | (write ignored except bit 5) |
  | `0x422` SCIRE | `0x00C8` | clear pending 3/6/7 |
  | `0x424` SCILV0 | `0x0088` | bits 3, 7 |
  | `0x426` SCILV1 | `0x0048` | bits 3, 6 |
  | `0x428` SCILV2 | `0x0000` | |
  | `0x42A` MCIEB | `0x0020` | main-CPU IRQ on bit 5 only (no main CPU is wired on Model 2, as far as known) |

  So **MIDI-in → level 3** (`011`), **timer A → level 2** (`010`), **timer
  B → level 1** (`001`). The vector table agrees: autovector 1 (`0x64`) →
  `0x60064C` (it reloads TIMB = `0x03C6` and clears SCIRE bit 7); autovector
  2 (`0x68`) → `0x602766` (it reloads TIMA from ROM word `0x60806A` =
  `0x00CE` and clears SCIRE bit 6); autovector 3 (`0x6C`) → `0x6004EA` (it
  reads MIBUF). The other vectors go to an error stub.
- Tick rates (H from the values; the exact period depends on the off-by-one
  in §2.6): timer A every (`0x100−0xCE`) = 50 samples ≈ **882 Hz**. Timer B
  every 58 × 8 = 464 samples ≈ **95 Hz**.

### Clocks (MAME, M)
- The 45.1584 MHz crystal is on the 2C board (PCB note: "OSC ... 45.158MHz").
  SCSP = 22.5792 MHz. Output rate = SCSP clock / 512 = **44.1 kHz** (32
  slots × 16 clocks).
- 68k = 45.1584 / 4 = 11.2896 MHz (MAME). The same as the Saturn's
  68EC000, so plausible. The part is rated 12 MHz.
- MAME adds 1 wait state per 68k access to RAM and SCSP. Its comment calls
  this a guess borrowed from the Saturn ([MAME-only], L).
- 2C PCB parts (MAME, Behind Enemy Lines 837-12469-01): MC68EC000FN12,
  315-5687 (×1), TDA1386T DAC, D71051GU-10 UART.

### i960 side: who resets the sound CPU?
- ROM (i960 `sub_b1450`): the game writes 0 to `0x01CE0000`, programs the
  UART (`0x00, 0x00, 0x00, 0x40` reset sequence, then mode `0x4E`, then
  command `0x37`), and then writes 1 to `0x01CE0000`. `0x01CE0000` is
  unmapped in MAME. It may be a sound-board or UART reset line. **L,
  unknown.**

---

## 2. SCSP (YMF292-F) at emulation level

### 2.1 Slot registers (32 slots × 0x20 bytes, slot n at `0x100000 + n·0x20`; SCSP p04_11, H)

Each slot uses offsets `+0x00..+0x17`. `+0x18..+0x1F` are unused.

| Off | Fields, MSB to LSB (bit ranges) |
|---|---|
| +00 | 15-13 unused · 12 KYONEX · 11 KYONB · 10-9 SBCTL · 8-7 SSCTL · 6-5 LPCTL · 4 PCM8B · 3-0 SA[19:16] |
| +02 | 15-0 SA[15:0] |
| +04 | 15-0 LSA (loop start, in samples from SA) |
| +06 | 15-0 LEA (loop end, in samples from SA) |
| +08 | 15-11 D2R · 10-6 D1R · 5 EGHOLD · 4-0 AR |
| +0A | 15 (EG bypass\*) · 14 LPSLNK · 13-10 KRS · 9-5 DL · 4-0 RR |
| +0C | 15-10 unused · 9 STWINH · 8 SDIR · 7-0 TL |
| +0E | 15-12 MDL · 11-6 MDXSL · 5-0 MDYSL |
| +10 | 15 (short wave\*) · 14-11 OCT · 10 (FNS bit 10\*) · 9-0 FNS |
| +12 | 15 LFORE · 14-10 LFOF · 9-8 PLFOWS · 7-5 PLFOS · 4-3 ALFOWS · 2-0 ALFOS |
| +14 | 15-7 unused · 6-3 ISEL · 2-0 IMXL |
| +16 | 15-13 DISDL · 12-8 DIPAN · 7-5 EFSDL · 4-0 EFPAN |

\* Undocumented bits from MDF only (L): `+0A` bit 15 = EG bypass (the level
is forced to 0 dB); `+10` bit 15 = "short wave" (the address wraps on the
lowest set bit of LEA & `0x780`); `+10` bit 10 is treated as FNS bit 10
(the phase increment uses `0x400 XOR FNS[10:0]`). Ignore these unless a game
sets them.

Field semantics (SCSP p04_21..p04_27, H unless noted):
- **KYONEX** (write-only, per slot but global in effect): writing 1 to it in
  *any* slot executes key-on/off for *all* slots according to their KYONB.
  It doesn't need to be cleared. **KYONB**: 1 = key on, 0 = key off.
- **SBCTL**: bit 0 inverts all non-sign bits of the sample (XOR `0x7FFF`);
  bit 1 inverts the sign bit (XOR `0x8000`).
- **SSCTL**: 0 = sound RAM, 1 = noise (the LFSR shared with the LFO), 2 = all
  zeros, 3 = reserved.
- **SA**: byte address. It must be even for 16-bit PCM.
- **PCM8B**: 0 = 16-bit signed big-endian words, 1 = 8-bit signed bytes
  (used as the high byte, `<<8`; MDF, MAME).
- **LPCTL**: 0 = loop off (stop at LEA), 1 = normal (LSA→LEA, jump back to
  LSA), 2 = reverse (play SA→LSA forward, then loop LEA→LSA backwards), 3 =
  alternating (ping-pong between LSA and LEA). In loops the samples at SA+LSA
  and SA+LEA should be equal (p04_21).
- **EGHOLD**: during attack, the output is held at maximum (0 dB) while the
  EG counts. **LPSLNK**: attack → decay 1 only once the read address reaches
  the loop start.
- **SDIR**: bypass EG, TL and ALFO (raw sample). **STWINH**: don't write this
  slot's output to the sound stack.
- **OCT**: signed 4 bits (0..7 = +0..+7, 8..F = −8..−1). **FNS**: 10-bit
  fraction of an octave (§2.3).
- **ISEL/IMXL**: DSP input mixer bus (MIXS 0-15) and level. IMXL
  0 = off, 1..7 = −36, −30, −24, −18, −12, −6, 0 dB.
- **DISDL/EFSDL**: send level, same scale as IMXL (0 = off, 7 = 0 dB).
  **DIPAN/EFPAN**: §2.5.
- The EFSDL/EFPAN in slot n (0-15) apply to **DSP output EFREG[n]**. In slots
  16 and 17 they apply to **EXTS[0]/EXTS[1]** (external digital input). In
  slots 18-31 they do nothing (MDF, MAME; M). The slot's own sample isn't
  involved in that path.

### 2.2 Common control registers (`0x100400+`; SCSP p04_12 and others, H; read/write detail from MDF/MAME, M)

| Addr | Bits | Field | Notes |
|---|---|---|---|
| `400` | 9 | MEM4MB | 1 = 4 Mbit DRAMs (512 KB per bank; the manual says "always set 1"). |
| | 8 | DAC18B | 18-bit DAC output format. |
| | 7-4 | VER | read-only version. |
| | 3-0 | MVOL | master volume, 0 = mute, 15 = 0 dB (§2.5). |
| `402` | 8-7 | RBL | DSP ring buffer length: 0..3 = 8K/16K/32K/64K **words** (MAME, MDF; M). |
| | 6-0 | RBP | ring buffer base = RBP × `0x1000` words (= RBP × 8 KB). The manual shows `[7:0]` and MAME masks 6 bits, MDF 7 bits. |
| `404` | 12 | MOFULL | MIDI out FIFO full (R). |
| | 11 | MOEMP | MIDI out FIFO empty (R). |
| | 10 | MIOVF | MIDI in overflow (R). |
| | 9 | MIFULL | MIDI in FIFO full (R). |
| | 8 | MIEMP | MIDI in FIFO empty (R). This flag is missing from the manual's list, but MAME and MDF both have it (M). |
| | 7-0 | MIBUF | MIDI in data. **A read that includes the low byte (`0x405`) pops the FIFO.** Reading only `0x404` doesn't (MAME, MDF; M). |
| `406` | 7-0 | MOBUF | MIDI out data (W); queues a byte. |
| `408` | 15-11 | MSLC | monitor slot select (W). |
| | 10-7 | CA | (R) bits 15-12 of the monitored slot's current sample offset from SA (units of 4096 samples). |
| | 6-5 | SGC | (R) EG phase: 0 attack, 1 decay 1, 2 decay 2, 3 release (MDF; M with MAME). |
| | 4-0 | EG | (R) top 5 bits of the EG attenuation (MDF). |
| `412` | 15-1 | DMEA[15:1] | DMA memory address (word). |
| `414` | 15-12 | DMEA[19:16] | |
| | 11-1 | DRGA | DMA register address (word). |
| `416` | 14 | DGATE | DMA writes zeros. |
| | 13 | DDIR | 0 = memory→registers, 1 = registers→memory. |
| | 12 | DEXE | start; reads 1 until done. |
| | 11-1 | DTLG | length in words. |
| `418`/`41A`/`41C` | 10-8 | TxCTL | timer prescale 2^n samples. |
| | 7-0 | TIMx | start count (W) (§2.6). |
| `41E` | 10-0 | SCIEB | sound-CPU interrupt enable. |
| `420` | 10-0 | SCIPD | pending (R). Writing 1 to bit 5 raises a "CPU" interrupt. |
| `422` | 10-0 | SCIRE | write 1 = clear pending. |
| `424`/`426`/`428` | 7-0 | SCILV0/1/2 | bit b of SCILVk = bit k of the level code for source b. Bit 7 is shared by sources 7, 8, 9 and 10. |
| `42A`/`42C`/`42E` | 10-0 | MCIEB/MCIPD/MCIRE | the same set for the "main CPU" output. On Model 2 that output probably goes nowhere (L). |

Sound stack, DSP and other blocks (SCSP p04_13, p04_14, H):
- `0x600–0x67E`: SOUS, 64 words = 2 generations × 32 slot outputs (R/W).
- `0x700–0x77E`: COEF[0..63], 13 bits, left-aligned (bits 15-3).
- `0x780–0x7BE`: MADRS[0..31], 16-bit word addresses.
- `0x800–0xBFF`: MPRO, 128 steps × 64 bits (four words, MSW first).
- `0xC00–0xDFF`: TEMP[0..127], 24 bits (low byte at even address bits 7-0,
  high 16 bits at the next word).
- `0xE00–0xE7F`: MEMS[0..31], 24 bits, same layout.
- `0xE80–0xEBF`: MIXS[0..15], 20 bits.
- `0xEC0–0xEDF`: EFREG[0..15], 16 bits.
- `0xEE0–0xEE3`: EXTS[0..1], 16 bits (external input; on Model 2 probably
  unconnected, L).

Interrupt sources (bit numbers for SCIEB/SCIPD/SCIRE/MCI*; SCSP p04_2c, H):

| Bit | Source |
|---|---|
| 0, 1, 2 | external inputs INT0N, INT1N, INT2N |
| 3 | MIDI in: set while the input FIFO holds data, and cleared by hardware when it has been read empty |
| 4 | DMA end |
| 5 | CPU manual (write 1 to SCIPD bit 5) |
| 6 | timer A |
| 7 | timer B |
| 8 | timer C |
| 9 | MIDI out FIFO empty |
| 10 | once per sample (44.1 kHz) |

**Level computation** (MDF, consistent with the manual; M). Take
`pend = SCIPD & SCIEB` and fold bits 8-10 into bit 7. Each pending source b
has level code `L(b) = SCILV2[b]·4 + SCILV1[b]·2 + SCILV0[b]`. The IPL
presented to the 68k is the **maximum L(b)** over pending sources, and 0
means no interrupt. The 68k takes autovector `24 + IPL`.
Clearing: write SCIRE. MIDI-in clears itself once the FIFO is empty
(MAME, manual).

### 2.3 Pitch

Per output sample (44.1 kHz), the phase advances by (MDF; agrees with MAME
and the manual's note table; M/H):

```
step (in samples, fixed point) = (1024 + FNS + PLFO_term) × 2^OCT / 1024
```

with OCT signed (−8..+7). In other words, the playback rate is
**44100 × 2^OCT × (1 + FNS/1024) Hz** relative to the sample data. FNS=0
and OCT=0 plays at the source rate (p04_25). The manual's table checks this
(H): FNS=`0x2BA` (698) gives 1 + 698/1024 = 1.6816 ≈ 2^(900/1200) =
1.6818, i.e. A4 = 900 cents above C4.

MDF keeps a 14-bit fractional phase. It adds `((0x400 ^ FNS) + PLFO) << (OCT
XOR 8)` and then shifts right 4 bits. The integer part (`phase >> 14`)
advances a 16-bit sample counter relative to SA, and the top 6 fractional
bits drive **linear interpolation** between sample n and n+1 (MDF). MAME
uses a 12-bit fraction (an implementation detail).

### 2.4 Envelope generator

State: a 10-bit attenuation **level**, 0 = loudest and `0x3FF` = silent,
3/32 dB (0.09375 dB) per step, about 96 dB full scale (MAME table, MDF; M),
and four phases: attack, decay 1, decay 2, release (SCSP p04_22, H).

Effective rate (MDF, L/M; see the MAME difference below):
- key scale `ks = 0` if KRS = 0xF, else `clamp(KRS + OCT_signed, 0, 15)`.
- `ER = min(0x1F, R + ks)`, where R is AR, D1R, D2R or RR for the current
  phase. **R = 0 never clocks.**

Clocking, per sample, using the global sample counter s (MDF):
- `WBT = (0x18 − min(ER, 0x18)) >> 1`.
- For **even ER < 0x18**: clock once every 2^(WBT+1) samples (when
  s mod 2^(WBT+1) = 2^WBT).
- For **odd ER < 0x18**: clock 1.5 times as often as ER−1. The EG fires when
  s has a 1 in bit WBT+1 or WBT+2 and zeros below.
- For **ER ≥ 0x18**: clock every other sample (s even), with a bigger step
  (below).
- Step shift `k`: `e = clamp(ER, 0x18, 0x1E)`, `k = (0x20 − e)/2`
  (integer), and for odd e `k` is one larger on alternate clocks. So k = 4
  for ER ≤ 0x18, then 3, 2, 1 at 0x1A, 0x1C, 0x1E/0x1F.
- **Decay 1, decay 2, release:** `level += 16 >> k` (+1 per clock for
  ER ≤ 0x18, up to +8 at the top rates). Clamp at `0x3FF`. Linear in dB.
- **Attack:** `level −= ceil((level+1) / 2^k)`, an exponential approach to
  0. This gives the convex attack curve in the manual's Fig 4.8. If
  `AR + ks ≥ 0x20` the attack is instant (level = 0 at key-on).
- At key-on the phase is set to attack, level = `0x280` (about −60 dB; MDF;
  MAME's start value is the same in its inverted scale), and the sample
  position is reset to 0.
- Transitions:
  - attack → decay 1 when level reaches 0 (or, with LPSLNK, when the loop
    start is reached);
  - decay 1 → decay 2 when `level >> 5 == DL`;
  - any phase → release on key-off;
  - D2R = 0 holds the level (sustain).
- The slot stops fetching memory (and goes silent) once level ≥ `0x3C0`
  (MDF), or at LEA with loop off.

Cross-check, MAME (M for shape, differs in detail). MAME uses the
manual-derived time tables `ARTimes[64]` / `DRTimes[64]` (ms for full
scale; e.g. decay at index 63 = 3.1 ms, attack at index 2 = 8100 ms) with a
**6-bit** rate index `2·R + OCT + 2·KRS + FNS bit 9`, linear increments,
and an exponential lookup at output. The MDF and MAME rate indices weight
the octave differently (MDF: +1 R unit per octave; MAME: +½ unit) and
treat FNS bit 9 differently. **Verify with real attack/decay timings if a
sound obviously differs.** The manual's own rate table is an image on
p04_22 and is not in the text.

### 2.5 Level, pan and output mixing

Per-slot amplitude (MDF, M; TL table from SCSP p04_24, H):
- `v = EG_level + TL·4 + ALFO` (each TL step = 4 EG steps = 0.375 dB; the
  manual quotes nominal bit weights −0.4/−0.8/−1.5/−3/−6/−12/−24/−48 dB).
  Clamp `v` to `0x3FF`.
- `out = sample × (0x7F − (v & 0x3F)) >> ((v >> 6) + 7)`: a 6-bit mantissa
  per 6 dB octave. SDIR skips this stage entirely.
- Send levels: `gain = 0x80 << SDL` (SDL = DISDL or EFSDL; 0 = off, 7 =
  `0x4000` = unity, each step −6 dB).
- Pan (DIPAN/EFPAN, 5 bits): bit 4 picks the side that is **attenuated**
  (0 → left attenuated, so the sound moves right; 1 → right attenuated).
  The attenuated side's gain is `gain >> ((pan & 0xF) >> 1)`. Odd values
  take another ×0.75 (about −2.5 dB, nominally −3 dB per step). `pan & 0xF
  = 0xF` → that side is silent. `0x00` and `0x10` are centre (p04_27, H;
  MDF arithmetic, M).
- Output: L/R = Σ over slots of `(slot_out × direct_gain) >> 14` + Σ
  `(EFREG[n] or EXTS × effect_gain) >> 14`. Then the master volume: `mv = 2 <<
  (MVOL >> 1)`, ×0.75 if MVOL is even, 0 if MVOL = 0, and `out = sum × mv >>
  8` (MVOL 15 = unity). Then clamp to 16 bits (MDF). MAME uses MVOL/15 as a
  linear gain (simpler, slightly different curve).
- DSP input: if IMXL ≠ 0, `MIXS[ISEL] += (slot_out << 4) >> (7 − IMXL)`
  (20-bit) (MDF).
- topskatr's init sets MVOL = 0, then `0x400` = `0x0300` (MEM4MB=1,
  DAC18B=1), RBL/RBP = `0x0138` (RBL=2: 32K words; RBP=`0x38`: base
  `0x70000`, so the ring buffer occupies sound RAM `0x70000–0x7FFFF`), and
  finally MVOL = 15. That fits the RAM clear stopping at `0x6FFFF` (ROM,
  H).

### 2.6 Timers A, B, C (SCSP p04_2b, H; MDF detail, M)
- 8-bit up counters. They count once every 2^TxCTL samples. Writing the low
  byte (TIMx) loads the count, and in MDF the load takes effect on the
  timer's next tick.
- The interrupt (SCIPD bit 6, 7 or 8) fires when the counter **reaches
  `0xFF`**. The manual gives the period as `(0xFF − TIM) × 2^TxCTL`
  samples. The counter keeps counting (it wraps through 0) unless it is
  rewritten. topskatr rewrites it in each ISR.

### 2.7 FM modulation and the sound stack (SCSP p04_23, H; MDF detail, M)
- Every slot's final output (after EG/TL, before pan) is written to a
  64-entry ring (SOUS) holding two sample generations of 32 slots. MDF
  writes it with a 4-slot delay. STWINH suppresses the write.
- MDXSL/MDYSL are 6-bit offsets into that ring relative to the current slot
  position. Per the manual, values `0x1C–0x3B` select the newest generation
  and others the previous one. Treat the ring as indexed by `(global_slot_
  counter + MDxSL) & 0x3F`, where the counter advances by 1 per slot (32
  per sample).
- Modulation: `m = ((X + Y) << 6) >> (16 − MDL)` in 1/64-sample units, 0 if
  MDL ≤ 4. Add it to the read position (its integer part offsets the
  address, its fraction combines with the interpolation fraction). Manual
  Table 4.8: MDL 5..F = ±π/16, π/8, π/4, π/2, π, 2π, 4π, 8π, 16π, 32π, 64π.

### 2.8 LFO (SCSP p04_26, H; MDF detail, M)
- Each slot has an 8-bit LFO counter. It advances once every N samples:
  `N = (((8 − (LFOF & 3)) << 7) >> (LFOF >> 2)) − 4`, so the waveform
  frequency is `44100 / (256·N)`. This reproduces the manual's table, e.g.
  LFOF 0x00 → 0.17 Hz, 0x10 → 2.87 Hz, 0x1C → 43.1 Hz, 0x1F → 172.3 Hz
  (checked, H). LFORE=1 holds the counter at 0.
- Waveforms (ALFOWS/PLFOWS): 0 = sawtooth, 1 = square, 2 = triangle, 3 =
  noise (a shared 17-bit LFSR, which is also the SSCTL=1 noise source).
- ALFO (unsigned 0..`0xFE`) `>> (7 − ALFOS)` is added to the attenuation
  (ALFOS 0 = off). That gives the manual's depths 0.4/0.8/1.5/3/6/12/24 dB.
  Check: `0xFE` × 0.09375 = 23.8 dB.
- PLFO (signed −128..127) `>> (7 − PLFOS)` is scaled by `(0x40 XOR
  (FNS>>4))/64` and added to `1024 + FNS` before the octave shift (MDF). The
  manual quotes ±7/13.5/27/55/112/230/494 cents for PLFOS 1-7, but MDF's
  arithmetic gives about ±190 cents at 7. **Discrepancy, unresolved (L).**
  MAME follows the manual's cent table.

### 2.9 DSP (SCSP p04_14, p05_10, H for the RAM layout; instruction fields M)
- It runs 128 steps once per sample. Memory accesses go to the ring buffer
  in sound RAM: `addr = (MADRS[MASA] (+1 if NXADR) (+ADRS_REG if ADREB)
  (+DEC unless TABLE), masked to the ring length unless TABLE) + RBP·0x1000`
  (in words). DEC is a counter that decrements once per sample.
- Instruction word (64 bits, step i at `0x800 + 8i`, MSW first). Bit
  numbers within the 64-bit word, from MDF's notes, which agree with MAME's
  field extraction except NOFL:

  | Bits | Field | Meaning |
  |---|---|---|
  | 62-56 | TRA | TEMP read address (+DEC) |
  | 55 | TWT | TEMP write |
  | 54-48 | TWA | TEMP write address (+DEC) |
  | 47 | XSEL | X = INPUTS (1) or TEMP (0) |
  | 46-45 | YSEL | Y = FRC_REG / COEF[CRA] / Y_REG[23:11] / Y_REG[15:4] |
  | 43-38 | IRA | INPUTS: 0x00-0x1F MEMS, 0x20-0x2F MIXS, 0x30-0x31 EXTS |
  | 37 | IWT | MEMS write (data from the read two steps earlier) |
  | 36-32 | IWA | MEMS write address |
  | 31 | TABLE | don't add DEC, don't wrap to the ring |
  | 30 | MWT | memory write |
  | 29 | MRD | memory read |
  | 28 | EWT | EFREG write |
  | 27-24 | EWA | EFREG address |
  | 23 | ADRL | latch ADRS_REG |
  | 22 | FRCL | latch FRC_REG |
  | 21-20 | SHIFT | shifter mode (×1 saturate, ×2 saturate, ×2 wrap, ×1 wrap) |
  | 19 | YRL | latch Y_REG = INPUTS |
  | 18 | NEGB | B = −B |
  | 17 | ZERO | B = 0 |
  | 16 | BSEL | B = ACC (1) or TEMP (0) |
  | 14-9 | CRA | COEF index |
  | 8 (MDF) / 15 (MAME) | NOFL | memory data is plain (`<<8`) instead of the 16-bit float format |
  | 6-2 | MASA | MADRS index |
  | 1 | ADREB | add ADRS_REG |
  | 0 | NXADR | +1 |

- Datapath: `ACC = (X × Y) >> 12 + B` (26-bit). The shifter output (24-bit)
  feeds TEMP, EFREG (`>>8`), memory writes and FRC/ADRS latches. Memory
  data uses a 16-bit float (sign, 4-bit exponent, 11-bit mantissa) unless
  NOFL. MAME only performs memory accesses on odd steps.
- ROM (H, by running it): topskatr loads a **108-step** program (MPRO,
  COEF, MADRS) after init; the ring is `0x70000–0x7FFFF` (RBL=2, RBP=`0x38`).
  Voices send to MIXS 0. The program writes EFREG 0-4, 14 and 15, has 10
  memory steps, and reads MIXS 0-2. Slots 0-4 keep EFSDL 7 in the low byte
  of `+0x16` (the driver writes only the high byte, DISDL/DIPAN, per voice):
  EFPAN `0x1F` on slots 0 and 2 (left), `0x0F` on 1 and 3 (right), `0x00`
  on 4 (centre); EFREG 14/15 have no return. A stereo reverb.
  `crates/sound` runs it, and tails follow each voice.
- Read the full semantics in MDF `ss/scsp.inc` (`RunDSPStep`, the
  `dspfloat_to_int`/`int_to_dspfloat` pair) and MAME `scspdsp.cpp`. The
  Neill Corlett "SCSP DSP" notes (Highly Theoretical / SSF) are the
  original reverse engineering. **Does topskatr use the DSP?** The ring
  buffer is configured, so probably (reverb). Check whether it writes MPRO
  (`0x100800+`).

### 2.10 MIDI port as the 68k sees it (SCSP p04_2a, H; MAME/MDF, M)
- 31.25 kbit/s, 8N1. The SCSP's MIDI-in is wired to the i960 UART TxD, and
  its MIDI-out to the UART RxD (MAME; see peripherals.md §3).
- The input FIFO is **4 bytes** (manual). MAME uses 32 and Supermodel
  enlarges it too (an emulation convenience; see `docs/mame-defects.md`
  MD-13). A byte arriving while the FIFO is full is lost and sets MIOVF.
- SCIPD bit 3 stays pending while the FIFO is non-empty. topskatr routes it
  to **IPL 3**. Its ISR (`0x6004EA`, ROM, H):
  - reads **one** byte from `0x100405` (MIBUF) per entry;
  - if the RAM flag byte `0x0C3F` is non-zero, echoes the byte to MOBUF
    (`0x100407`);
  - parses it as **MIDI with running status**. Status bytes (bit 7 set) set
    the expected data count: 2 for `0x80–0xBF` and `0xE0–0xEF`, 1 for
    `0xC0–0xDF`. `0xF0+` and `0xFF` are handled separately;
  - stores data bytes into a 1 KB word ring at RAM `0x0800` (index at
    `0x0EC4`, masked `0x3FF`);
  - blinks the LEDs.
- The interrupt stays asserted while bytes remain, so the 68k re-enters
  until the FIFO is empty.
- ROM (H): a **`0xFF`** byte (MIDI System Reset) makes the ISR reset SP,
  write the `"ErroRSet"` marker to RAM `0xC08` and jump to the reset entry
  `0x600120`: the driver restarts (re-initialising the SCSP and clearing
  RAM). The i960 sends one at init (§4), so the driver always restarts
  then.
- Status dispatch (`0x6006D4`): `(status >> 4) & 7` indexes a jump table at
  `0x600710`. `An` (`0x601E60`): first data byte `0x10–0x5F` = sound bank
  (`bank − 0x10` indexes the table at ROM `[0x60801C]`), second = entry;
  `< 0x10` and `≥ 0x60` go to other handlers (`0x70|a` from `sub_b1a30` lands
  in the latter). **The channel nibble is not filtered**: replaying
  `AE 1A nn` keys on slots exactly like `A0 1A nn` (M: by emulation). The
  game's own `AE 10 nn` / `AE 02 vv` / `AE 03 vv` select entries that start
  no voice, so they are silent on the SCSP. A sound already playing is not
  restarted by a repeat command (`A0 1D 05` arrives every ~0.3 s in a race,
  and its looping slot is keyed on once).
- With a 4-byte FIFO and bytes spaced at the link rate, the ISR keeps up:
  0 of 6,253 bytes lost in a boot-to-race capture (see MD-13).
- MIDI-out: the driver waits for MOFULL (bit 12 of `0x100404`, tested as
  bit 4 of byte `0x100404`) to clear, then writes MOBUF (`0x6029E2`, ROM,
  H). So the sound 68k **does transmit** in some state. See §4 for what the
  i960 does with it.

---

## 3. DSB2 (Digital Sound Board 2) on topskatr

### 3.1 Board
- MAME's Model 3 PCB note for the DSB2 (837-12273 "DIGITAL SOUND BD 2 SEGA
  1995", 171-7165D) lists:
  - MC68EC000FN12 at **11.000 MHz** (33 MHz / 3), 2× KM62256 and 2×
    MCM6206 (4 × 32 KB = 128 KB SRAM), D71051 UART, a 6-pin MIDI TX/RX
    connector (CN1), two GALs;
  - **Sega 315-6028A** (QFP100) at **12.288 MHz** (= 384 × 32 kHz),
    "probably a NEC DSP", with an unpopulated footprint for a **NEC
    uPD77016** DSP on the underside. The 315-6028A is therefore almost
    certainly a Sega-marked NEC DSP running an MPEG Layer II decoder;
  - 2× NEC uPD63210 (16-bit DAC with digital filter), an LMC6484 op-amp;
    stereo out on CN6;
  - JP8 tied to the 68k's IPL1.

  This is M as a board fact and L as "this is topskatr's board". BLOG lists
  Top Skater as the one Model 2 game with a DSB2, but nobody has documented
  its board revision.
- The first DSB (Z80, 837-10084) used a different decoder (NEC uPD65654
  gate array plus Sega 315-5762). `stcc` has one; it is modelled from its
  Z80 program in `docs/hw/dsb1.md`. Its MPEG data is the same format as
  the DSB2's and its decoder is driven the same way (start, end, a
  continue mode, read-back position), so the model reuses this decoder's
  behaviour behind the DSB1's ports.
- ROM string at DSB2 `0x7F00` (H): "DS2-BOARD SOUND PROGRAM FOR OLLIE OVER
  VERSION 0.30 PROGRAMMED BY YOHSUKE TAKEDA COPYRIGHT SEGA ENTERPRISES,LTD."
  ("Ollie Over" was evidently the working title).

### 3.2 68000 memory map (MAME, SM; M. ROM usage confirms each entry, H)

| Address | What | Evidence |
|---|---|---|
| `0x000000–0x01FFFF` | Program ROM `mpr-19760.2s` (128 KB, word-swapped) | ROM vectors: SSP=`0xF0FFFE`, PC=`0x000500` |
| `0xC00001` | UART data (8251-type, uPD71051) | ROM: L1 ISR reads it at `0x116`, writes it at `0x186` |
| `0xC00003` | UART control (W) / status (R) | ROM: init writes `00 00 00 40 4E 37` (reset, mode 8N1 ×16, command TxEN/RxE/DTR/RTS/ER), the same as the i960 side |
| `0xD00001` | System control latch (W) | ROM writes `0x3F`, pulses to `0x1F` and back (bit 5 low = probably decoder reset, L), later `0x38`. The L2 ISR pulses bit 4 (`&0xEF`, then `|0x10`), probably a timer-IRQ acknowledge or watchdog (L). MAME: bit 3 = MPEG ROM A24 bank, inverted; irrelevant with 16 MB. |
| `0xE00001` | MPEG decoder port, high/command byte (W) | ROM: used together with `0xE00003` at init. **MAME leaves it unmapped.** |
| `0xE00003` | MPEG decoder port, data/FIFO byte (W) | MAME and SM parse the commands here (§3.4) |
| `0xE80001` | MPEG decoder status (R) | ROM polls **bit 1** until it is 0 before each write (SM comment: "bit 1 = busy"). MAME returns `0x01` (never busy). |
| `0xF00000–0xF1FFFF` | 128 KB work RAM | SSP `0xF0FFFE`; host-command ring at `0xF00000–0xF00FFF` (§3.3) |

Interrupts (ROM vectors, H; meanings M):
- autovector 1 (`0x64`) → `0x110`: **UART receive** (RxRDY → IPL1;
  MAME, SM).
- autovector 2 (`0x68`) → `0x1BC`: **periodic tick** (SM and MAME use
  1 kHz; Supermodel notes that 1 kHz makes fades time correctly in other
  games, L for the exact rate). It increments a counter at `0xF0100E` and
  pulses latch bit 4 low and back high. Because the 68000's interrupt inputs
  are level-sensitive, that pulse must be what clears the request; reModel
  models it that way and the firmware runs normally (M). The firmware only
  uses the tick in units of 80 (§3.7), so its rate is not pinned down by
  the code; reModel uses 1 kHz (L).
- autovector 3 (`0x6C`) → `0x100`: a stub that resets SP and jumps into the
  L1 handler. Its purpose is unknown (L).

### 3.3 Host byte handling on the DSB2 (ROM `0x110–0x1BA`, H; interpretation M)
- Every byte from the host UART raises IRQ1. The ISR:
  - counts consecutive `0xFF` bytes in `0xF01011`, but then compares
    **`d0`**, not the count, with 4 before jumping to its reset entry
    (`0x1F0`: reload SP, jump to `0x500`). `d0` there is whatever the
    interrupted code left in it, so the reset-on-`0xFF`-run feature is
    effectively broken: a firmware bug (H for the code);
  - keeps a two-state parser:
    - **a status byte `0xAE`** (MIDI "channel 14") switches to *capture*.
      The `0xAE` and every following data byte are stored in a 4 KB ring at
      `0xF00000` (write pointer at `0xF01004`) for the main loop;
    - **any other status byte** switches to *pass-through*. That byte and
      its data bytes are **retransmitted unchanged on the DSB2's own UART
      TxD** (after waiting for TxRDY).
- So the DSB2 behaves like a MIDI-thru that keeps channel 14 for itself. This
  raises a topology question (**L**): on real hardware the link might be
  daisy-chained, i960 → DSB2 → SCSP MIDI-in, and not a parallel feed as in
  MAME and Supermodel. With a daisy chain the SCSP would never see `0xAE`
  messages. Either way, a byte stream that reaches both boards gives the
  same result if the SCSP driver ignores channel 14. Check the sound 68k
  driver's handling of `0xAn` with n = 14 before choosing.

### 3.4 MPEG decoder command protocol (68k → `0xE00003`; MAME, SM; M)
Byte-wide commands. The low bit of each opcode selects MPEG player 0 or 1
(MAME: the firmware ORs a player id of 4/5 into `0x10/0x20/0x70/0x80`).

| Byte(s) | Meaning |
|---|---|
| `0x14`/`0x15`, then 3 bytes | start address, MSB first (24-bit ROM byte address) |
| `0x24`/`0x25`, then 3 bytes | end address, MSB first |
| `0x74`/`0x75` | play from start to end |
| `0x84`/`0x85` | stop |
| `0xA0`/`0xB1` + 1 byte | SM: non-zero = mono-left / mono-right, zero = stereo |
| `0xB0`/`0xA1` + 1 byte | SM: left / right volume (`0x00–0xFF`). Other games also use `0xB2/0xA3` (front) and `0xB6/0xA7` (rear) |
| `0xA4`/`0xA5`/`0xB4`/`0xB5` + 1 byte | SM: other games use these; `0xB4 0x96` stops in one of them |

- SM: writing a new start address *while playing* sets the loop point. When
  the current end is reached, playback continues from the new start ("set
  loop"). MAME doesn't implement looping and stops at the end.
- **Boot upload, not modelled by either emulator** (ROM `0x5E8–0x672`, H):
  1. the firmware pulses the latch;
  2. it writes three byte pairs to `0xE00001`/`0xE00003` (`00/00`, `40/00`,
     `00/00`);
  3. it streams a **256-byte block** from its ROM (`0xF28`) to the decoder
     through the same two ports, polling `0xE80001` bit 1 between writes;
  4. it writes `0x01` to `0xE00003`.
  This looks like loading microcode or configuration into the 315-6028A (L).
  None of the uploaded bytes match the FIFO opcodes, so MAME's
  byte-for-byte parser doesn't misfire on them.
- **What topskatr's firmware actually sends** (ROM, H; meanings from the
  firmware's use and reModel's render, M unless marked):
  - `80`-`87`, then `A0`-`A7` each with `00`, at boot (`0xD2E`);
  - `14 aa aa aa`, `24 aa aa aa`: start and end, **byte addresses into the
    16 MB MPEG space**. Every track range lands on frame boundaries (§3.7);
  - `64` then `74` for a looping track, `74` alone for a one-shot. A start
    address written while playing is the loop point. So `64` arms looping
    (M), and Supermodel's "set loop" reading of a mid-play start write holds
    for this firmware;
  - `84` stops (before every new track, and when a fade reaches zero);
  - gain bytes `A0`, `A1`, `A4`, `A5`. A 2-bit mode per track picks which
    pair is zeroed and which carries the volume: mode 0 sets `A0`/`A4`,
    mode 1 `A1`/`A5`, mode 2 `A0`/`A5` (table at `0xE1C`). Every topskatr
    track uses mode 2, and the music is stereo, so the best fit is a 2×2
    matrix: `A0` L←L, `A1` L←R, `A4` R←L, `A5` R←R (modes: mono-left,
    mono-right, stereo). reModel models it that way with gain = value/255
    (L for both the matrix and the linear scale). This is not the
    `A0`/`B1`, `B0`/`A1` layout Supermodel uses for other games;
  - no `B0`-`B7`, no odd (player 1) opcodes.
- Data path (M): the decoder **reads MPEG ROM directly** between the
  start/end addresses. The 68k never maps the MPEG ROM (no 68k address
  reaches it in MAME or SM, and the ROM has no references to one). Output
  is 32 kHz stereo.

### 3.5 MPEG data format (ROM, H)
- The four 4 MB ROMs form one 16 MB space (`.18s` at 0, `.20s` at
  `0x400000`, `.22s` at `0x800000`, `.24s` at `0xC00000`). They are stored
  **as plain bytes** (MAME `ROM_LOAD`, no swap). Frame sync `0xFFF` is found
  only in this order.
- Scanning with frame-length chaining finds **MPEG-1 Layer II, 128 kbit/s,
  32 kHz, stereo** (576-byte frames) in all four ROMs: 7,160 + 7,137 +
  7,278 + 5,075 = 26,650 frames of 1,152 samples, about 16 minutes of
  audio. No other
  rate/bitrate/layer occurs.
- `mpr-19747.18s` `0x0000–0x10FFF` holds something other than audio:
  little-endian 16-bit values that start like a gain table (`0x0CCD, 0x199A,
  0x2666, ...`). Its purpose is unknown, maybe a decoder table that the
  315-6028A reads itself (L).

### 3.6 Mixing with the SCSP
- The DSB2 has its own DACs and analog outputs (PCB note; BLOG: music goes
  out on separate connectors to the amp, and without a DSB2 the game plays
  effects only). So the real mix happens **in the analog domain**,
  outside the SCSP (M).
- MAME's TODO "should be chained with SCSP EXTS" is therefore doubtful (L).
  It holds only if a Model 2C board feeds digital audio into the SCSP's EXTS
  input, and nothing suggests that.
- **Emulation choice:** decode at 32 kHz, resample to 44.1 kHz, and add to
  the SCSP output after the gain matrix of §3.4.
- **Relative level: unknown (L).** Summed at unity, the DSB2 averaged about
  18 dB above the SCSP in a race (RMS −17 vs −35 dBFS; peaks −0.4 vs −10)
  and buried the announcer. The analog gains aren't in any ROM. The SCSP is
  set to 18-bit DAC mode (DAC18B), and how its 16-bit mix maps onto that
  DAC's full scale is also open. `model2::audio::DEFAULT_MIX` (SCSP +8 dB,
  DSB2 −8 dB, chosen by ear in play) is a listening choice, adjustable per
  player. `crates/dsb` does the
  first two (Catmull-Rom resampling).
- **Replacement soundtracks** (`docs/mods.md`) hook in at the decoder: a
  play (`74`) whose start/end matches a replaced track's range streams a
  file instead, resampled straight to 44.1 kHz. It goes through the same
  gain matrix and obeys `64` (loop armed) and `84` (stop).

### 3.7 DSB2 firmware: commands and track table (ROM, H unless marked)
The main loop (`0x6D2`) takes bytes from the 4 KB ring and parses MIDI with
running status; only `AE` messages are acted on:

| Message | Firmware action |
|---|---|
| `AE 10 nn` | `84` (stop). nn = 0: nothing else. 1 ≤ nn < 13 (count at `0x800A`): play track nn from the table; nn ≥ 13 is ignored. |
| `AE 02 vv` | master volume = 2·vv (default `0x00FF` from `0x8008`); if playing, rewrite both gains at once and cancel fades |
| `AE 03 vv` | if playing, fade out at a rate of `−496·vv − 256` (16.16 per step); `84` when both sides reach 0 |
| `AE 60 01` + 12 bytes, mode, volume, loop flag | direct play: start, end and loop addresses as three 4×7-bit groups. Not sent by the i960 (a test hook, L) |
| `AE xx yy` (other) | ignored |

- Track table: a pointer at `0x8000` to 12 entry pointers. Each entry: a
  flags word (bit 15 loop, bit 14 a second loop segment, low bits the gain
  mode), start and end (32-bit, low 24 bits used), a fade-in rate word, a
  left and right volume byte, then the loop start (and with bit 14 two more
  addresses) and a fade-out rate word plus delay. topskatr uses no fade-in,
  no auto fade-out, no bit 14, volume `0x80/0x80` and mode 2 throughout.
- Volume → gain register goes through a 256-byte curve at `0xE28`, close to
  linear (entry `0x80` is `0x60`, entry `0xFF` is `0xFE`). The fade-out
  step writes the raw volume, skipping the curve: a firmware slip (H).
- Timing runs in steps of 80 ticks. The main loop does one action per step
  when idle: 8 steps after a looping play it writes the loop start (`14`),
  and each fade step waits a further 80 ticks per side. With a 1 kHz tick,
  `AE 03 40` from gain `0xE8` fades out in about 1.8 s (L: rate).

| Track | Start | End | Loop to | Length |
|---|---|---|---|---|
| 1 | `0x011000` | `0x413FC0` | `0x011000` | 262.9 s |
| 2 | `0x193700` | `0x413FC0` | `0x011000` | 164.0 s |
| 3 | `0x2EAFC0` | `0x413FC0` | `0x011000` | 76.0 s |
| 4 | `0x96FBC2` | `0xAB68C2` | once | 83.7 s |
| 5 | `0x7AC3C4` | `0x7E9B04` | `0x7AD144` | 15.7 s |
| 6 | `0x800000` | `0x96FBC0` | once | 94.1 s |
| 7 | `0x413FC2` | `0x7A9482` | `0x413FC2` | 234.8 s |
| 8 | `0x544042` | `0x7A9482` | `0x413FC2` | 157.0 s |
| 9 | `0x67AF02` | `0x7A9482` | `0x413FC2` | 77.4 s |
| 10 | `0xAB68C4` | `0xBBE804` | once | 67.6 s |
| 11 | `0xBBE806` | `0xCC7706` | once | 67.8 s |
| 12 | `0xCC7708` | `0xEC8948` | `0xCCFBC8` | 131.4 s |

Every start, end and loop address is a frame boundary and every range is a
whole number of 576-byte frames (H). Tracks 2-3 and 8-9 are later entry
points into tracks 1 and 7. In the captured attract-into-race run the i960
plays 4 (attract), stops it, plays 5 (whose 15.5 s loop repeats), fades it
out with `AE 03 40`, then plays 7 (`make music`).

---

## 4. What the i960 sends (ROM, H unless marked)

From `out/topskatr/program.lst` (i960):
- **Transport:** a 128-byte transmit ring at RAM `0x5DCED0`, with a count
  at `0x5DCF50` and a write index at `0x5DCF54`.
  - Init fills the ring with `0x99` and sends a single **`0xFF`**, the MIDI
    System Reset byte (`sub_b1368`).
  - A periodic routine (`sub_b13c8`) sends **one byte per call**, only when
    status bit 0 (TxRDY) is set.
  - The i960 code found so far **never reads the UART data register**: no
    load from `0x01C80000`, only status reads from `0x01C80002`. So
    replies from the sound board (MIDI-out) seem to be ignored by the game
    (M; it depends on the analysis having found all the code).
- **Message format:** always **3 bytes**, enqueued by `sub_b17f0` (skipped
  under some game-state condition) or `sub_b18a0` (always). They look like
  MIDI channel messages:
  - **`0xA0 | ch`, d1, d2** ("polyphonic pressure" status, repurposed). The
    main sound-effect call (`sub_b1930(id)`) takes `id ≤ 0x115` and sends
    the 3 bytes at i960 ROM `0xAFA10 + 3·id`. 276 of those 278 entries are
    `A0 xx yy`, with xx in `0x10–0x1D` (a bank or group) and yy an index.
  - `sub_b1a30(ch, a, b)` sends `A0|ch, 0x70|a, b&0x7F`. This is probably a
    per-channel parameter such as volume or pan (L).
  - `sub_b19f0(ch, a, b)` sends `B0|ch, a, b&0x7F` (control change).
  - **Music:** `sub_b21b0(n)` first sends `AE 02 vv` (vv from a 16-entry
    table at `0xB21A0`) and then **`AE 10 nn`** (nn = n & 0x7F). Table entry
    `0x114` is `AE 10 01` and `0x115` is `B9 50 00`. Channel 14 (`0xAE`) is
    exactly what the DSB2 firmware captures (§3.3), so **`AE 10 nn` = play
    music track nn** and `AE 02 vv` is a DSB2 parameter, probably volume (M:
    the two firmwares agree; the meaning of `02` is L).
- **Reply path:** the sound 68k can echo every received byte to MIDI-out
  (when its flag `0x0C3F` is set) and has a general MOBUF transmit routine.
  The DSB2 forwards non-channel-14 traffic on its TxD. MAME's old HLE
  (peripherals.md §3) made the UART always report RxRDY with data `0xFF`.
  Given that topskatr never reads the data register, a model that sends no
  replies should be safe for this game (M).
- General Sega convention (L, speculative): the Model 2/3 SCSP drivers
  ("CRX_drv" here) implement a MIDI-like protocol. Channel numbers select
  logical voices or sound groups, and `An`/`Bn` statuses carry
  driver-specific commands, not real aftertouch or CC semantics. This
  matches what topskatr's tables show. No public specification is known.

### 4.1 Emulation choices in `crates/sound` (where the sources disagree)

| Topic | Choice | Other source |
|---|---|---|
| EG rates and clocking | MDF's scheme (§2.4) | MAME: manual time tables, different octave weight |
| Instant attack | AR + KS ≥ `0x1F` | MDF: ≥ `0x20` |
| PLFO depth | manual cents table | MDF: about ±190 cents at 7 |
| Master volume | MDF's 3 dB steps | MAME: MVOL/15 linear |
| MIDI-in FIFO | 4 bytes (manual) | MAME 32 (MD-13) |
| DSP NOFL bit | bit 8 (MDF) | MAME: bit 15 |
| DSP memory access | every step, one-step read latch | MAME: odd steps only; MDF: two-step delay |
| FM sound stack | written without MDF's 4-slot delay | MDF delays by 4 slots |
| 68k bus | no wait states | MAME: 1 wait per RAM/SCSP access |

None of these is known to matter for topskatr: its voices use instant
attack, sustain, fast release, no LFO and no FM.

---

## 5. Open questions / what to verify against the ROM

1. **Reset vector source.** Does real hardware mirror the program ROM at 0
   during reset? Emulate by loading SSP/PC from ROM. *Answered for the
   emulation:* nothing reads RAM `0x0–0xF` before `0x600354` (§1). The
   hardware mechanism is still unknown.
2. **Sample streaming.** *Answered* (§1, "Confirmed by running the
   driver"): double-buffered 8 KB streams refilled on CA.
3. **`0x400001` bits.** Are bits 3-0 really LEDs? Does anything ever set bits
   4-7? topskatr's writes say no.
4. **DSP use.** *Answered:* yes, a 108-step program (§2.9).
5. **EG timing.** MDF's and MAME's rate formulas differ (octave weighting,
   FNS bit 9). Use MDF's, and compare a known sound's decay by ear if
   possible.
6. **PLFO depth.** MDF's arithmetic gives about ±190 cents at PLFOS=7 while
   the manual and MAME use ±494 cents. topskatr's voices seen so far set
   PLFOS=0, so it doesn't matter here; `crates/sound` uses the manual's
   table.
7. **Link topology.** Parallel (MAME/SM) or daisy-chained through the DSB2's
   MIDI-thru (§3.3)? The sound 68k does **not** treat channel 14 specially,
   but topskatr's `AE` commands make no SCSP sound (§2.10), so both
   topologies give the same output for this game. The real wiring is
   still unknown.
8. **`0x01CE0000`** on the i960 side, written 0/1 around UART init: a sound
   reset? If it holds the 68k in reset, a model must keep the 68k halted
   until the i960 writes 1.
9. **DSB2 315-6028A.** What does the 256-byte boot upload do? What is
   `0xE00001` for (address/command half of a 16-bit port)? *Answered:* the
   start/end commands take byte addresses into the 16 MB space (§3.7).
   Still open: the gain registers' scale (linear or log) and the `64`
   opcode's exact meaning.
10. **DSB2 timer rate** (IRQ2): 1 kHz is a guess shared by MAME and SM.
    The firmware only counts ticks in steps of 80 for fades and the
    loop-point write, so it cannot confirm the rate. Fade length on real
    hardware would.
11. **MPEG ROM header** `0x0000–0x10FFF` of `.18s`: what reads it?
12. **SCSP 68k bus waits.** MAME's 1-cycle guess. It only matters if music
    tempo is visibly off; MDF notes the same issue on the Saturn.

---

## 6. Jaleco's driver on `overrev` (ROM, H unless marked)

`overrev`'s sound program `epr-20002.31` (512 KB, about 90 KB used) is
Jaleco's own driver, not CRX_drv: no driver string, a different layout,
and its own conventions. It runs on the same board model with **no
changes**: everything it uses is already covered above. Found by running
it (`make run GAME=overrev INPUT=coin-start STEPS=900000000`, then `make
sound GAME=overrev`) and reading the code with `make sound-disasm
GAME=overrev`.

- **Reset.** SSP `0x5000`, PC `0x601200`. The vector table is copied to
  RAM from ROM `0x600000` as by CRX_drv; reset and all unused vectors point
  at `0x60186E`, which restarts the driver.
- **Init.** Clears all 512 KB of RAM, sets `0x100400` high byte to 3
  (MEM4MB and DAC18B), then probes the RAM size: it writes `0x12345678`
  to RAM 0 and loops forever if it shows up at `0x40000`, so it needs the
  whole 512 KB (no mirroring at 256 KB). Then it loads a DSP image (COEF,
  MADRS, MPRO: `0x500` bytes copied to `0x100700`), sets up the slots
  (KYONEX with KYONB clear, so all keyed off), the ring (`0x100402` =
  `0x0138`: RBL 2, 32K words at `0x70000`, as `topskatr`), interrupts and
  timers, reads MIBUF four times to flush the MIDI FIFO, and then
  **busy-waits about 2.2 s** (`0x8000` × 20 `muls.w`) before setting MVOL
  15 and unmasking interrupts. So the first commands are taken about 3 s
  after reset.
- **Interrupts.** SCIEB `0x01C8`: MIDI in (bit 3), timers A, B and **C**
  (bits 6, 7, 8). SCILV0 `0x48`, SCILV1 `0x88`, SCILV2 0: MIDI in →
  level 3, timer A → level 1, timer B → level 2, and timer C shares timer
  B's level (bits 7-10 of SCIPD all take SCILV bit 7's level; SCSP
  p04_2c). CRX_drv never enables timer C. The level 2 handler tests
  SCIPD bit 7 to tell timer B from C, reloads that timer from RAM and
  clears its SCIRE bit. Timer A: TACTL 3, `0xC0` (512 samples, 86 Hz);
  B and C: ÷1, `0xCE` (50 samples, 882 Hz).
- **MIDI.** The level 3 handler (`0x6017A2`) reads one MIBUF byte per
  entry into a ring in RAM; `0xFF` restarts the driver. The i960 sends the
  same `A0 xx yy` messages and one `0xFF` at init as in the Sega games. The
  `0xFF` arrives during the driver's first busy-wait and waits in the
  FIFO, so the driver initialises twice and is ready about 5.7 s after
  reset (H, by running it). Nothing is ever written to MOBUF.
- **Voices.** All 8-bit PCM (PCM8B), no FM, LFO depth 0, LPCTL 0 or 1.
  Long sounds, the music included, are **streamed** from sample ROM into
  8 KB double buffers (LEA `0x1FFF`) refilled when CA (`0x100409` bit 7,
  after selecting the slot with MSLC) shows the other half playing, much
  as CRX_drv does; others play from RAM outside the stream buffers
  (SA `0x50000`, LEA `0x9980`, for example).
  Most voices play at OCT −1 (22.05 kHz) with DISDL 3 (−24 dB), so the
  mix is quieter than the other games': over 75 s of `make sound` its mean
  is −40.2 dB (peak −21.9 dBFS) against −30.9 to −37.6 dB (peaks −10.2 to
  −16.6) for the other sound boards. The manifest's `volume_db: 6.0`
  stands in for the cabinet's volume pot (a listening choice; the ROMs
  don't set the analog gain).
  Direct-out level, pan and the effect send (ISEL 0 or 1, IMXL 1-3) are
  written per voice, and the driver writes the whole `+0x16` word, EFSDL
  3 / EFPAN 0 included.
- **DSP.** The init image is an 80-step program: 28 memory steps, all on
  odd steps, each MEMS write (IWT) taking the read two steps before; it
  reads MIXS 0 and 1 and writes EFREG 1-4 (an echo/reverb). The driver
  loads other programs on command (5,120 MPRO writes in a boot-to-race
  run; a 42-step one writing only EFREG 0 was loaded at the end).
  Nothing uses TABLE, ADRL, FRCL, NOFL or EXTS, so the open DSP
  questions in §2.9 and §4.1 don't matter for it.
- **Board latch `0x400001`.** Bits 3-0 count down through init (`0x3F`,
  `0x3E`, `0x3C`, `0x38`, `0x30`) and then blink; **bits 4 and 5 are
  always set** (the base value is `0x30`), where `topskatr` keeps them
  clear. MAME takes bit 5 as a sample bank select for boards with more
  than 8 MB of samples (§1); `overrev` has exactly 8 MB, and its music
  and effects come out as recognisable sound with the plain linear map,
  so the bits are ignored here. *Unconfirmed:* what bits 4-5 do on the
  real board.
- *Seen, unconfirmed:* while the driver restarts with MVOL still 15, its
  RAM clear runs through the DSP ring, and the DSP reads the zeroed words
  as large values (16-bit float `0x0000` has the hidden bit set, so it
  is +0.5 full scale, not 0): about 30 ms of DC (2,046) comes out until
  the restart clears MVOL. Whether the real board makes the same click
  isn't known.

## 7. EPL's driver on `bel` (ROM, H unless marked)

`bel`'s sound program `mpr-20249.31` (128 KB) has no driver string either,
and it is largely the same code as Jaleco's driver in §6: about half of
its 32-byte windows (2,127 of 4,096) appear in `overrev`'s `epr-20002.31`,
and it starts the same way (SSP `0x5000`, PC `0x601200`). Presumably both
companies used the same third-party library (*unconfirmed*). It runs on
the board model with **no changes**; found with `make run GAME=bel
INPUT=coin-start STEPS=900000000` and `make sound GAME=bel`.

- **Getting commands at all** needed an i960-side change: `bel` sends its
  sound commands from the UART's transmit interrupt (I/O request bit 10,
  i960 IRQ3), not by writing the bytes in a loop as the other games do.
  The handler (`0x6F930`) masks bit 10, clears the request until it reads
  clear, writes one byte to `0x1C80000` and enables bit 10 again; when
  its queue is empty it leaves the bit masked, and queuing a command
  enables it. Without a TxRDY interrupt nothing was sent. The board model
  now raises request bit 10 whenever it is enabled, the UART's command
  register has TxEN set and the transmitter took its last byte at least a
  byte time ago (10 bits at 31.25 kbaud). The other games never enable
  bit 10, so they're unaffected (their command streams and renders are
  byte-identical).
- **Commands.** The same `A0 xx yy` three-byte messages as the Sega games;
  2,214 bytes boot to game over in a coin-start run. Nothing is written
  to MOBUF.
- **Init.** Like §6: interrupts unmasked at 2.65 s, the first key-on at
  7.3 s after reset (the busy-wait and the double initialisation).
- **Voices.** All 8-bit PCM, OCT −1 or 0 (22.05 or 44.1 kHz), looping or
  one-shot, no FM (MDL 0), LFO written with zero depth. The 8 MB of
  samples (`.32`-`.35`, 2 MB each, concatenated in that order) are
  streamed into RAM as in §6. All 32 slots get used; music plays
  throughout, at about −35 dBFS RMS (peak −13 dBFS) over a run.
- **DSP.** An 80-step program with 28 memory steps that reads MIXS 0 and 1
  and writes EFREG 1-4, the same shape as `overrev`'s; slots 0-2 send to
  it (EFSDL 3).
- *Unconfirmed:* whether the music and effects sound as on the board
  (they come out continuous and unclipped, with no silent gaps after the
  driver starts), and what the board latch's upper bits do (the last
  value written is `0x1B`).
