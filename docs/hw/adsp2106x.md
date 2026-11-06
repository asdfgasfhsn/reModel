# Analog Devices ADSP-21062 "SHARC": decoder and interpreter notes

Status: 2026-11-06. The core is complete as an interpreter and runs both of
`overrevb`'s programs on the Model 2B board model (`docs/hw/model2b.md`),
which plays. The crate is `crates/adsp2106x`.

**Clean room.** Everything here was written from Analog Devices' public
documentation. No emulator's source code was read (MAME's, ElSemi's or any
other), and nothing from `rom/m2emu1.1a.zip` was used. Where the manual is
silent or contradicts itself, the choice is listed below with its reason.

## Sources

| Tag | Source |
|---|---|
| **[UM]** | *ADSP-2106x SHARC User's Manual*, Revision 2.1 (March 2004), 698 pages. analog.com resets scripted downloads; the same file is mirrored at `docs.ampnuts.ru/analog.com.datasheet/adsp-21062/related_data/50836807228561ADSP2106xSHARCProcessorUsersManual_Revision2_1.pdf`. Page numbers like "UM A-28" and sections like "UM §3.5" refer to it. Chapter 2 covers the computation units, 3 the sequencer, 4 the DAGs, 5 memory, 11.6 booting, Appendix A the instruction set, B the compute operations, C number formats, E the registers. |
| [UM2] | The Second Edition (July 1996) of the same manual, chapters 1-11 as separate PDFs (`signal.uu.se/Staff/pd/DSP/Doc/SHARC/`). Used only to cross-check text; it has no appendices. |
| [DS] | ADSP-21060/21062 data sheet, Rev. H. Not needed for the core. |

Downloads stay in `out/scratch/` and are never committed.

**Opcode field positions.** The text of Appendix A's opcode diagrams loses
the column alignment, so each field's bit range was recovered from the PDF's
word coordinates (`pdftotext -bbox`: a label's horizontal extent against the
row of bit numbers), and every type's field widths were checked to add up to
48 bits. The result was then confirmed on the Model 2B programs: their reset
vector decodes as `JUMP 0x20080 (DB)`, every interrupt vector as `RTI`, their
first instructions as MODE1/MODE2/DAG setup, and 0 of the 5,097 words reached
as code decode to undefined encodings.

## Layout

| File | Contents |
|---|---|
| `src/lib.rs` | `decode` (48-bit word to `Insn`/`Op`), the disassembler (`Display`, manual syntax), `Flow` for code discovery, universal register and condition names |
| `src/compute.rs` | The 23-bit compute field: ALU, multiplier, shifter, MR transfers, multifunction |
| `src/float.rs` | 40-bit floating point: exact significand arithmetic and one rounding |
| `src/mem.rs` | Internal memory as 16-bit columns (UM §5.3) |
| `src/cpu.rs` | `Sharc`: registers, sequencer pipeline, loops, interrupts, DAGs, memory access, IOP registers, host interface, save states |
| `src/cpu/exec.rs` | Instruction execution and the computation units |
| `src/cpu/recomp.rs` | What recompiled programs run on: one interpreter step of a constant instruction, blocks of straight-line instructions (below), dispatch, watching compiled words, `code_hash` |
| `src/codegen.rs` | The recompiler (`docs/recompiler.md`): code discovery, block regions and a Rust crate per program |
| `crates/cli/src/sharc.rs` | `remodel sharc-disasm` / `sharc-check`: listings, code discovery, a host boot and run |

`make sharc-test` runs the unit tests and, when the programs have been
captured (below), checks them; `make sharc-disasm FILE=...` writes a listing.

## What is implemented

**Instructions (UM Appendix A), all of them for the ADSP-21062:** types 1-7
(compute with dual DM/PM moves, conditional compute, universal register and
register file transfers with M-register or 6-bit immediate modifiers, register
moves, immediate shifts with a transfer, MODIFY), 8-13 (direct, PC-relative
and indirect jumps and calls with DB/LA/CI, ELSE compute, the jump-or-move
type 10, RTS/RTI with DB/LR, DO UNTIL with an immediate or register count or a
termination condition), 14-17 (direct and immediate-offset transfers,
immediate stores and loads), 18-24 (BIT SET/CLR/TGL/TST/XOR, MODIFY/BITREV
with immediates, PUSH/POP LOOP/STS/PCSTK, FLUSH CACHE, NOP, IDLE, CJUMP,
RFRAME). IDLE16 decodes but faults: it is ADSP-21061 only.

**Compute (UM Appendix B):** every fixed- and floating-point ALU operation
(Tables B.1, B.2), dual add/subtract, every multiplier operation with the 80-bit
MRF/MRB and all mod1/mod2 formats, SAT/RND/clear, MR transfers, every shifter
operation including FDEP/FEXT with sign extension, EXP, LEFTZ/LEFTO and
FPACK/FUNPACK, and both multifunction forms (Table B.7, multiply with dual
add/subtract). Flags: ASTAT (incl. compare accumulation and BTF) and STKY
sticky bits as Tables 2.5.4, 2.6.6 and 2.7.4 list them.

**Floating point:** the 40-bit format, RND32 (inputs lose their 8 LSBs,
results rounded to 23 fraction bits; an add or multiply of two normal
inputs is then done exactly in f64 and rounded once, the same result as
the exact significand path, which handles everything else), TRUNC (round toward zero; FIX toward
-Infinity), denormal inputs as zero, post-rounding overflow to Infinity or
NORM.MAX, underflow to zero, NaN results as all ones.

**Memory (UM Chapter 5):** two blocks of 16 columns x 4K 16-bit words, with
48-bit (5 groups of 3 columns, H M L / L H M as Figure 5.9a shows), 32-bit and
short-word views that alias exactly; block 1's aliases at 0x30000-0x3FFFF;
data width per block from SYSCON IMDW; PX transfers 48 bits on the PM bus and
40 on the DM bus. External and multiprocessor addresses go to the `Bus` trait
(`Data32` for DM bus register transfers, `Word48` for PM bus, PX and
instruction fetches).

**DAGs (UM Chapter 4):** pre/post-modify with M registers or immediates,
circular buffers (B, L; CB7/CB15 wrap flags and latches), bit-reverse mode on
I0/I8 and BITREV, 24-bit DAG2 with sign-extended M registers, B loads I, the
alternate register halves.

**Sequencer (UM Chapter 3):** the condition codes (Table 3.2, with the LT/LE
formulas), delayed and non-delayed branches, the PC stack (30), loop address
and counter stacks (6), the status stack (5), LCNTR/CURLCNTR/LADDR/PCSTK/
PCSTKP as registers, alternate register files and MR via MODE1, FLAG0-3 as
pins or outputs (MODE2), the timer.

**Interrupts (UM §3.6):** IRPTL/IMASK/IMASKP with priority, nesting (NESTM),
the status stack push for IRQ2-0/timer/VIRPT, RTI and JUMP (CI), interrupts
held off during branches and delay slots, IRQ2-0 lines (edge or level),
software interrupts, the arithmetic exception latches from STKY, VIRPT from
the host. Vectors are in internal memory (host boot mode).

**IOP registers and the host (UM Appendix E, §8, §11.6.3):** SYSCON (IMDW,
HPM/HMSWF host packing, SRST), VIRPT, WAIT, SYSTAT, MSGR0-7, BMAX/BCNT/ELAST
as storage, DMAC6 and the channel 6 parameters with EPB0 (6 deep) running
slave-mode receive DMA with 16-to-32 and 16-to-48 packing, LSW or MSW first.
The host side is `host_write`/`host_read`, one bus transfer at a time.
Anything else (other DMA channels when enabled, link ports, SPORTs, DMASTAT)
is reported, not ignored: a core access returns `Fault::Iop`, a host access
`HostError::Unimplemented`.

**Waiting on the host:** `Sharc::spinning` says when the core can only
repeat itself until a FLAG input changes: the last instruction was a taken,
non-delayed `IF [NOT] FLAGn_IN JUMP` to itself with no compute, or the
innermost loop is up to four NOPs ending on a FLAG input (or `FOREVER`),
with the timer off and no interrupt that could be taken. The board stops
stepping a SHARC then (both 2B programs wait this way); the result is the
same as stepping it.

**Reporting:** undefined encodings decode to `Op::Unknown` or
`Compute::Unknown` and execute as `Fault::Undefined`; stack overflow, a return
with an empty PC stack and fetches from where instructions can't be are faults
too. Nothing is skipped silently.

**Save states** cover memory, registers, stacks, pipeline, IOP registers and
the EPB0 FIFO (`remodel-state`); the decode cache is rebuilt.

## How execution is modelled

- **Pipeline.** The program can observe fetch/decode/execute (UM §3.1.1), so
  each cycle executes the word in the execute stage and fetches one more. A
  delayed branch leaves the two fetched words to run; a non-delayed one turns
  them into NOPs (counted in `cycles`). A non-delayed CALL returns to the word
  in the decode stage, a delayed one to the third word after it (UM §3.4.1).
- **Loops.** The termination test happens when the loop's last word is
  fetched, i.e. while the word two before it executes (UM §3.5, Figure 3.6),
  using ASTAT as it was at the start of that cycle; the counter decrements
  there too. A DO's new loop applies to the word fetched in the same cycle
  (Figure 3.8), and for a one-instruction loop the already-fetched word is
  fetched again through the loop logic. This reproduces, without special
  cases, every short-loop behaviour the manual describes (§3.5.1: one-word
  loops of 1, 2, 3 iterations; a condition becoming true "three more times";
  one or two more passes of two- and three-word loops). The unit tests check
  each of them.
- **Order within an instruction (UM §2.9).** All register reads happen first
  (a store takes the old value of a register the compute changes); flags
  update from the compute; then results land in precedence order: shifter,
  multiplier, ALU, PM, DM/universal register. An explicit ASTAT/STKY write
  supersedes the compute's flags (UM §2.5.3).
- **Not modelled:** effect latencies of system register writes (UM §E.2.1,
  one cycle), stall cycles (DAG register loads, PM bus conflicts, cache misses,
  wait states), the instruction cache, multiprocessing, link ports, SPORTs,
  DMA other than the host boot channel.

## Recompiled code: blocks

A recompiled program (`docs/recompiler.md`) runs most of its instructions
as blocks: straight-line runs of words executed one after the other, with
everything the pipeline model does per cycle (the tick of the timer and
interrupt latches, the fetch stage's loop test and slot moves, the
interrupt check, the step and cycle counters, the board's `after_step`)
done once at the block's end, for all of them (`Sharc::block_start`,
`block_exec`, `block_end` in `src/cpu/recomp.rs`). The state at the end is
what single steps leave, bit for bit; that holds because a block runs only
where that bookkeeping could do nothing but count:

- **The pipeline is the plain sequence**: the word executing at *a*, *a*+1
  in decode, *a*+2 being fetched. Each instruction of the block fetches the
  word two after it, and the block stops before the one whose fetch reaches
  the innermost loop's end (where the loop test happens), so the loop
  stacks and status aren't consulted inside it.
- **No interrupt can be taken**: IRPTEN is clear and the core isn't
  booting, and the timer is off. The latches still tick: the IRQ lines are
  constant during a run and STKY only gains bits inside a block (nothing
  in it writes STKY), so the last tick's latch, with STKY as it was before
  the last instruction, is the union of them all.
- **Which words** (`codegen::in_block`): no DO, stack operation, IDLE,
  unconditional branch, or sequencer or system register operand (universal
  registers are limited to the register file, the DAG registers and PX),
  and no undefined compute. A conditional branch or return can be inside:
  not taken it is an ordinary instruction; taken, the block ends before it
  and it runs as a single step.
- **Memory**: before each instruction runs, the addresses its DAGs will
  output are worked out (`dag_peek`, which `dag_output` itself uses). An
  instruction that would reach IOP registers or reserved addresses (side
  effects such as a reset or DMA) ends the block before it, and runs as a
  single step. A store that overwrites compiled code ends the block after
  it.
- **The board** says how many instructions it can let run before it must
  look (`Bus::block_budget`): the 2B geometrizer to its next check every
  256 instructions or the end of its FLAG0 pulse, the copro to its step
  limit. Its per-step work between those is replayed exactly:
  `Bus::after_access` runs after each instruction that used the external
  port, with `steps` counting it (the copro sets its FIFO flags there,
  the geometrizer notes list or rasteriser activity), and `spinning`
  can't hold after any instruction of a block but the last, since none is
  a taken branch and a block doesn't start inside a short loop that ends
  on a condition.

`make lockstep` checks it against the interpreter (both SHARCs' whole
state after every frame), and a unit test runs programs through blocks
of several sizes beside single steps (`blocks_match_the_interpreter`).

## Ambiguities in the manual, and what the core does

1. **Immediate-shift SHIFTOP (type 6, UM A-25).** A 6-bit field for 8-bit
   shifter opcodes, mapping not stated. Every opcode in Table B.6 has bits 1-0
   zero, so SHIFTOP is taken as opcode bits 7-2. Confirmed by the programs:
   SHIFTOP 0x33 (BTST, 0xCC) is always followed by `IF SZ`/`IF NOT SZ`, and
   0x01 (ASHIFT) appears with negative amounts on signed data.
2. **Type 18 SREG (UM A-47).** The diagram's field could be read as 5 bits;
   §A.3 says SREG is 0-15, so bits 35-32, with bit 36 unspecified.
3. **Multiplier overflow width.** Appendix B (B-47) says 33 upper bits for a
   signed fractional result, 32 unsigned; §2.6.5.3 says 17 and 16 (bits 79-63:
   the MR2 extension plus MR1's sign). The core follows §2.6.5.3, which matches
   the result placement of Figure 2.2; integer formats agree (49/48).
4. **MUS for fixed point.** §2.6.5.1 says fixed-point underflow sets MU and
   MUS; the summary Table 2.6.6 shows MUS unaffected by fixed-point
   operations. The core sets MU only (and MOS for fixed overflow), as the
   table says.
5. **Which products are shifted left.** §2.6.1: "if both inputs are fractional
   and signed"; Appendix C.5 says "if both inputs are signed". The core shifts
   signed x signed fractional products only (integer products stay in MR0).
6. **Rounding an MR value (§2.6.3.2):** to nearest at bit 32, ties to even
   (§2.4's definition), and MR0 is cleared; the manual doesn't say what MR0
   holds afterwards.
7. **Fixed ABS flags.** Table 2.5.4 shows AN cleared; B-15 sets AN from the
   result's MSB (which differs only for ABS of 0x80000000 without
   saturation). The core uses B-15; AC is cleared as §2.5.3.5 says.
8. **Shifter overflow.** B-55 says SV is set if the input is "shifted to the
   left by more than 0"; §2.7.3.2 says when "significant bits are shifted to
   the left of the 32-bit field". The core uses §2.7.3.2: SV when the shift
   loses bits.
9. **Shift amounts from a register.** B-55 says shifts use "the 32-bit value
   in register Ry", Figure 2.4 shows an 8-bit shf8 field. The core uses the
   whole 32-bit two's-complement value (anything beyond ±31 is off-scale
   either way). Bit positions for BSET/BCLR/BTGL/BTST are taken unsigned, so a
   negative position is out of range: no bit changes and SV is set, which
   satisfies both B-60's "greater than 31 or less than 0" and its "SV if
   greater than 31".
10. **RECIPS and RSQRTS seeds (B-39, B-40).** The mantissas come from ROM
    tables that the manual doesn't list. The core computes them: the
    reciprocal (or reciprocal square root) at the middle of each 7-bit
    index's interval, rounded to 8 (or 4) fraction bits, with the exponent
    rules, specials and flags exactly as described. Results are within the
    stated accuracy; after the manual's Newton-Raphson sequences they agree
    with exact division to the last bit or so, but not necessarily bit for bit
    with the chip. **Open question.**
11. **FPACK rounding** ("rounded upper 11 bits") is taken as round to nearest
    even; a carry out of the fraction increments the exponent.
12. **NaN results** ("all 1s") are 0xFF_FFFF_FFFF in the 40-bit register;
    MANT of a NaN or Infinity gives fixed-point -1 (0xFFFFFFFF, low 8 bits
    zero), as B-35 says.
13. **The timer's second interrupt.** §3.6.4 says the timer causes "interrupt
    4 and interrupt 14"; Table 3.3 and §E.8 put TMZLI at bit 23. The core
    latches bits 4 and 23.
14. **Booting with interrupts disabled.** §11.6.3 says the DMA completion
    vectors to 0x20040 after a host boot, while §3.6.5 says IRPTEN is clear
    at reset. The core takes EP0I from the reset idle state regardless of
    IRPTEN (only that interrupt, only until the first one is taken). IM6 is
    1 at reset ("increment by 1 is automatic").
15. **RTS (LR) (UM A-34, §3.5.1.1).** The manual says LR stops the counter
    being decremented twice when a non-delayed CALL in the last instructions
    of a loop returns. The core remembers which loop end last looped back;
    after RTS (LR) the next fetch of that end loops back without testing
    again. An interrupt that squashes such a fetch makes its RTI behave the
    same way.
16. **PUSH LOOP / PUSH PCSTK (UM A-50)** don't say what is pushed: LCNTR with
    an empty loop address, and zero, respectively (programs then write
    LADDR/CURLCNTR/PCSTK). POP of an empty stack does nothing.
17. **Stack overflow** sets the overflow flag and raises SOVFI on the chip,
    with the lost data unspecified; the core stops with `Fault::Stack`
    instead.
18. **Bus master condition (BM).** A single-processor model: true when MODE1
    CSEL is zero.
19. **A short word read into PX** is zero-filled (UM §5.3.4) and placed as
    any 32-bit value would be; where the 16 bits land in PX isn't stated.
20. **CJUMP** decodes only the two encodings UM A-54 shows.
21. **Unspecified opcode bits** are ignored, as UM §A.3 says (for instance
    bits 7-0 of an MR transfer).

## Model 2B: the upload, as observed

First seen in a throwaway boot of `overrevb` on the 2C board model with
logging of the i960's writes to the SHARC windows; now what the 2B board
model does (`docs/hw/model2b.md`). `sgt24h` uploads the same two programs'
code (the 4,096-word images differ past it).
The host windows map IOP register *n* at window + 4*n*: `0x8C0000` is the
copro SHARC's, `0x840000` the geometrizer's. Each 32-bit register is written as two
16-bit transfers to the same address, low half first: the reset SYSCON
(HPM=01, 16-bit host bus, HMSWF=0).

| i960 write | SHARC register | Value |
|---|---|---|
| `0x980000` (`0x980008` for the geometrizer) | board: bit 31 set | |
| window + `0x3C` (geometrizer only) | MSGR7 | `0x7378842E` |
| window + `0x70` | DMAC6 | 0 |
| window + `0x100` | II6 | `0x20000` |
| window + `0x104` | IM6 | 1 |
| window + `0x108` | C6 | `0x1000` |
| window + `0x70` | DMAC6 | `0xA1`: DEN, DTYPE (instructions), PMODE 10 (16 to 48), LSW first |
| `0x884000` / `0x804000`, 12,288 times | EPB0 | 4,096 48-bit words as 16-bit halves, low first |
| `0x980000` / `0x980008` | board: bit 31 clear | |

With the core's host interface this loads 0x20000-0x20FFF; the count
reaching zero raises EP0I, whose vector (0x20040) is `RTI`, returning to
0x20005: `JUMP 0x20080 (DB)`. Bit 31 of `0x980000`/`0x980008` can't be
RESET (UM §11.6.3 needs the SHARC out of reset, idling in host boot mode, to
take the host's writes, and the program runs once the bit clears): it
behaves as the host bus request (HBR); `docs/hw/model2b.md` has the
reasoning.

Captured images are 48-bit words, six little-endian bytes each, loaded at
0x20000 (`out/overrevb/sharc-copro.bin`, `sharc-geo.bin`): `make run
GAME=overrevb` writes them from the uploads it sees.

### What the programs show

`make sharc-test` on both images (`remodel sharc-check`):

| | copro | geometrizer |
|---|---|---|
| words reached as code (from the vectors, following jumps, calls, loops and the dispatch tables) | 2,111 | 2,987 |
| undefined encodings among them | 0 | 0 |
| undefined words elsewhere (data tables after or inside the code) | 57 | 684 |
| DO UNTIL loops / RTS / RTI | 28 / 91 / 28 | 32 / 2 / 28 |
| host boot, then run | 148 instructions to its first external read, `0x400000` | polls FLAG0; once it is raised, sets FLAG1 and writes `0x3F000000` to `0x430002` |

Discovery follows a delay slot on from where it is also a branch target
(the copro's `0x20811`, a slot of the `RTS (DB)` at `0x2080F` and the
target of `IF LT JUMP` at `0x20809`, falls through to the `RTS` at
`0x20812`); before that was fixed it found 2,110 words and 90 RTS.

- *Confirmed (from the programs):* both write SYSCON and WAIT through I0 in
  IOP space early on: the copro SYSCON = `0xA100` (IMDW0: block 0 holds
  40-bit data; MSIZE 10), WAIT = `0xC9400`; the geometrizer SYSCON = `0x3100`
  (IMDW0; MSIZE 3), WAIT = `0xB400`. Both clear HPM, so the host bus is
  32-bit and unpacked from then on. MODE1 = `0x18000` (copro: TRUNC, RND32)
  or `0x1A000` (geometrizer: also ALUSAT); MODE2 = `0x60000` (copro: FLAG2,
  FLAG3 outputs) or `0x70000` (geometrizer: FLAG1-3 outputs). FLAG0 (and on
  the copro FLAG1) are inputs the programs poll in handshakes with the host.
- *Confirmed:* each builds a table of routine addresses at DM `0x30000`
  (block 1; 80 entries in the copro, 32 in the geometrizer) and constant
  tables after it. The copro reads 32-bit command words from external
  `0x400000` once FLAG0 is low and dispatches on bits 30-23.
- *Confirmed:* the copro routine at `0x20593` has
  `LCNTR = R15, DO 0x2034f UNTIL LCE` at `0x2059d`, whose PC-relative end
  (-590) lands on the end of an identical loop earlier in the program
  (`0x20347`-`0x2034f`): apparently a copy-paste slip in the original source.
  On the chip such a loop never meets its end, so the RTS after it would pop
  the loop's top address (0x2059e) and run the tail again before returning,
  leaving a loop stack entry behind. Whether the game ever calls it is
  unknown.

## Open questions

- The RECIPS and RSQRTS seed ROM contents (item 10 above).
- The board side of 2B is in `docs/hw/model2b.md`, with its own open
  questions (what the output flags and some registers drive, FIFO depths,
  resets). Neither program enables interrupts (IRPTEN stays clear), so
  IRQ2-0 and the timer don't matter to them.
- Whether the copro's backwards DO is reached, and what the chip does with
  the leftover loop stack entry.
- Cycle timing beyond branch bubbles, if the board needs it.
- Speed: the interpreter takes about 54 ns an instruction on the 2B
  geometrizer program (a race frame is 300,000-500,000 instructions);
  recompiled, about 30 ns as single steps and 13 ns with blocks
  (`docs/recompiler.md`), most of what's left the instructions' own work
  (40-bit float, the memory columns).
