# Model 2C-CRX peripherals touched during init

Research notes for the board model (`crates/model2`). Everything here is a
paraphrase; no source code was copied. Each fact carries a source tag and a
confidence level.

Confidence: **H** = primary datasheet or two independent sources agree;
**M** = one credible source, consistent with others; **L** = single source,
inference, or contradicted somewhere. **[MAME-only]** = the only source is
MAME, which does not run topskatr (`MACHINE_NOT_WORKING`). Verify against
the game's own code before relying on it.

Sources (short tags used below):

| Tag | Source |
|---|---|
| MCHP | Microchip DS21749H, 93AA46A/B/C, 93LC46A/B/C, 93C46A/B/C datasheet (2010). Mirror: https://www.rcscomponents.kiev.ua/datasheets/93xx46x.pdf |
| INTEL | Intel 8251A Programmable Communication Interface, order no. 205222-002 (Nov 1986). Mirror: https://mark-ogden.uk/files/intel/publications/205222-002%208251A%20Programmable%20Communication%20Interface-Nov86.pdf |
| NEC | NEC uPD65881GB-P02 datasheet (uPD71051 replacement, doc A18499EJ2V0DS00). https://assets.sourcengine.com/datasheets/28e1e181-a9fd-49fb-b643-5a8a73831ea9.pdf |
| SCSP | Sega SCSP User's Manual (1997), register map. https://www.infochunk.com/saturn/segahtml_en/hard/scsp/hon/p04_12.htm |
| MAME | MAME master, `src/mame/sega/model2.cpp`, `315_5649.cpp`, `dsb2.cpp` (fetched 2026-11). https://github.com/mamedev/mame |
| MAME150 | MAME 0.150 `src/mame/drivers/model2.c` (older HLE of the sound serial port) |
| SM | Supermodel (Model 3 emulator), `Src/Model3/DSB.cpp`, `SoundBoard.cpp`. https://github.com/trzy/Supermodel |
| MAN | Top Skater operator's manual, Sega Enterprises USA, no. 4201-6308-01 (May 1997). https://www.segaretro.org/images/5/53/TopSkater_Model2_US_Manual.pdf |
| AC | Arcade Controls forum thread on measured Model 2 refresh (2012). https://forum.arcadecontrols.com/index.php/topic,129135.0.html |
| SR | Sega Retro "Model 2" page (via search snippet; the site blocks direct fetch). https://segaretro.org/Model_2 |
| S16 | System16 Model 2C-CRX page. https://system16.com/hardware.php?id=717 (403 to fetchers; data from search snippet) |

---

## 1. 93C46 serial EEPROM (64 x 16)

### Pins and electrical behaviour (MCHP, H)
- Four signals: CS (chip select, active high), CLK, DI (serial in), DO (serial out).
- DI is sampled on the **rising edge of CLK**. Read data on DO also changes on rising edges and is valid tPD (≤200 ns at 5 V) later.
- DO is high-impedance except while it shifts out read data or reports ready/busy. It goes back to high-Z on the falling edge of CS.
- The device does nothing until it sees a **start bit**: the first rising CLK edge with CS and DI both high. Before that, CS, CLK and DI can wiggle freely. While CS rises, CLK or DI must be low, so that CS rising does not itself count as a start condition.
- An instruction runs only after all of its opcode, address and data bits have been clocked in.
- 'B' parts are fixed at x16. 'C' parts select x16 when ORG is tied high. In x16 mode the address is **6 bits** (A5..A0, 64 words).
- At power-up the device is **write-disabled** (EWDS state). A read works whatever the EWEN/EWDS state is.

### Instruction set, x16 (MCHP Table 1-3, H)
Each instruction is the start bit (1), a 2-bit opcode, then 6 address bits, sent MSB first.

| Instr | Op | Address field | Data in | DO after | Clocks |
|---|---|---|---|---|---|
| READ  | 10 | A5..A0 | – | dummy 0, then D15..D0 | 25 |
| WRITE | 01 | A5..A0 | D15..D0 | ready/busy | 25 |
| ERASE | 11 | A5..A0 | – | ready/busy | 9 |
| EWEN  | 00 | 11xxxx | – | high-Z | 9 |
| EWDS  | 00 | 00xxxx | – | high-Z | 9 |
| ERAL  | 00 | 10xxxx | – | ready/busy | 9 |
| WRAL  | 00 | 01xxxx | D15..D0 | ready/busy | 25 |

### Read (MCHP, H)
- Once the last address bit (A0) is in, DO stops being high-Z and drives a **dummy 0**. The next 16 rising edges then shift out D15 first, down to D0.
- If CS stays high, the address counter moves on and the next word follows straight away (sequential read).
- If DI and DO are tied together, they can conflict during the dummy-zero slot. On Model 2 they are on separate I/O-port bits (see §4), so this does not apply.

### Write, erase and ready/busy (MCHP, H)
- 93AA/93LC parts start the self-timed program cycle on the falling edge of CS after the last bit. 93C parts start it on the rising CLK edge of the last bit.
- To poll: hold CS low for at least tCSL (250 ns), then raise CS again. DO reads **0 while busy and 1 when ready**. Status is valid within tSV (≤200 ns).
- The ready/busy indication stays on DO until the host sends a start bit and then drops CS.
- Worst-case cycle times: ERASE/WRITE 2 ms (93C) or 6 ms (93LC/AA), ERAL 6 ms, WRAL 15 ms. ERAL and WRAL need Vcc ≥ 4.5 V.
- 5 V timing: tCSS ≥ 50 ns, tCKH ≥ 250 ns, tCKL ≥ 200 ns, fCLK ≤ 2 MHz (3 MHz for 'C' parts). The i960 bit-bangs this through an I/O chip, so the real bus is far slower than these limits.
- Model hint: completing writes instantly (DO reads 1 at the first poll) is legal behaviour, because tWC is only a maximum.

### On Model 2
- MAME instantiates a 93C46 in 16-bit mode on 2A, 2B and 2C (MAME, M). Older MAME (MAME150) did the same.
- For the port-bit wiring, see §4.

---

## 2. Intel 8251A USART (Model 2 part: NEC uPD71051 "D71051GU-10")

The board's part is the NEC uPD71051 (MAME PCB notes for 2C board 837-12469-01, M). NEC describes its successor, which is "same functions/commands as uPD71051", as functionally equivalent to the uPD8251AF (NEC, M). Treat its programming model as 8251A.

### Register select (INTEL, H)
The C/D input selects data or control. Reading and writing both have a C/D=0 and a C/D=1 target:

| C/D | RD | WR | Operation |
|---|---|---|---|
| 0 | 0 | 1 | read RX data |
| 0 | 1 | 0 | write TX data |
| 1 | 0 | 1 | read **status** |
| 1 | 1 | 0 | write **mode or command** (which one depends on internal state) |

**Model 2C mapping:** in MAME the USART sits at 0x01C80000–0x01C80003 on the low byte of each 16-bit half (MAME, M). That puts data at byte 0x01C80000 and control/status at 0x01C80002. A 32-bit read therefore returns data in bits 7:0 and status in bits 23:16. MAME150 used the same layout and returned its status hack in bits 23:16. This matches the existing `bus.rs` assumption.

**Model 2B mapping** (from `overrevb`'s code, confirmed by running it): data at 0x009C0000 and mode/command/status at 0x009C0004, one register per 32-bit word with the byte in bits 7:0. Its init writes 0, 0, 0, then 0xD0 (internal reset), mode 0x4E and command 0x37, and polls status bit 0 (TxRDY) before each byte (`model2b.md`).

### Initialisation sequence (INTEL, H)
1. After a reset, the **first** control write is the **Mode instruction**.
2. In sync mode, one or two sync-character bytes follow. In async mode nothing follows.
3. Every control write after that is a **Command instruction**.
4. Setting command bit 6 (IR) sends the chip back to step 1.
5. At power-up the chip may be in any of these states, so Intel recommends a worst-case sequence: three 0x00 bytes to C/D=1, then 0x40 (internal reset), then the real mode byte and command byte. Expect the game to do exactly this.
6. Recovery time between control writes during mode setup is 6 tCY. For async data writes it is 8 tCY (16 tCY in sync mode). Data may be written only while TxRDY=1.

### Mode instruction, async (INTEL, H)
| Bits | Field |
|---|---|
| 1:0 | baud factor: 00 = sync mode, 01 = x1, 10 = x16, 11 = x64 |
| 3:2 | character length: 00 = 5 bits … 11 = 8 bits |
| 4 | parity enable |
| 5 | even parity (1) or odd (0) |
| 7:6 | stop bits: 01 = 1, 10 = 1.5, 11 = 2 (00 is invalid) |

For example, 0x4E is 8N1 at x16. This is a typical value; it has not yet been confirmed from topskatr's code.

### Command instruction (INTEL, H)
| Bit | Name | Meaning when 1 |
|---|---|---|
| 0 | TxEN | transmit enable |
| 1 | DTR | drive /DTR low |
| 2 | RxE | receive enable |
| 3 | SBRK | force TxD low (send break) |
| 4 | ER | clear the PE, OE and FE flags |
| 5 | RTS | drive /RTS low |
| 6 | IR | internal reset (back to mode format) |
| 7 | EH | enter hunt (sync mode only) |

### Status register (INTEL, H)
| Bit | Name | Notes |
|---|---|---|
| 0 | TxRDY | **Status bit = TX buffer empty only.** The TxRDY *pin* also requires /CTS low and TxEN=1. |
| 1 | RxRDY | a received character is waiting |
| 2 | TxEMPTY | the transmitter is fully idle (shift register empty too) |
| 3 | PE | parity error (cleared by ER) |
| 4 | OE | overrun error (cleared by ER; the earlier character is lost) |
| 5 | FE | framing error, async only (cleared by ER) |
| 6 | SYNDET/BRKDET | sync detected; in async mode, a break was detected |
| 7 | DSR | the /DSR input is low |

- A status update can lag the event that caused it by up to 28 clocks (INTEL, H).
- CLK must be faster than 30× the bit rate in x1 mode, or 4.5× the bit rate in x16/x64 mode (INTEL, H).

---

## 3. Sound board link (i960 ↔ 68EC000 + SCSP) and DSB2

### Topology
- **Path to the sound board:** i960 → uPD71051 TxD → **SCSP MIDI-in**. The sound 68000 reads the bytes from the SCSP MIDI input FIFO (MAME, SM for Model 3; M).
- **Path back:** SCSP MIDI-out → uPD71051 RxD (MAME, M).
- The SCSP exposes its MIDI port to the 68000 through two registers (SCSP, M):
  - 0x100404: MIBUF (input byte in bits 7:0), plus flags for output-FIFO full/empty and input-FIFO overflow/full, and input-empty in MAME's implementation.
  - 0x100406: MOBUF (output byte).
  On Model 2 the SCSP register block sits at 68k address 0x100000 (MAME, M).
- **Baud rate:** 31.25 kbaud, the MIDI standard, from a 500 kHz clock at x16 (MAME, M). MAME's own comment calls this "standard Sega/MIDI" without citing a measurement. The original Model 2 with the Model 1 sound board uses the same arithmetic (16 MHz / 2 / 16). MAME clocks the UART itself at 8 MHz with the comment "clock unknown".
- **i960 interrupt:** request bit 10 is set while the UART's RxRDY or **TxRDY pin** is active and the bit is enabled. It reaches i960 IRQ3 (bits 10–11 are ORed onto IRQ3) (MAME-only, M). Because the TxRDY pin is gated by TxEN and /CTS, it does not behave like the status bit, so a constantly ready transmitter keeps re-raising the interrupt. MAME ties /CTS low. reModel models the TxRDY side only, as `bel` needs it (it sends its sound commands a byte per interrupt; `docs/hw/sound.md` §7): request bit 10 is raised while enabled, TxEN is set in the command register and a byte time (10 bits at 31.25 kbaud) has passed since the last data byte. The other games never enable bit 10. RxRDY is never raised (nothing is received).

### Command protocol
- **There is no public document of the i960→sound command bytes.** The game-specific 68000 program in the `audiocpu` ROM interprets them, and MAME simply runs that code. No source describes a fixed handshake. **L: unknown.**
- **Reply / ack:** the MAME150 HLE (before the 68k was emulated) made the status always read 0x07 (TxRDY, RxRDY and TxEMPTY all set) and RX data always 0xFF. Its comment says one game (Zero Gunner) also needed bit 4. This strongly suggests some games poll RxRDY and read replies at boot (MAME-only, L).
- **Action for us:** find i960 reads of 0x01C80000/2 in topskatr, and 68k writes to MOBUF (0x100406) in the sound program. Together these show whether the sound driver echoes or acknowledges commands, and what values the i960 expects.
- The current `bus.rs` returns status 0x05 (no RxRDY), so it will never deliver a reply. If init spins on RxRDY, that is the first place to look.
- **Observed in topskatr (reModel `run`, `out/topskatr/sound-tx.txt`; confirmed):**
  - **Initialisation.** On frame 88 the game writes the UART control register 00, 00, 00, 40 (the 8251 software reset sequence), then mode 0x4E (async ×16, 8 data bits, no parity, 1 stop bit), then command 0x37 (TxEN, DTR, RxEN, error reset, RTS). It then sends 0xFF.
  - **Commands are MIDI-shaped.** Each is a status byte of 0x80 or above followed by two data bytes below 0x80, for example `AE 10 00`, `A0 1A 02` and `B6 50 00`. A0 to AF (polyphonic key pressure in MIDI terms) dominate, and B6 to B8 (control change) appear in bursts.
  - **Commands follow events.** `A0 15 02` is sent on the frame a coin is inserted. About 6,000 bytes are sent from boot to mid-race.
  - **No reply needed so far.** The game never waits for a reply: with status stuck at 0x05 (TxRDY, TxEMPTY, never RxRDY) it runs through attract into gameplay.

### DSB2 (MPEG digital sound board, 68000-based)
- Both MAME (Model 2 topskatr) and Supermodel (Model 3) feed the **same host serial byte stream** to the DSB in parallel with the SCSP. The DSB sees every command byte, and its firmware decides which ones apply to it (MAME, SM; M).
- In MAME's topskatr config, the DSB2's own TxD is **not** connected back to the main board (MAME-only). Whether the real board returns anything is unknown.
- **DSB2 internals** (MAME, SM; M):
  - CPU and memory: a 68000 (clock unknown; MAME uses 8 MHz) with program ROM at 0x000000 and work RAM at 0xF00000.
  - Command input: an 8251-type UART at 0xC00000 (data) and 0xC00002 (status), also at 31.25 kbaud. A received byte raises 68k IRQ1. Supermodel instead treats 0xC00003 bit 0 as "command valid".
  - IRQ2 is a periodic timer, about 1 kHz in MAME.
  - 0xD00001 is a system-control latch (bit 3 = MPEG ROM bank).
  - 0xE00003 is a byte FIFO to the MPEG decoder ASIC, and 0xE80001 is MPEG status.
- **MPEG ASIC FIFO opcodes, 68k side, not host commands** (MAME, M): 0x14/0x15 set the start address (3 bytes follow, MSB first). 0x24/0x25 set the end address. 0x74/0x75 play. 0x84/0x85 stop. The low bit selects player 0 or 1.
- The host-level DSB2 command set is defined only by the firmware in the topskatr `mpegcpu` ROM, which we have. It is not publicly documented (L).

---

## 4. Sega 315-5649 I/O controller and Model 2C wiring

**This whole section is MAME-only** (`315_5649.cpp`, by Dirk Best). I found no datasheet or pinout. It is a 100-pin QFP on the 2C board (MAME PCB notes).

### CPU addressing (MAME, M)
- The chip has 16 byte-wide registers at 0x01C00000–0x01C0001F, using the low byte of each 16-bit half.
- Register **N** is at byte address 0x01C00000 + (N>>1)·4 + (N&1)·2. A 32-bit read of word k returns reg 2k in bits 7:0 and reg 2k+1 in bits 23:16. This matches `bus.rs`.

### Register map [MAME-only]
| Reg | Read | Write |
|---|---|---|
| 0x00–0x06 | ports A–G: pin state if the port is an input, last written latch if it is an output | port A–G output latch |
| 0x08 | (reads 0xFF) | **port direction**, one bit per port A..G; **1 = input**. Reset value 0xFF (all inputs) |
| 0x09 / 0x0A | – | RS-422 channel 1 / 2 TX byte |
| 0x0B / 0x0C | RS-422 channel 1 / 2 RX byte | – |
| 0x0D | RS-422 status: bit 0/1 = TX1/TX2 buffer full, 2/3 = RX1/RX2 buffer full, 4/5 = framing error, 6/7 = unknown "IE". MAME hard-codes 0x0C (a hack) | – |
| 0x0E | – | mode: bit 7 = port G counter mode, bit 5 = RS-422 satellite mode, bit 4 = loopback, bits 3:0 = satellite number |
| 0x0F | **analog value**, 8 bits, for the current channel; each read **advances the channel by 1** (wrapping 7→0) | select analog channel (bits 2:0) |

In counter mode, reads of port G come from four 16-bit counters. The low 3 bits of the port G latch select the counter and byte, and auto-increment. Top Skater does not use this.

### Model 2C hookup (MAME `model2c`/`topskatr`; M, but unverified on topskatr)
| Port | Dir | Use |
|---|---|---|
| A (0) | out | bit 0 = "ctrlmode" (puts the EEPROM DO onto port B); bit 5 = EEPROM **DI**; bit 6 = EEPROM **CS**; bit 7 = EEPROM **CLK** |
| B (1) | in | IN0: coin 1/2, test, service, plus cabinet buttons. **When ctrlmode=1**: bit 5 = EEPROM **DO**, bits 7, 6 and 4 read as 1, bits 3:0 are still IN0 |
| C (2) | in | IN1 |
| D (3) | in | IN2 |
| F (5) | out | lamps (start and select lamps per MAN's output test; exact bits unknown) |
| G (6) | in | DIP SW3, 8 positions. SW1 and SW2 are on-board push buttons |
| AN0 | in | **CURVING** (board tilt) |
| AN1 | in | **SLIDE** (board left/right swing) |

- **EEPROM bit-bang recipe implied by this wiring:**
  1. Write port A with bit 0 set and the desired CS/CLK/DI levels, toggling bit 7 for each clock.
  2. Read DO from port B bit 5.
  The older MAME150 driver used the same bit assignment, so it is stable across MAME versions, but MAME is the only source.
- **Topskatr input names** (MAN input test, H): CURVING, SLIDE, JUMP FRONT, JUMP TAIL, START, SELECT LEFT, SELECT RIGHT, COIN CHUTE #1/#2, SERVICE (on board), TEST (on board), SERVICE, TEST.
  - MAME puts Select Right, Jump Front, Start and Select Left on IN0 bits 4–7, and Jump Tail on IN1 bit 0 (MAME-only, L). Note that the IN0 bits clash with the EEPROM read mode above.
- **Analog ranges** (MAN, H for the values shown):
  - Both pots centre at **0x80**. The manual says "80H ±8" in one place and "180 ±8" in another; the second looks like a typo.
  - Example calibration screen: CURVING min/max 0x4F/0xB1, SLIDE 0x3E/0xC2.
  - The calibration is stored by the game, presumably in the EEPROM or backup SRAM (L).
  - Inputs are active low, so read 0xFF when idle.
- **Current `bus.rs` gaps against this model** (for information only, not changed):
  - no EEPROM: port A writes are ignored and port B never shows DO;
  - port directions are not modelled;
  - the analog channel select is only partly modelled.

---

## 5. Public hardware information: boards, clocks, video, IRQs

### Schematics and pinouts
- **No public Model 2 / 2C-CRX schematic was found.** Arcade-Projects forum threads have filter-board pinout photos for 2A and 2B/2C (power and JAMMA side only). Model 1 and Y-Board pinout PDFs exist on arcade-museum.com, but nothing for the 2C CPU board.
- **Top Skater manual (MAN)** is 107 pages. It covers cabinet wiring, the test menu and input/output tests (above), board-setting calibration, and the memory test. The memory test checks IC9–12, 15, 16, 79–84, 88 and 89 (useful for matching RAM test loops).
  - Game board assembly: 833-13080 (833-13080-01 USA).
  - The "wiring diagrams" section is missing from the scanned copy, and the parts-list pages are images only.

### 2C-CRX board components (MAME PCB note for HOTD 2C board 837-12469-01; M)
- **Crystals:** 32.000 MHz ×3, 50.000 MHz, 20.000 MHz, 45.158 MHz.
- **CPUs:** i960 (KB); MC68EC000FN12 (sound); **two** Fujitsu MB86235 (TGPx4).
  - MAME emulates only one MB86235. System16 lists one; LaunchBox lists two.
- **Custom chips:** 315-5687, 315-5673, 315-5798, 315-5799, 315-5725 ×2, 315-5292A, 315-5648, 315-5672, 315-5649 (I/O), and PAL 315-5879.
- **Other parts:** D71051GU-10 (USART), 2× Lattice pLSI2032, TDA1386T audio DAC.
- **RAM parts include:** M5M44170 and HM514270 (DRAM); HM538254 (VRAM); TC55V328, CY7C185, CY7C188 and IDT71256 (SRAM); LH521002 ×16 (fast SRAM, probably framebuffer/Z).

### Clocks (MAME; M unless noted)
| Item | Value | Basis |
|---|---|---|
| i960KB | 25 MHz (50 MHz / 2) | MAME; S16 also gives 25 MHz (H) |
| MB86235 | 20 MHz crystal | MAME |
| Sound 68EC000 | 11.2896 MHz (45.158 / 4) | MAME. The "FN12" part is rated 12 MHz, which is consistent |
| SCSP | 22.5792 MHz (45.158 / 2) | MAME; the Saturn SCSP also runs at 22.58 MHz (H for the chip) |
| i960-side timers | count down at 25 MHz, 4 channels at 0x00F00000 | MAME-only |
| UART baud | 31.25 kbaud | MAME-only (see §3) |

### Video timing (especially wanted)
- **Refresh 57.524 Hz** (H):
  - MAME's raw screen timing is a 16 MHz pixel clock (32 MHz / 2), 656 total × 424 total lines, 496 × 384 visible. 16e6 / (656 × 424) = **57.5241 Hz**, with **24.39 kHz** horizontal.
  - Independent support:
    - a forum member captured a real Daytona cabinet signal and got 57.524160 Hz, and the same for Sega Rally (AC);
    - Sega Retro lists 57.52416 Hz, 496×384 and a 24 kHz H-sync (SR);
    - the Sega Ages VF2 developers cited "57.5 fps like the original".
  - System16's "60 Hz" is a nominal figure.
  - **`bus.rs` uses `VBLANK_HZ = 60`, which is about 4.3% fast.** One frame is exactly 25 MHz × 278,144 / 16 MHz = **434,600 i960 cycles**.
- **Counter-claim** (L): the SR snippet also gives a "19.523 MHz pixel clock". That does not fit 656 × 424 at 57.52 Hz and is probably wrong.
- **Sync is programmable**: the System-24-style tilemap block has H-sync and V-sync registers at 0x01040000 and 0x01060000, mirrored at +0x100000 (MAME, M). The game may program its own timing at init; capture what it writes.
- **30 Hz mode:** bit 0 of the video control register at 0x0098000C makes the geometrizer start only on every other vblank (MAME-only, M). Reads of the same register expose a frame-parity bit.

### Interrupt controller (MAME-only, M)
- 0x00E80000: read = pending requests; write = **AND-mask acknowledge**. Write 0 to a bit to clear it.
- 0x00E80004: enable mask. MAME delays enable writes by about 80 ns because one game needs it.
  *Reads:* `lastbrnx`'s interrupt dispatcher (i960 `0xC30`, vector 240) loads the pair with
  `ldl 0xE80000` and services the lowest set bit of the *second* word until none is left, so
  that word can't be the enable mask (with it the vblank handler ran for ever, the interrupt
  never returned and the main program never ran; *confirmed*); it holds the requests. reModel
  returns request & enable there (*unconfirmed* whether the hardware mirrors the request
  register or masks it); no other game reads it.
- Mapping from request bits to i960 interrupt lines:

| Request bits | i960 line | Known source |
|---|---|---|
| bit 0 | IRQ0 | vblank (start of vblank) |
| bit 1 | IRQ1 | unknown |
| bits 2–9 | IRQ2 | bits 2–5 = timers 0–3 (fire when the 25 MHz count reaches 0) |
| bits 10–11 | IRQ3 | bit 10 = sound UART (RxRDY or TxRDY pin) |

The comm board also checks for vblank IRQs; it is absent on topskatr.

---

## Summary of disagreements and MAME-only claims

1. **Refresh rate:** MAME (57.524 Hz) agrees with the real-cabinet measurements. Our `bus.rs` (60 Hz) and System16's nominal 60 Hz do not. Sega Retro's 19.523 MHz pixel clock contradicts MAME's 16 MHz.
2. **TGPx4 count on 2C:** MAME's own PCB note lists two MB86235. MAME emulates one, and System16 says one.
3. **UART status at boot:** current MAME derives status from a real 8251 model. Old MAME forced 0x07 (RxRDY set, data 0xFF), and Zero Gunner apparently wanted OE set too. This hints that games expect replies from the sound board. Our 0x05 never delivers one.
4. **315-5649 serial status (reg 0x0D):** MAME hard-codes 0x0C by its own admission (a hack). The real value is unknown.
5. **Everything in §4 register layout, the EEPROM port bits, the IRQ map, the timers and the DSB2 internals** comes only from MAME (and Supermodel for the DSB). There is no datasheet.
6. **The UART chip is a uPD71051**, not an Intel 8251. Its programming model is compatible (NEC). The clock is unknown, and MAME's 8 MHz is a guess.
7. **Topskatr input bit assignments** in MAME are not verified. The input *names* match the operator manual, but MAME's IN0 bits 4–7 collide with the EEPROM read mode.
