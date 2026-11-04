# MAME defects found while building Remodel

A record of places where MAME's emulation differs from what the hardware
does, kept so they can be turned into upstream patches. MAME marks
`topskatr` as `MACHINE_NOT_WORKING`; several entries below are plausible
reasons why.

**Scope and method.** Remodel is clean-room: we read MAME for facts, never
copy its code. A patch to MAME would be written fresh against MAME's tree and
its licence, using the evidence here. Line numbers refer to `mamedev/mame`
master at commit `64eae1fe` (fetched 2026-11-05). Re-check them before
patching.

**Status levels**

| Status | Meaning |
|---|---|
| Confirmed | The game's own code or output demonstrates the defect, and MAME's code was read to confirm what it does. |
| Suspected | MAME's code contradicts other sources or itself, but the game hasn't exercised the path yet (or doesn't use it). |
| Known hack | MAME's own comments admit a placeholder or guess. |

To add an entry: give it the next ID, say how to reproduce it with Remodel,
and keep the evidence concrete (addresses, counts, outputs).

---

## Confirmed

### MD-1. MB86235: absolute addressing mode (0xC) is fatal
- **Where:** `src/devices/cpu/mb86235/mb86235ops.cpp`, `mb86235_device::decode_ea` (switch around lines 100-178; `fatalerror` at line 180).
- **MAME does:** the switch has cases for modes 0x0-0xB and 0xD-0xF but none for 0xC, so mode 0xC hits `fatalerror("TGPx4: illegal decode_ea type ...")`.
- **Correct:** mode 0xC is a valid absolute address (the displacement field is the address). MAME's own disassembler (`mb86235d.cpp`) prints it as such.
- **Evidence:** the 4,096-word program `topskatr` uploads contains 8 mode-0xC transfers (program addresses 0xAC7, 0xAE1, 0xB08, 0xB1E, 0xBFB, 0xC00, 0xC18, 0xCE5). All 8 execute during boot and attract (Remodel's TGP coverage), so MAME would abort the moment the copro reaches the first one. **Likely a blocker for topskatr in MAME.**
- **Reproduce:** `make run STEPS=300000000`, then check `out/topskatr/copro-coverage.txt` for those addresses.
- **Fix direction:** add `case 0x0c: return disp;` (absolute, no post-modify). Remodel implements it this way and the game runs. The exact displacement width/sign for mode 0xC still deserves a check against more games.

### MD-2. System 24 tile chip: split mode applies the line-scroll entry as X scroll
- **Where:** `src/mame/sega/segaic24.cpp`, `segas24_tile_device::draw_common`, the `hscr & 0x8000` branch, `case 2: case 3:` (around lines 389-417; scroll set at lines 397-401).
- **MAME does:** with line scroll enabled in left/right split mode, it reads the line's table entry and uses its low 9 bits as **both** the split point and the X scroll of both maps.
- **Correct (per topskatr):** the table entry is only the split point (bit 9 picks which map is on the left). The maps keep their HSCROLL register scroll. topskatr's attract text ("INSERT COIN(S)" / "CREDIT 0") is centred in the map with zero scroll. Under MAME's reading it jumps sideways from frame to frame; under ours it stays put while the split wipes it in and out.
- **Evidence:** rendering the same frames both ways (`docs/hw/tilemap.md`, "Checks against topskatr").
- **Reproduce:** `make run STEPS=300000000 SNAP=86` and view `out/topskatr/frames/`.
- **Fix direction:** in that branch, keep `set_scrollx` from the HSCROLL register (`hscr` before it is replaced by the table entry), and use the table entry only for the split point and side. Affects Model 1, Model 2 and System 24 games that use split mode with line scroll; check one of them for regressions.
- **Related, suspected:** the top/bottom split case (`case 1:`, lines 381-384) uses the table entry as X scroll in the same way. topskatr doesn't use mode 1.

### MD-3. MB86235 disassembler and recompiler: relative branch target off by one
- **Where:** `src/devices/cpu/mb86235/mb86235d.cpp` (relative targets for DBLP/DBBC/DBBS and EF2 bit 12), plus `mb86235fe.cpp` / `mb86235drc.cpp`.
- **MAME does:** computes the target as branch address + displacement. The interpreter (`mb86235ops.cpp`) uses branch address + 1 + displacement.
- **Correct:** address + 1 + displacement (the interpreter is right).
- **Evidence:** in topskatr's program, all 4 DBLP loops land 1-2 words after their SETL with +1, and none do with +0.
- **Impact:** wrong labels in debugger disassembly. The DRC (disabled by `ENABLE_DRC 0`) would branch wrongly if enabled.
- **Fix direction:** add 1 in the disassembler and frontend relative-target paths.

---

## Suspected

### MD-4. MB86235 interpreter: MVD1 register-to-register moves are no-ops
- **Where:** `mb86235ops.cpp`, `mb86235_device::do_alu2_trans2_1`, `case 0:` (around lines 899-906 and 926-932).
- **MAME does:** for SD = 0 it reads the A source from bits 24..20 and the B source from bits 13..10, then writes them back to the **same** fields. That makes the move a no-op, and the B field is one bit narrower than the disassembler's.
- **Expected:** A source 24..20 to A destination 19..15; B source 14..10 to B destination 9..5 (B codes OR 0x20). This is the layout `mb86235d.cpp` prints.
- **Evidence so far:** topskatr's program has 44 non-trivial SD = 0 moves, all at 0xFBE and above. None execute during boot or attract (TGP coverage), and outputs are identical either way over 300M instructions. Promote to Confirmed once gameplay reaches them.

### MD-5. MB86235 interpreter: EBL write shifts the value into the high bits
- **Where:** `mb86235ops.cpp`, transfer-register write for code 0x12 (EBL).
- **Expected:** replace EB's low 14 bits. topskatr never writes EBL.

### MD-6. MB86235 interpreter: FEA/FES/FLOG arithmetic
- **Where:** `mb86235ops.cpp`, ALU ops 0x08, 0x09 and 0x0C.
- **MAME does:** FEA/FES keep only the exponent bits, and FLOG computes ln(x)/log10(2) rather than log2(x).
- **Expected:** FEA/FES scale by a power of two via the exponent, and FLOG is log2. topskatr doesn't use these ops.

### MD-7. MB86235: unimplemented MOV4/ICDTR, MOVI and MVD2 field disagreement
- **Where:** `mb86235ops.cpp`: MOV4 is executed as MOV2, MOVI isn't executed at all, and MVD2 memory-to-register takes the register from the destination field.
- topskatr's program uses none of these (no format-4 words).

### MD-8. Model 2C: one TGPx4 emulated, PCB note lists two
- **Where:** `src/mame/sega/model2.cpp` (header PCB notes vs the `model2c` machine config).
- **Now strong evidence (promote with MD-10):** topskatr uploads a second 4,096-word MB86235 program through 0x00804000 with 0x00980008 bit 31 set (i960 0x47EC0). That program is the geometrizer, and running it in Remodel produces the full rasteriser stream. MAME counts these words and discards them.

### MD-9. topskatr inputs overlap the EEPROM read-mode bits (resolved: not a defect)
- **Where:** `model2.cpp`, the `topskatr` input ports. They put four buttons on IN0 bits 4-7, the bits that show EEPROM DO and fixed 1s when port A control mode is on.
- **Resolved: not a defect.** Every button in MAME's layout is confirmed in Remodel from the game's response to a scripted press:
  - coin 1 (IN0 bit 0) gives "CREDIT 1";
  - start (IN0 bit 6) leads to course select;
  - select right and left (IN0 bits 4 and 7) move the course highlight;
  - jump front (IN0 bit 5) and jump tail (IN1 bit 0) press the nose and the tail;
  - the two analog channels steer and slide.

  The overlap with the EEPROM bits is harmless because the game reads buttons outside control mode. Kept here so nobody re-investigates it.

### MD-10. Model 2B/2C geometrizer is HLE'd from one game's program, not run
- **Where:** `src/mame/sega/model2_v.cpp` (`geo_parse` and its command handlers), `model2.cpp` (the geo upload in `geo_ctl_w`).
- **MAME does:** discards the uploaded geometrizer program and runs a fixed C++ HLE modelled on a 2B program.
- **Correct (per topskatr):** the game's program differs from that HLE in at least four ways, all **Confirmed** by running it in Remodel (`docs/hw/geometrizer.md`):
  1. Command 0x11 is a compact skinned model format (attr, then 1-2 points, no stored normals; bit 0 of X selects a second matrix). MAME treats it as an alias of 0x01 with the 10-word format.
  2. Command 0x1B writes a secondary matrix. MAME treats it as an ordinary matrix overwrite.
  3. Command 0x1C copies matrix 0 into matrix 1. MAME treats it as a translation.
  4. Commands 0x12, 0x13, 0x15, 0x17-0x1A and 0x1D-0x1F are invalid in this program and trap to its error loop. MAME aliases them.
- **Also observed:** the program's frame handshake is driven by flag input F0, and it emits a NOP to the rasteriser on every pass of its idle loop (a delay-slot write). MAME's HLE has no equivalent.
- **Impact:** with MAME's HLE, topskatr's models (sent as command 0x11) are decoded with the wrong record size, so its 3D cannot render correctly. Together with MD-1, this explains MAME's `MACHINE_NOT_WORKING`.
- **Fix direction:** run the uploaded program on an MB86235 core (LLE), at least for 2C. Otherwise, add a topskatr-specific HLE for 0x11, 0x1B and 0x1C.
- **Reproduce:** `make run STEPS=660000000 INPUT=coin1@1100-1104,start@1250-1254`. The report's geometrizer line shows list words turning into raster words, and `out/topskatr/raster-last.txt` holds the stream.

### MD-11. DSB2: 68000 clocked at 8 MHz; the PCB note says 11 MHz
- **Where:** `src/mame/sega/dsb2.cpp`, `device_add_mconfig` (`M68000(config, m_ourcpu, 8'000'000)`, line 62, commented "unknown clocks").
- **MAME does:** runs the DSB2's 68000 at 8 MHz.
- **Correct (probably):** MAME's own Model 3 PCB note for the DSB2 (`model3.cpp`, board 837-12273) gives "68EC000FN12, clock 11.000MHz [33/3]". Supermodel uses 11 MHz too.
- **Impact:** small. The firmware is interrupt-driven (UART IRQ1, tick IRQ2), but busy-wait delays and command latency scale with the clock.
- **Evidence:** `docs/hw/sound.md` §3.1. Remodel's `crates/dsb` runs the firmware at 11 MHz (`make music`); nothing in it depends visibly on the clock, so this stays unconfirmed from the game.
- **Fix direction:** use `33_MHz_XTAL / 3`.

### MD-12. DSB2: decoder port 0xE00001 unmapped, boot upload and busy flag not modelled
- **Where:** `src/mame/sega/dsb2.cpp`, `dsb2_map` (lines 52-55).
- **MAME does:** maps only `0xE00003` (command FIFO). `0xE00001` is commented "acknowledge FIFO writes?" and unmapped. The status at `0xE80001` always reads `0x01`.
- **Correct (per topskatr's DSB2 program `mpr-19760.2s`):** at boot the firmware writes byte pairs to `0xE00001`/`0xE00003` and then streams a 256-byte block from its ROM (`0xF28`) through both ports. It polls `0xE80001` **bit 1** (busy) before every write. So `0xE00001` is part of the decoder interface, apparently the other half of a 16-bit port.
- **Impact:** none observed. None of the uploaded bytes collide with the FIFO opcodes, and a never-busy status is harmless. Listed so that a future LLE of the 315-6028A knows the upload exists.
- **Evidence:** `docs/hw/sound.md` §3.2, §3.4 (ROM `0x5E8–0x672`).

### MD-13. SCSP: MIDI input FIFO is 32 bytes; the manual says 4
- **Where:** `src/devices/sound/scsp.h` (`m_MidiStack[32]`, line 138), `scsp.cpp` (`& 31` wrap, lines 976 and 1540).
- **MAME does:** buffers up to 31 received bytes before MIOVF/MIFULL.
- **Correct:** the SCSP manual (MIDI register page, p04_2a) gives the MIDI-in buffer as 4 bytes. MIFULL is set at 4 and MIOVF on the 5th byte.
- **Impact:** with accurate timing a deeper FIFO hides overruns. MAME probably keeps it deep because its CPU/serial interleave is coarse. Supermodel enlarges it for the same reason. Not a topskatr blocker.
- **Fix direction:** keep it deep only behind a "coarse timing" option, or fix the scheduling and use 4.
- **Evidence that 4 is enough:** Remodel's sound board runs the 68000 in one-sample (256-cycle) slices, delivers link bytes no faster than 31.25 kbaud and uses a 4-byte FIFO. topskatr's driver reads all 6,253 bytes of a boot-to-race capture with **0 overflows**. Reproduce: `make run STEPS=600000000 INPUT=coin1@1100-1104,start@1250-1254`, then `make sound` and read the `midi in` line.

### MD-14. DSB2: looping tracks stop at their end address
- **Where:** `src/mame/sega/dsb2.cpp`, `fifo_w` (start-address case around lines 140-156) and `sound_stream_update` (end handling around lines 270-291, loop code commented out).
- **MAME does:** ignores opcode `0x64`. A start address written while playing just overwrites the start register, and at the end address playback stops. The loop variables are never set.
- **Correct (per topskatr's DSB2 firmware):** 8 of the 12 tracks loop. For them the firmware sends `64`, `74` (play), then about 8 × 80 ticks later a new `14` start address: the loop point. Track 5 (the race music) is 15.7 s long and loops from 6 frames in, at `0x7AD144`. Supermodel's reading (a mid-play start write sets the loop point) fits the firmware.
- **Impact:** in MAME the race music would go silent after 15.7 s, and the other looping tracks after one pass.
- **Evidence:** `docs/hw/sound.md` §3.4 and §3.7 (firmware `0x848–0x8A0` and `0xAB6–0xAFE`, track table at `0x8000`).
- **Reproduce:** `make music` (after a `make run` into a race). The log shows `play 0x7ac3c4-0x7e9b04 ..., looping`, `loop point 0x7ad144`, then `end reached, loop to 0x7ad144` at about 37.5 s.
- **Fix direction:** on `0x64` arm looping. On a start write while playing, latch it as the next start. At the end, if armed, restart from the latched start; `0x84` disarms.

### MD-15. DSB2: gain commands ignored, so master volume and fades have no effect
- **Where:** `src/mame/sega/dsb2.cpp`, `fifo_w` (only `14/24/74/84` handled) and the constructor/reset (volume fixed at `0x7F`, pan 0).
- **MAME does:** drops `A0`-`A7` and their argument bytes. Volume and pan never change.
- **Correct (per topskatr):** the firmware writes `A0`, `A1`, `A4`, `A5` with gains derived from each track's volume, the host's master volume (`AE 02 vv`, sent before every track) and its fade-out (`AE 03 vv`, which steps the gains down to 0 over about 1.8 s before `84`). The best fit is a 2×2 L/R matrix (`docs/hw/sound.md` §3.4; the matrix layout is unconfirmed).
- **Impact:** music at a fixed level regardless of the game's volume, and fade-outs cut off abruptly. Because MAME does not track argument bytes after unknown opcodes, an `Ax` argument equal to `0x14`/`0x24`/`0x74`/`0x84` would also be taken as a command. topskatr's gain bytes vary with the master volume and each fade step, so such collisions are possible.
- **Reproduce:** `make music`: the log lists every gain write (`gain A0 L<-L = ...`).
- **Fix direction:** parse `A0`-`A7`/`B0`-`B7` as two-byte commands and apply the gains as a per-output mix.

---

## Known hacks (admitted in MAME's source)

| ID | Where | Note |
|---|---|---|
| MH-1 | `315_5649.cpp`, serial status register 0x0D | Hard-coded to 0x0C; the source calls it a hack. |
| MH-2 | `model2.cpp`, uPD71051/i8251 UART clock | 8 MHz is a guess, so the baud rate is unmeasured. |
| MH-3 | `model2rd.ipp` / `model2_v.cpp`, colour gamma | The gamma step is a calibration guess, not hardware. |
| MH-4 | `model2.cpp`, `reset_model2_scsp` | Copies the first 16 bytes of the sound program ROM into sound RAM so the 68000 finds a reset vector. How real hardware provides it is unknown. topskatr's driver installs its own vector table in RAM (`docs/hw/sound.md` §1), and before that copy it reads no RAM below `0xC04`, so only SSP/PC at reset matter. Remodel fetches them from the program ROM during reset, which plays identically. |
| MH-5 | `model2.cpp`, `topskatr` machine config (line 3127) | MPEG output goes straight to the speaker. The TODO says it "should be chained with SCSP EXTS", but the DSB2 has its own DACs, so analog mixing is more likely (`docs/hw/sound.md` §3.6). |
