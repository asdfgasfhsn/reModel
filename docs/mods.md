# Mods

Changes to how a game looks or sounds that the original hardware could not
make. The game's own code keeps running unchanged; Remodel swaps what it
produces at a well-defined point in the hardware model.

## Custom soundtracks

You can replace any of the game's music tracks with your own audio. The game
still decides when a track starts, stops and loops, how loud it plays and when
it fades. Remodel swaps the audio at the MPEG decoder: when the DSB2 firmware
tells the decoder to play a range of MPEG ROM that belongs to a replaced
track, the decoder streams your file and leaves the ROM alone. Tracks you
don't replace play the original music, bit for bit.

This works for any game with a DSB2 music board. Tracks are matched by number
through the firmware's track table, or by an explicit ROM range (see
`range` below).

### Quick start

```sh
# 1. Put your files in a folder. A leading number in a file name picks the
#    track ("04 - My attract theme.flac" replaces track 4).
# 2. Import: WAV and MP2 files are copied; anything else (mp3, flac, ogg,
#    m4a, ...) is converted to WAV with ffmpeg, which must be on PATH.
make soundtrack-import SRC=~/music/my-mix SOUNDTRACK=my-mix
# 3. Edit soundtracks/my-mix/tracks.txt (loop points, gain) if you like.
# 4. Render the music of a captured run with it:
make music SOUNDTRACK=my-mix          # -> out/topskatr/music-my-mix.wav
```

Or call the CLI directly:
`remodel music-render --soundtrack soundtracks/my-mix [WAV] [SECONDS]`.
`soundtracks/` is git-ignored. `SOUNDTRACK` that contains a `/` is taken as
a path, for example `SOUNDTRACK=out/soundtracks/demo`.

The render lists each replaced track with its file, format, rate, length and
settings, and which tracks stay original. Its event log marks every play
that was replaced (`-> replaced: soundtrack track 5`) and every loop or end
of a replacement file.

### `tracks.txt`

A plain text file in the soundtrack directory:

```ini
# Lines starting with # or ; are comments.
title = My mix

# Shorthand: just a file.
track 4 = 04 - attract.wav

# Full form.
[track 5]
file = menu theme.mp2      # relative to this directory
loop_start = 0:12.5        # where the loop goes back to
loop_end = 1:45            # where it jumps back from
gain = -3 dB               # or a factor: gain = 0.7
loop = game                # game (default), on or off

[track 12]
file = ending.wav
range = 0xCC7708-0xEC8948  # ROM range to replace, if not from the track table
```

Only full-line comments are allowed, so file names may contain `#`. Spaces
in values are kept except at either end.

| Key | Meaning |
|---|---|
| `title` | Optional name, shown in diagnostics. Outside any section. |
| `track N = FILE` | Shorthand for a `[track N]` section with only `file`. |
| `[track N]` | Starts the entry for track N (1-based, as in the game's `AE 10 nn` command). |
| `file` | The audio file, relative to the soundtrack directory. Required. |
| `loop_start`, `loop_end` | The loop region. Times are seconds (`12.5`, `12.5s`), `m:ss.s` or `h:mm:ss`. Sample counts are written `551250 samples` (or `551250 smp`), at the file's own rate. Default: the first loop of the WAV's `smpl` chunk if it has one, else the whole file. A missing end means the end of the file. |
| `loop` | `game`: loop when the game arms looping for the track, else play once. `on`: always loop. `off`: never loop. |
| `gain` | Extra gain on top of the game's volume: a factor (`0.5`) or decibels (`-6 dB`). Default 1. |
| `range` | `START-END`: the MPEG ROM byte range this track replaces. Normally read from the firmware's track table. Needed only when the firmware's table isn't recognised, or to replace a range that isn't a table entry. |

### Audio formats

Read natively, with no external tools:

- **WAV**: integer PCM at 8, 16, 24 or 32 bits, float at 32 or 64 bits,
  plain or `WAVE_FORMAT_EXTENSIBLE`. Any sample rate. Mono plays on both
  sides; with more than two channels, the first two are used.
- **MP2**: a raw MPEG-1 Layer II stream at 32, 44.1 or 48 kHz, decoded with
  Remodel's own decoder (the one that plays the game's ROM). A leading ID3
  tag is skipped.

`soundtrack-import` converts everything else with `ffmpeg` (`-ac 2 -c:a
pcm_s16le`, keeping the sample rate). Format conversion stays outside the
emulator core. Note that ffmpeg turns mono into stereo at -3 dB per side.

Every file is resampled to the 44.1 kHz output on the fly with a windowed-sinc
filter, which avoids aliasing when downsampling. A 44.1 kHz file is passed
through untouched. The game's own music goes 32 kHz → 44.1 kHz as before.

### How the game controls a replacement

The firmware drives the decoder with a few byte commands
(`docs/hw/sound.md` §3.4); a replacement obeys all of them:

- **Play** (`74`) of a replaced range starts the file from the beginning.
  The file plays in place of the ROM range the firmware asked for.
- **Stop** (`84`) stops it. Tracks change by stopping and playing again.
- **Loop arming** (`64`): with `loop = game`, a replacement loops only when
  the game armed looping for that play. In topskatr, tracks 1-3, 5, 7-9
  and 12 loop and tracks 4, 6, 10 and 11 play once. A looping replacement
  runs from 0 to `loop_end`, then repeats `loop_start`-`loop_end`. The
  resampler reads across the seam, so a loop cut at matching points is
  seamless. The firmware's own loop address is not followed: tracks 2-3 and
  8-9 of topskatr loop back into tracks 1 and 7, but a replacement always
  loops within its own file.
- **Gain** (`A0`, `A1`, `A4`, `A5`): the replacement goes through the same
  gain matrix as the ROM audio, so master volume (`AE 02`) and fade-outs
  (`AE 03`, stepped every 80 ticks as on the hardware) apply unchanged. In
  topskatr's capture the gain register runs from `0x74` (attract, about
  -7 dB) to `0xF3` (race, about -0.4 dB). The original MPEG audio peaks near
  full scale, so a file mastered at a normal level sits at the game's level;
  use `gain` to adjust.
- A one-shot file shorter than the original ends early and leaves silence
  until the game's next command. A longer one is cut off when the game stops
  it. The firmware never learns when the decoder finishes (its status port
  never reports busy), so neither changes the game's behaviour.

Matching: a play is replaced when its start and end addresses equal a
replaced track's range. Failing that, the start address alone is matched. On
topskatr every track starts at a different address. Remodel reads the track
table from the firmware layout found in topskatr's DSB2 program (pointer at
`0x8000`, count at `0x800A`, start and end at entry offsets 2 and 6). Other
DSB2 firmware is untested. If its table isn't recognised, the render says
`firmware track table: 0 tracks`; give each entry a `range` instead.

### Limitations

- A replacement is decoded into memory in full when the game first plays it
  (about 21 MB per minute). The playing track and the one before stay
  cached. That's fine offline; a live player will want to load in the
  background.
- MP2 means MPEG-1 Layer II only (no MPEG-2 half-rate streams).
- Fades step exactly as the firmware writes the gain registers, about 80 ms
  apart, with no smoothing (as with the ROM audio).
- Only player 0 of the decoder is modelled. topskatr never uses player 1.

### Trying it without your own music

With ffmpeg you can make a test soundtrack from tones and check by ear or by
analysis that the game's track changes switch files at the right moments.
This is how Remodel was verified on topskatr's attract-into-race capture:

```sh
mkdir -p out/st-src
ffmpeg -f lavfi -i "sine=f=440:r=48000:d=30" -c:a flac "out/st-src/04 attract.flac"
ffmpeg -f lavfi -i "aevalsrc=0.5*sin(2*PI*if(lt(t\,1)\,300\,if(lt(t\,3)\,500\,if(lt(t\,5)\,700\,900)))*t):s=48000:d=6" \
       -ac 2 -c:a mp2 -b:a 192k "out/st-src/05 menu.mp2"
ffmpeg -f lavfi -i "sine=f=880:r=22050:d=8" -c:a libvorbis "out/st-src/07 race.ogg"
make soundtrack-import SRC=out/st-src SOUNDTRACK=out/soundtracks/demo
cat > out/soundtracks/demo/tracks.txt <<'EOF'
track 4 = 04 attract.wav
[track 5]
file = 05 menu.mp2
loop_start = 1
loop_end = 5
track 7 = 07 race.wav
EOF
make music SOUNDTRACK=out/soundtracks/demo   # -> out/topskatr/music-demo.wav
```

In that capture the
game plays track 4 (440 Hz, cut off by the game's stop at 19.3 s) and then
track 5. Track 5 plays 300 Hz, then 500 Hz and 700 Hz alternating every 2 s
as the 1-5 s loop repeats; the 900 Hz tail never plays. It fades out left,
then right, from 48.0 s to 49.7 s. Track 7 plays 880 Hz from 54.7 s, looping
every 8 s. The parts of the render not replaced are sample-identical to a
render without the soundtrack.
