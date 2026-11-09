# What is known about the Model 2C board (findings)

## What's known about the Model 2C board

From the five games' own code and test menus; it holds for every 2C set
supported unless a game's section says otherwise.

- *Confirmed (from the code, then by running it):* the memory test in
  `segawski`, `skisuprg` and `waverunr` checks two more RAMs on the video
  side, IC55 at `0x10A00000` and IC54 at `0x10C00000`, 64 KB each. Its
  table entry gives each one byte-wide, 64 KB long, compared through mask
  `0x0F` (a 4-bit-wide RAM, presumably 64K x 4), next to IC81, the luma
  RAM at `0x11400000`, tested the same way. The test sets render mode bit
  0 (`0x10000000` = `0x4005`, host access) first, then for 16 rotations of
  `0x1234ABCD` saves each location, writes the pattern, reads it back and
  restores it. reModel left both unmapped (reads 0), so the three games
  reported them BAD; they are now plain RAM (`bus.rs`). No game reads or
  writes them outside the test in a coin-start run, so their function is
  *unconfirmed*, as are mirrors past 64 KB and the upper four bits of each
  byte. `topskatr`'s and `stcc`'s memory tests don't list them.
- *Confirmed (from `bel`'s code and memory test):* ROM banks don't always
  give each chip pair an 8 MB slot. `bel`'s main data (four pairs of
  16 Mbit chips) sits every 4 MB, `.11/.12`, `.9/.10`, `.7/.8`, `.5/.6`
  from `0x2000000`, and so do its four polygon pairs (model addresses run
  to `0xBFFFFF`, 4M words), while its two texture pairs still take 8 MB
  slots. The manifest lists the packed banks (`Game::packed`). Why the
  banks differ (jumpers, the ROM board revision) is *unconfirmed*.
- *Confirmed (from `bel`'s code):* the copro status `0x980014` reads with
  bit 0 set before any program is uploaded, not only once an upload has
  started: `bel` uploads its copro program at boot only if bit 0 of
  `0x980010` or `0x980014` is set, and otherwise sends commands to a copro
  with no program. The board model now reads all ones whenever the copro
  holds no program. The other games start an upload unconditionally.
- *Confirmed (from `bel`'s code):* the sound UART raises I/O request bit
  10 (IRQ3) while its transmitter can take a byte, and a game may send
  its sound commands from that interrupt (`docs/hw/peripherals.md`,
  `docs/hw/sound.md` §7).
- *Unconfirmed:* `stcc`'s memory test reports IC29 and IC30 BAD; those are
  its copro data ROMs (`.29/.30`), which the i960 can only reach through
  the TGP. Not investigated.
- *Confirmed (from the programs, the games' I/O and by running them):*
  the geometrizer is render-only, as on 2B. Its programs write only the
  rasteriser FIFO, polygon RAM and status bits (PDR), and the i960 reads
  none of it (only its own list pointers and the frame parity); with
  `--geo native` (the TGP runs no frame) 600M-instruction coin-start runs
  of all eight 2C games end with the TGP's state hashes.
- *Confirmed (from the disassemblies, then by running them side by
  side):* the eight 2C games upload four geometrizer programs of one
  family: one shared by `skisuprg`, `segawski`, `waverunr`, `hotd` and
  `bel`; `stcc`'s, the same but for one status-port write; `overrev`'s,
  which lights quads and triangles from the diffuse term clamped at 0,
  takes the specular term from that clamped value, counts a quad with a
  zero dot product as a back face, and logs each command's fn in B RAM;
  and `topskatr`'s, the shared maths with its state elsewhere in memory
  plus the skinned compact model format and a second matrix. All check
  every command word (fn in bits 4-0 and 12-8 too) and stop for good on
  a bad one, an unknown fn, or a polygon RAM write or data push out of
  range. The specular control indexes a table of squarings that runs on
  into other state (a control of 15 squares 16,383 times in the shared
  program); the games' controls stay inside it.
  `docs/hw/geometrizer-2c.md` has the command set, formats and maths, with
  the native implementation that reproduces the stream and state word for
  word.
- *Unconfirmed:* the TGP's NaN results. reModel defines them (the first
  NaN operand quieted, else `0xFFC00000`, `docs/hw/mb86235.md`) so that
  every build agrees; the 2C games' state hashes are the same with and
  without the rule.

