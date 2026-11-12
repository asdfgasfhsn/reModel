# Mods

Changes to how a game looks or sounds that the original hardware could not
make. The game's own code keeps running unchanged; reModel swaps what it
produces at a well-defined point in the hardware model.

## Custom soundtracks

You can replace any of the game's music tracks with your own audio. The game
still decides when a track starts, stops and loops, how loud it plays and when
it fades. reModel swaps the audio at the MPEG decoder: when the DSB2 firmware
tells the decoder to play a range of MPEG ROM that belongs to a replaced
track, the decoder streams your file and leaves the ROM alone. Tracks you
don't replace play the original music, bit for bit.

This works for any game with a DSB2 music board, and for `stcc`'s DSB1
(`make music GAME=stcc SOUNDTRACK=...`). Tracks are matched by number
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
  reModel's own decoder (the one that plays the game's ROM). A leading ID3
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
topskatr every track starts at a different address. reModel reads the track
table from the firmware layout found in topskatr's DSB2 program (pointer at
`0x8000`, count at `0x800A`, start and end at entry offsets 2 and 6). Other
DSB2 firmware is untested. If its table isn't recognised, the render says
`firmware track table: 0 tracks`; give each entry a `range` instead.

On the DSB1 (`stcc`) a track is a list of segments the Z80 program chains
(`docs/hw/dsb1.md`). Its number is matched through the range of its first
segment, and the file then stands in for the whole track: intro and loop
segments alike. The game's play mode decides whether it loops (all but
track 17 do), unless `loop` says otherwise.

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
This is how reModel was verified on topskatr's attract-into-race capture:

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

## Planned: live soundtracks

*Not started; recorded 2026-11-12.* The custom soundtracks above replace a
game's music with fixed files. The plan is to let a music service supply it
instead, so a long session isn't the same handful of tracks on repeat, and to
make it look native: the song's title in the game's own pop-up and its album
art on the game's billboards. Everything is render-only and opt-in. The game's
code, save states and state hashes are unaffected, and with it off the game
plays its own music, bit for bit.

It works on games whose music has its own board: the DSB2 (`topskatr` and
others) and `stcc`'s DSB1. There, the music can be muted while the sound
effects keep playing, and the board's play, stop and fade commands (above)
say when the game changes track. Games that play music on the main SCSP can't
separate it cleanly.

### 1. Where the music comes from

In order of how native it feels:

- **A Spotify Connect receiver (native player only).** While a game runs,
  reModel appears as a Connect device ("reModel: Top Skater") in the Spotify
  app on your phone or desktop. You pick it and play to it, as you would to a
  console or a speaker. The audio goes into reModel's own mixer, under the
  game's fade and gain commands like a soundtrack file, with ducking under
  callouts. Track changes, title, artist and album art come with it. It needs
  Spotify Premium.
  - Spotify's official receiver SDK is only for licensed hardware partners.
    The way open to us is librespot, an open-source Connect receiver written
    in Rust (MIT), which is what spotifyd and Raspotify use.
  - librespot is reverse-engineered and not sanctioned by Spotify. Spotify
    has broken it before, and using it is outside Spotify's developer terms.
    So it would be an optional build feature, off by default and kept apart
    from the rest, like DLSS in `docs/enhancements.md`. That needs revisiting
    before any open-source release.
  - The browser build can't do this: a web page can't announce itself on the
    network.
- **An AirPlay receiver**, the equivalent for Apple Music, and for anything
  else an iPhone, iPad or Mac can play. reModel would appear in the AirPlay
  menu as a speaker while a game runs. It works the same way as Connect:
  audio into reModel's mixer, with title, artist, album and art. It is
  native only, for the same reason. Two ways to get it (checked
  2026-11-12; licences not yet read):
  - **Embedded, in Rust.** `openairplay2` is a receiver library: it takes
    the network side and hands the host decoded PCM and now-playing events
    (title, artist, album, cover art, volume), which is exactly the shape
    reModel needs. As described in its README (v0.5), it supports one
    sender and one stream, AAC at 44.1 kHz only (no ALAC, no realtime
    streams), and no persistent pairing: senders pair again each session.
    It needs no PTP clock daemon for a single output. `shairplay` is another
    Rust receiver, LGPL-3.0, with AirPlay 2 behind a feature flag, pre-1.0.
    Both are young.
  - **Decided (2026-11-12): receivers run inside reModel**, not as
    separate programs. Shairport Sync, the mature receiver that runs
    alongside (audio and metadata through pipes, with its NQPTP clock
    daemon), stays only as a reference point if the Rust libraries fall
    short.
  - AirPlay, like Connect, is reverse-engineered and not sanctioned by
    Apple, so the same rule applies: an optional feature, off by default,
    kept apart.
- **Remote control of a player you already run.** On Linux this goes through
  MPRIS, the desktop's media-control interface: Spotify's desktop app,
  browsers, mpv and others support it. No accounts, keys or unofficial
  clients. reModel mutes the game's music and sends play, pause and next to
  the player. The music plays through the player, not reModel's mixer, so
  the game's fades only become pause and play. This is the simplest version
  and the fallback for the others.
- **Local files.** The existing soundtrack packs, with titles and art taken
  from the files' tags.

Behaviour, for all of them (settings per game, as the others are):

- **The streamed music is independent of the game's track changes by
  default.** It plays song after song, as the service sends them, and the
  game's music stays muted. `topskatr` plays a few tracks a game, and they
  appear to be cut and edited to fit the game's sections: a menu loop, a run,
  the results (unconfirmed; its tracks 4, 6, 10 and 11 play once, the rest
  loop). A streamed song can't match those cuts. Skipping to a new song at
  every game track change would cut songs off mid-way.
- Optional: **follow the game.** Skip to the next song when the game starts
  a new track.
- Optional: **let the game's cues through.** The game's short one-shot
  tracks, such as a results or game-over sting, play over the stream with
  the stream ducked, so the game's punctuation stays.
- Pause the music when the game is paused, in attract mode, or at game over.
- Duck under the game's announcer callouts.
- With no source, or if the source stops, fall back to the game's own music
  and say so.

### 2. In the player's interface

- Settings: Music (Game, Soundtrack pack, Spotify Connect, External player),
  the player to control, and the behaviour switches above.
- A small now-playing notice (title, artist, art) for a few seconds when a
  song starts, drawn over the bezel or wide-screen side area, never over
  the play field. It can be turned off.
- Next, previous and play/pause on keyboard keys and a gamepad
  combination, and a "Now playing" line in the Esc menu.

### 3. The game's own track-title pop-up

`topskatr` shows each song's title when it starts. The plan is to put the
playing song's title in that pop-up, in the game's own font and animation:

- Find how the game draws it. Unknown so far: it is likely text on the 2D
  tile layer, but could be polygons. Which code shows it and when, where the
  text goes, which characters the font has, and how wide the box is all need
  finding.
- At render time, draw our title with the game's glyphs in place of the
  game's text, as the wide-screen HUD placement does, without writing to
  game memory. The font is read from the game's ROM at run time.
- When a song changes and the game didn't change its track, play the
  pop-up's animation ourselves.
- Arcade fonts usually have only capitals, digits and some punctuation, so
  accents become plain letters. Missing characters, such as Japanese, fall
  back to the now-playing notice. Long titles scroll or are cut short.
- The pop-up's location, font and timing go in the game's manifest, so the
  mechanism stays generic. Games without a pop-up use the notice.

### 4. Album art on billboards

`topskatr`'s billboards carry a band's logo. A live texture-pack entry would
show the playing song's album art there instead:

- Texture packs already replace any texture, matched by content, and draw
  with mipmaps on the GPU (`docs/texture-packs.md`). New is an entry whose
  image is "the current album art", updated on the GPU when the song
  changes. Every billboard using that texture follows, lit and fogged as
  the game draws it.
- Find the logo's textures with `make dump-textures`. There may be several,
  at different sizes. If the logo shares a texture sheet with other
  graphics, the entry needs a sub-rectangle, which is a small addition to the
  pack format.
- Album art is square and billboards may not be: the entry says whether to
  fit it with a border, crop it, or fill around it with a colour taken from
  the art.
- The entry lives in the user's own pack, never in the repo. With no music
  playing, the original texture shows. It works for any game with
  billboards or screens.

### Order of work

1. Investigate `topskatr`'s title pop-up and logo textures.
2. Remote control over MPRIS, with the settings and the notice.
3. Live album-art texture slots.
4. The in-game title pop-up, first with local soundtrack packs.
5. Spotify Connect (librespot) and AirPlay (`openairplay2`, else
   `shairplay`) receivers built into reModel, as optional features.
