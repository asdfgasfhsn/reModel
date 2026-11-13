# MultiPCM sample chip (`crates/sound/src/multipcm.rs`)

Sega's MultiPCM (315-5560) is the sample player of the original Model 2
sound board (`docs/hw/model2.md` §5): two of them, each with its own sample
ROMs. There is no public manual. Everything here was worked out from
`daytona`'s sound program (`epr-16720.7`) and from the sample ROMs' own
contents, and checked by running the model on the game's sound link
(`make sound GAME=daytona`). `vcop`'s and `desert`'s sample ROMs start with
header tables of the same format (checked). Where the program leaves a
behaviour open, the model borrows the published figures of Yamaha's
YMF278B (OPL4), whose wave-table part takes the same 12-byte sample
headers; those are marked *assumed*.

Addresses below are 68000 addresses in `daytona`'s program.

## Interface

Three byte ports, at `+1`, `+3` and `+5` of the chip's base (`0xC40000`,
`0xC60000`) (*confirmed* from the write routines `0x74C` and `0x77A`):

| port | write | read |
|---|---|---|
| `+1` | data: the selected slot's selected register | status: bit 0 busy |
| `+3` | slot select | |
| `+5` | register select (0-7) | |

- The program waits for bit 0 to clear before each port write. The model
  takes writes at once and never reports busy. *Unconfirmed* how long the
  chip is busy for.
- **Slots** (*confirmed*): the reset loop (`0x34A`-`0x392`) walks a table
  of 28 slot numbers at `0x822`: `00-06, 08-0E, 10-16, 18-1E`. Every eighth
  number (7, 15, 23, 31) is skipped: 28 voices.
- At reset the program writes `3` to `+0x13` (`0x34A`) before anything
  else. *Unknown* what it sets; the model records it and ignores it.
- **Bank** (*confirmed* from the data, below): a byte at `base + 0x10000`
  (`0xC50001`, `0xC70001`), set at reset and per tune from its table at
  `0x50A2` (values 0-3), and by a sequence command (`0x14C8`).

## Registers

The key-on routine (`0xC22`-`0xD26`) writes, in order: 3, 2, 1, 5, 0, 4.

| reg | use | evidence |
|---|---|---|
| 0 | **pan**, high nibble, signed | `0x10B2`-`0x1144`: MIDI pan `v` becomes `v >> 3` with bit 3 flipped, so 64 (centre) is 0, 127 is 7, 63 is -1 and 0-15 are -7; the reset value is `0x80` (-8) |
| 1 | **sample number** (0-255): its header in the table at ROM 0 | `0xCE0`; from a per-instrument note-range table at `0x5E16` |
| 2 | F-number bits 5-0 in bits 7-2 | `0xCCE`: the low byte of a 16-bit pitch value whose low two bits are always 0 |
| 3 | **octave** (signed high nibble), F-number bits 9-6 | `0xCC4` |
| 4 | bit 7: **key on**, 0: key off | `0xD1E` (`0x80`); key-offs at `0x1552`, `0x15D2` |
| 5 | **total level** (attenuation) in bits 7-1; bit 0 set: apply at once | key-on writes `~level | 1` (`0xCFE`); a volume change on a sounding voice `~level & 0xFE` (`0x12D6`); reset `0xFE` |
| 6 | LFO: frequency in bits 5-3, vibrato depth in bits 2-0 | `0x11C4`: the modulation wheel (`>> 4`, 0-7) ORed with a per-channel value |
| 7 | tremolo depth | written only from the header (below) in `daytona` |

- **Pitch** (*confirmed* from the tables): the program turns a note into
  an octave and semitone with the table at `0x5C62` (note 84 is octave 0,
  semitone 0), then a semitone and fine offset into the 16-bit value with
  the tables at `0x5282`/`0x5432`: 16 entries a semitone, the low 12 bits
  rising from `0x000` to `0xFFC` over an octave and carrying into the
  octave nibble, the steps growing from 4 to about 8 F-number units across
  it. So the rate is linear in F within an octave: **`2^octave * (1 +
  F / 1024)`** times the base rate (semitone 1 is F = 61, `2^(1/12) =
  1.0595`).
- **Base rate**: *inferred* 44.1 kHz. A sample played at octave 0, F = 0
  steps one sample per base-rate sample; the instrument tables give each
  multisample's root note (84 minus its transpose), and the loops of
  sustained samples with several periods in the loop put the base rate at
  44,040-44,200 Hz for the four clearest. The model uses the output rate,
  44,100 Hz.
- **Level glide**: with bit 0 of register 5 clear, the level is taken to
  glide to the new value rather than jump (the program clears it only for
  changes to sounding voices, which would otherwise click). *Assumed*
  (OPL4's "level direct" bit); the model steps one level unit every 16
  samples (*unconfirmed* rate).
- **Level**: *assumed* 0.375 dB a step (OPL4), so `0x7F` is -47.6 dB.
- **Pan**: *assumed* 3 dB a step off the far side, 7 silencing it, -8
  silencing both (OPL4's curve); positive is right (*confirmed*: MIDI 127
  gives 7).
- **LFO**: *assumed* OPL4's frequencies (0.17 to 7.07 Hz), vibrato depths
  (up to 79 cents) and tremolo depths (up to 11.9 dB), as triangles.

## Sample ROM

Each chip reads its own ROMs as one byte-wide space: the first chip
`mpr-16491.32` then `mpr-16492.33`, the second `mpr-16493.4` then
`mpr-16494.5` (*confirmed*: each pair's first chip starts with a header
table; the second chips are sample data). Samples are **8-bit signed PCM**
(*confirmed* from the data: every header's format bits are 0).

### Headers

The table at ROM 0 holds 12-byte headers, one per sample number
(*confirmed* from the data; `daytona` fills about 215 of each chip's, the
rest are `FF`). Entry 0 describes the table itself (start 0, length
`0x1FF0`); the first sample starts at `0x2000`.

| bytes | field |
|---|---|
| 0-2 | start address: 22 bits; the top two bits (the format) are 0 |
| 3-4 | loop offset from the start |
| 5-6 | end offset, **inverted**: `~(last sample's offset)` |
| 7 | LFO frequency and vibrato depth (register 6) |
| 8 | attack rate (high nibble), decay 1 rate (low) |
| 9 | decay level (high), decay 2 rate (low) |
| 10 | rate scaling (high), release rate (low) |
| 11 | tremolo depth (register 7) |

- *Confirmed*: starts and lengths: within a bank the samples follow each
  other (each ends 16 bytes or so before the next starts), and one-shot
  samples loop on their last sample (loop = end).
- *Inferred*: the meaning of bytes 7-11 (OPL4's layout). In `daytona`
  bytes 7 and 11 are always 0; bytes 8-10 vary (`F0 00 0F`, `F0 00 08`,
  `F3 FF 05`, ...), as attack-decay-release settings would: attack 15
  everywhere, release rates 5-15.
- The model loads the header at key-on (start, loop, end, envelope, and
  registers 6 and 7), and starts the sample from its first byte.

### Banking

*Confirmed* from the data: each chip's 2 MB address space is a fixed
first MB (the first MB of its ROMs) and a **banked second MB**. The
headers with starts at `0x100000` and up form three runs, each starting at
`0x100000`; matching each run's loop points against the ROM's four MBs
(the sample before a loop's end should continue into the loop start) puts
run 1 in the second MB, run 2 in the third, run 3 in the fourth, for both
chips (the mean jumps at the loops, with the samples' edges, drop from
35-80 to 2-16 at the right MB). So the bank register selects which MB of the ROMs `0x100000-0x1FFFFF`
reads, and the tunes' bank bytes (1, 2 and 3; one tune has 0) select
their instruments' run. The program agrees: in a race and an attract
run every banked sample was keyed with its run's bank selected (both
chips; `REMODEL_SOUND_TRACE=1 make sound` lists the key-ons with the
bank).

**Sockets** (*confirmed* from `vcop`): each MultiPCM's ROMs sit in 2 MB
sockets, two a chip, and a smaller chip repeats through its socket.
`vcop`'s first chip has two 1 MB ROMs, `mpr-17172.32` (the header table)
and `mpr-17173.33`; its banked samples (headers 52 on, from `0x100000`)
are in `.33` (their loops fit it, a mean jump of 1.7 against 8.8 in
`.32`) and the sound program selects them with **bank 2**, the MB where the
second socket starts. Packed back to back, bank 2 read past the end and
those samples (one of the most used) were silent. `RomSet::multipcm`
assembles the sockets; `daytona`'s 2 MB chips fill theirs, so nothing
changes for it.

## Envelope (*assumed*)

Attack, decay 1 to the decay level, decay 2, and release on key-off, from
the header's 4-bit rates. The effective rate is 4 x rate plus a key
scaling of `(octave + scaling) * 2 + F-number bit 9`, 0-15 (none at
scaling 15), at most 63; rate 0 holds. 96 dB of decay takes 39.3 s at
effective rate 4, halving every four rates (2.4 ms at 60); attack is an
exponential approach taking 2.8 s at rate 4, instant from 60. The decay
level is 3 dB a step. These are OPL4's figures; nothing in the program
pins them down. A voice stops when its envelope reaches -96 dB.

## The model

`sound::multipcm::MultiPcm`, one output sample at a time at 44.1 kHz: the
sample position steps by the pitch, linearly interpolated, through the
loop; the envelope, the level, the LFO and the pan scale it; the 28 voices
are summed at 16-bit scale. Save states keep each voice's registers and
playback state and the chip's selects and bank.
