# The Sega settings records in the EEPROM

How several Model 2 games keep their operator settings in the 93C46
EEPROM (64 words, 128 bytes), as **the games' own code does it**: the
layout, the checksum, and what a game does with a record it does not
like. It is what `romset::LinkSettings` writes when linked play sets a
cabinet's role (`docs/hw/link-board.md`): a node's EEPROM image has to
carry a record the game accepts, with the role bytes in it and a
checksum that is right.

Everything here was read from the games' own i960 programs (the
routine that loads and saves the record, found from the record's magic
and from the constant of its checksum; disassembled with `remodel
disasm` on the user's ROMs) and then **confirmed against images the
games themselves wrote** (`make link-record-test`). Nothing was taken
from another emulator, and no image or listing is kept in the tree. The
algorithm is described in words; the numbers that are data (the start
values, the lengths) are in `crates/romset/src/games/link.rs`. Marked
*inferred* is what the code suggests but no run exercised.

## 1. Which games keep which kind

| Kind | Games | Notes |
|---|---|---|
| **S32A**, shifting checksum | `indy500`, `motoraid`, `waverunr` (start value A), `overrev`, `overrevb`, `skisuprg` (start value B) | section 2 |
| **S32A**, byte-sum checksum | `sgt24h` | section 3 |
| CRC-16 header (`srallyc`'s kind) | `srallyc`, `manxtt`, `manxttc` | `docs/hw/link-board.md` section 2; CCITT, start `0xFFFF`, complemented, over bytes 2 to the record's length |
| `SEGA` I/O board record | `daytona` | `docs/hw/link-board.md` section 10 (not in the EEPROM) |
| `SEGASTCb` | `stcc` | checksum not read yet |

## 2. The S32A record, shifting checksum

Layout of the 128-byte image (all numbers little-endian):

| Offset | Size | Contents |
|---|---|---|
| `0x00` | 4 | the magic "S32A" |
| `0x04` | 2 | a format word, one constant per game (the game compares it with its own; a record with another word is rebuilt from the defaults, keeping the write counter) |
| `0x06` | 2 | zero |
| `0x08` | R | **record A**: the checksum word (2 bytes), then bytes that the checksum covers: a zero word, the **write counter** (4 bytes, at `0x0C`), then the settings from `0x10` |
| `0x08 + R` | R | **record B**, a copy of record A, checksum word included |
| after | | `0xFF` |

R is the record's size: 36 bytes in `indy500`, `motoraid` and `waverunr`
(B at `0x2C`), 52 in `overrev` and `overrevb` (B at `0x3C`), 40 in
`skisuprg` (B at `0x30`). The settings are the game's menu values, one
or two bytes each; `indy500`'s NETWORK TYPE and CABINET ID are bytes
`0x1E` and `0x1F`.

### The checksum

The checksum word is what a small routine returns for a count of bytes
starting two bytes after the word. In words:

1. A 32-bit register starts at a constant: `0xDEBDEC00` in `indy500`,
   `motoraid` and `waverunr`; `0xDEBDEB00` in `overrev`, `overrevb` and
   `skisuprg`.
2. For each data byte: the register's low byte is replaced by the byte
   (the register's low 8 bits are zero at that point, so the byte is
   added to it); then, eight times, the register is shifted left one bit,
   and **if the bit that left was a 1, the constant `0x10210000` is added
   to the register** (an addition, not an exclusive or, which is why the
   result is no CRC-16 with the polynomial `0x1021` of any start value,
   although the constant is that polynomial's).
3. The checksum is the register's top 16 bits.

The data bytes are the ones after the checksum word, as many as the
game passes: 34 in the 36-byte record's games, 50 for `overrev` and
`overrevb`, 39 for `skisuprg` (each time the record's own length minus
two, or for `skisuprg` minus one; what the byte count does at the end is
that **the last byte fed never changes the result**: the byte is added at
the bottom of the register and the eight shifts that follow carry it only
to bits 8-15, below the 16 that are returned, so `skisuprg`'s count
reaches one byte past the end of its record without harm). Both copies
give the same value, the data being the same.

*Confirmed*: the stored checksum of both copies of an image written by
the game itself, for `indy500`, `motoraid`, `waverunr`, `overrev`,
`overrevb` and `skisuprg`, at the lengths and start values above, and
nothing else matched over every start offset and length tried.

### What the game does at start and at each save (`indy500`, read from its code)

- **Start.** It reads all 40 words into RAM, computes the checksum of
  each copy, and looks at the magic and the format word. A wrong magic
  (or neither copy good) starts the defaults with the write counter set
  to 100. With the right format word: if record A's stored checksum is
  right, the game uses it, and if record B's stored checksum is not the
  same word it saves again (to repair B); if only B is right, B is copied
  over A and saved; if neither is right, the defaults. With another
  format word the game rebuilds the record from its defaults but keeps
  the counter of whichever copy was good. *Inferred*: only the good path
  and the first-boot path were run.
- **Save.** The game adds one to the counter, computes the checksum over
  the data, copies record A to record B, and writes the 80 bytes
  (40 words) to the EEPROM. The first save on a fresh EEPROM writes
  counter 101, as every game's fresh image shows. The test menu saves
  when it is left (`docs/hw/link-board.md` section 14.4).

So a node's image for linked play needs: the magic and format word, the
defaults, the role bytes **in both copies**, and one checksum word
(equal in both). The write counter can be left as the game's first save
leaves it: the game raises it by itself at its next save. `romset::
RecordCrc::s32a` describes a game's record that way, and `fix` rewrites
record B from record A after the checksum.

## 3. The S32A record, byte sum (`sgt24h`)

Same header (magic, a format word, zero) and two copies, but the record
is 32 bytes (copies at `0x08` and `0x28`) and the checksum word is the
plain sum, modulo 65,536, of the 30 bytes after it. The first copy carries a
marker word `0xAD85` (bytes `85 AD`) at `0x10`, which the game tests at start
(a record without it is rebuilt from the defaults); the second copy has zeros
there and its own sum, so the copies are *not* identical here. *Confirmed*
against an image the game wrote (both sums). *Not yet built*: `sgt24h` needs the
`0x380` block layout as well (`docs/hw/link-board.md` section 14.6), and `RecordCrc`
would need the second copy to be built (marker out) rather than copied.

## 4. Not covered

- `skisuprg`: the checksum is known (section 2), but its start is a
  different handshake (`docs/hw/link-board.md` section 14.2).
- `stcc`: `SEGASTCb` record, checksum word at `0x08`, counter at `0x0C`,
  LINK TYPE in the top three bits of byte `0x10`: the routine has not been
  read.
- `von` keeps the setting in its backup SRAM (a third store).

## 5. Checking it

`remodel --game G link-settings FILE` (and `make link-record-test`) checks
the checksums of an image a game wrote, from a `run`'s `eeprom.bin` or
a `link-run`'s `eeprom-N.bin`, against the game's `LinkPlay` record, and
prints the role bytes and the write counter. The test runs each game from a
fresh EEPROM through its test menu (so that the game itself saves a
record), and again from a node image `LinkSettings::image` made, to check
that the game accepts it (its own save, with the counter one higher,
passes the check too).
