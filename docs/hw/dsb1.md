# DSB1 music board (`crates/dsb/src/dsb1.rs`)

The DSB1 ("Digital Sound Board") is `stcc`'s music board: a Z80 that takes
the host's sound link bytes on a UART and drives an MPEG audio decoder
reading MPEG-1 Layer II frames from its own ROMs. It sits where `topskatr`'s
DSB2 sits (`docs/hw/sound.md` §3): on the same link as the sound board, with
no reply path to the host.

Everything here was worked out from `stcc`'s Z80 program (`epr-19275.2s`,
"MPEG SOUND DRIVER Z80 VERSION 0.26 - 96/08/29") and its MPEG ROMs
(`.57s`-`.60s`), and checked by running the model against a recorded game
session. Addresses are Z80 addresses in that program. *Unconfirmed* marks
what the program leaves open.

## Memory map

| range | device | evidence |
|---|---|---|
| `0000-7FFF` | program ROM | reset at `0000`, code up to about `0600`, data at `1000` |
| `8000-FFFF` | 32 KB RAM | the boot test (`0085`-`009C`) writes and checks `55`/`AA` over all 32 KB, then clears it |

- The chip is 128 KB but only its first 8 KB hold anything: code in
  `0000-0FFF`, the track data in `1000-1FFF`. The rest is `FF`. No port the
  program writes looks like a bank select, so the model maps the first
  32 KB and no banking. *Unconfirmed* whether the upper 96 KB are
  reachable at all.
- At boot (`00A7`) the program copies ROM `1000-6FFF` to RAM `A000-FFFF`
  and reads the track data from RAM, so a host command can patch it (the
  `AF 60`-`63` commands below do).
- RAM use: `8000-8FFF` receive ring, `9000`/`9002` ring read/write
  pointers, `9004`-`9026` player state, stack from `FFFE`.

## UART

Ports `F0` (data) and `F1` (control/status), programmed like an 8251 at
boot (`00DA`-`00F5`): three zero writes, reset `40`, mode `4E` (asynchronous,
x16 clock, 8 bits, no parity, one stop bit), command `37`; `37` again just
before the main loop (`0134`).

- **Receive interrupt**: interrupt mode 1 (`0068`), handler at `0038`. It
  reads the byte from `F0`, waits for status bit 0 (TxRDY), echoes the byte
  out of `F0`, appends it to the ring at `8000-8FFF` (write pointer `9002`)
  and returns with `EI; RET`. Nothing else enables or acknowledges an
  interrupt, so INT is the receiver-ready line, cleared by reading the data
  port. The model raises INT while a received byte is unread, with status
  `05 | RxRDY << 1` (TxRDY and TxEMPTY always set), the DSB2's layout.
- The echo goes nowhere in `stcc` (the host never reads a reply); the model
  counts it.
- Bytes arrive at 31.25 kbaud (the link rate). *Unconfirmed*: the Z80 clock.
  The model uses 4 MHz, which fits the x16 mode if the UART is clocked at
  the CPU clock divided by 8; the program has no timing that pins it down
  except the fade speeds (below).

## Decoder interface

The decoder is driven through ports `E0`-`EB`. Every MPEG frame in the four
ROMs is MPEG-1 Layer II, 32 kHz, 128 kbit/s, stereo, the same format as the
DSB2's, and the program drives it the same way (start, end, a continue
mode, position read back), so the model reuses the DSB2's decoder
(`MpegChip` and `crates/mpeg`) behind these ports. The chip itself is
reportedly different (`docs/hw/sound.md` §3.1; not checked). The MPEG ROMs
form one 8 MB space, `.57s` first: the segment addresses span `000000` to
`7CF8CD`, each starting on a frame header.

| port | direction | use | evidence |
|---|---|---|---|
| `E0` | write | play mode: `0` stop, `2` play and continue, `1` play to the end and stop | `0188`, `0227`, `0498`, `0570` |
| `E2`-`E4` | write | start address, 24-bit, MSB first | `OUTI` loops at `01F6`, `045C` |
| `E5`-`E7` | write | end address: the segment's **last byte** | same loops |
| `E2`-`E4` | read | the decoder's current address | `0393`-`039B` |
| `E8` | write | attenuation: `00` loudest, `7F` silent | fades at `04FB`-`0570`, every main-loop pass at `057C` |
| `E9` | write | one value per track (`0`, `1` or `2`), `0` at boot | `01E1`, `00C3` |
| `EA` | write | LEDs | the `RST 08` routine at `0008`, the boot pattern at `0114` |
| `EB` | write | `0A` once at boot | `00CA` |

- **End is inclusive** (*confirmed from the data*): for every segment in the
  table, walking 576-byte frames from the start ends exactly at end + 1. The
  model passes end + 1 to the decoder.
- **Continue mode** (*inferred from the program*): the program writes a
  segment's start and end, then `E0 = 2`, and straight away writes the
  *next* segment's start and end while the first still plays. It then
  polls the current address: once it is past the middle of the playing
  segment (`039C`-`03CA`, midpoint kept at `9011`), and then inside the
  next segment's first half (`03D7`-`0414`), it queues the one after.
  So in mode 2 the decoder, at the end of a segment, carries on at the
  start and end written meanwhile; mode 1 plays the current segment out and
  stops (written when the list ends, after the last segment has started).
  This is the DSB2 decoder's loop behaviour (`docs/hw/sound.md` §3.4), and
  the model uses it as is.
- *Unconfirmed*: what `E0 = 1` does while stopped (the program never does
  it; the model starts playing), and whether a start address takes effect
  as its low byte is written (the model's choice; the program always writes
  all three bytes, MSB first).
- **Attenuation** (*partly unconfirmed*): a fade-in starts at `7F` and
  steps down to the track's volume, a fade-out steps up to `7F` and then
  writes `E0 = 0`, and a new track is muted with `7F` before the decoder
  stops. So smaller is louder and `7F` is silent. The curve is not known:
  the model uses a linear gain, `(7F - v) / 7F`. The track volumes
  (`10`-`41`) don't follow the tracks' loudness (all are mastered within a
  few dB of each other), so they don't calibrate it.
- Values with bit 7 set go to `E8` only at boot (`80`, then `C5`), and `E9`,
  `EB` have no visible effect on the data; the model records them and
  ignores them. *Unconfirmed* what they set (output format, de-emphasis,
  channel routing are all candidates).
- The current address read back is the address of the next frame the
  decoder will read; the program only compares it against segment bounds.

## Host commands

The main loop (`013D`) takes bytes from the ring. Status bytes (bit 7 set)
start a message; only `AF` (channel 15) messages are acted on, others are
skipped to the next status byte (`037D`). Running status applies.

| bytes | meaning | routine |
|---|---|---|
| `AF 6F n` | play track `n` (1 to the count in the header); `0` stops | `016A` |
| `AF 01 v` | volume scale for later volumes: attenuation = min(`7F`, volume × v / 64) | `029E`, `05CA` |
| `AF 02 v` | volume now (scaled) | `025B` |
| `AF 03 r` | fade out at rate r, then stop | `027A` |
| `AF 60`-`63` ... | edit the RAM segment table: select a segment, set its start or end, override a start until the next track | `02B0`, table at `02D8` |

`stcc` sends `AF 01 50` at boot, `AF 6F n` for each tune and `AF 03 rr`
to fade out (seen in a coin-start run: `20`, `40`).

## Track data

The header at ROM `1000` (RAM `A000`): a pointer to the track entries
(`A000`), a pointer to the segment table (`A002`), the track count + 1
(`A004`, 18 in `stcc`) and the default volume scale (`A005`).

- **Segment**: 6 bytes, start and end (inclusive), 24-bit MSB first.
- **Track entry**: a 16-bit fade-in rate (zero: start at full volume), a
  volume, the `E9` value, then a list: a byte below `F0` is a segment
  number; `F0` ends the track (`E0 = 1`); `F1 lo hi` jumps to another list
  address (how tracks loop); `F2` + 4 bytes sets a delayed fade-out; `F3 v`
  sets the volume. Handlers at `0436`-`04D6`.
- `stcc` has 17 tracks over 23 segments; most are one segment looped with
  `F1`, some an intro segment then a looped one, one (`16`) a sequence of
  segments, and one (`17`) plays once.

Fades step the attenuation once per main-loop pass when a 16-bit
accumulator overflows, so their speed depends on the Z80 clock
(*unconfirmed*, see above). At 4 MHz `AF 03 20` fades a track out in about
a second.

## The model

`dsb::Dsb1` runs the Z80 (`crates/z80`) on a cycle budget, one output
sample at a time like `Dsb2`: link bytes are queued and reach the UART at
the line rate, the decoder's 32 kHz output is resampled to 44.1 kHz and
scaled by the attenuation. `model2::audio::Live` creates it when the
manifest has a `Dsb1Cpu` chip, in the DSB2's place: same mix, same
`--mix` music gain, same save-state slot. `remodel music-render` (`make
music GAME=stcc`) replays a `make run` session's link bytes into it and
prints the host commands, decoder events and per-second levels.

A replacement soundtrack (`docs/mods.md`) replaces a track by the range of
its first segment (`Dsb1::track_table`). The replacement plays in place of
the whole track: the decoder's address then stays at the segment start, so
the program never queues the next segment, and the file loops while the
play mode is "continue" (or as its `tracks.txt` says).

Checked: in a coin-start session the board plays track 1 and then track 11
from `AF 6F` commands, chains and loops segments, and fades out on
`AF 03`. The rendered track lines up with the same frames decoded straight
from the ROM (correlation 0.995 after resampling) at the gain the
attenuation gives.
