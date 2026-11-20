# alaska

A simple, fast terminal music player for local files, built around a file browser.

```
╭ ~/Music/Artist/Album ──────────╮╭ Queue (2/6) ──────────────────╮
│01 Intro – Artist           1:12││  01 Intro – Artist        1:12│
│02 Song – Artist            3:45││▶ 02 Song – Artist         3:45│
│03 Other – Artist           4:01││  03 Other – Artist        4:01│
╰────────────────────────────────╯╰───────────────────────────────╯
╭─────────────────────────────────────────────────────────────────╮
│ ▶ 02 Song – Artist  ·  Album                                    │
│ 1:23 / 3:45  ━━━━━━━━━━━━━━━━━━━━━━━━──────────────────────────  │
│ vol  80%  shuffle off  repeat all                               │
╰─────────────────────────────────────────────────────────────────╯
```

## Features

- Browse the file system; playing a track queues its whole folder
- MP3, FLAC, Ogg Vorbis, Opus, M4A/AAC and WAV
- Tags (track number, title, artist, album, duration) with file-name fallback
- Editable queue, shuffle, repeat (off / all / one), seeking, volume
- Live fuzzy filter for the current directory
- MPRIS: media keys, `playerctl` and desktop widgets work
- Remembers volume, shuffle/repeat, queue and position between runs (resumes paused)

## Build

Requires Rust and the system libraries for ALSA and Opus (on Arch: `alsa-lib opus`).

```sh
cargo build --release
./target/release/alaska [DIRECTORY]   # defaults to the current directory
```

## Keys

| Key                     | Action                                              |
|-------------------------|-----------------------------------------------------|
| `j`/`k`, `↓`/`↑`        | Move cursor                                         |
| `Ctrl-d`/`Ctrl-u`, PgDn/PgUp | Move cursor by 10                              |
| `g`/`G`                 | Top / bottom                                        |
| `l`/`Enter`/`→`         | Open directory, play track (queue: jump to track)   |
| `h`/`Backspace`/`←`     | Parent directory                                    |
| `~`                     | Home directory                                      |
| `Tab`                   | Switch focus between browser and queue              |
| `Space`                 | Play / pause                                        |
| `n` / `p`               | Next / previous track                               |
| `+` / `-`               | Volume up / down                                    |
| `.` `>` / `,` `<`       | Seek forward / backward 5 s                         |
| `s`                     | Toggle shuffle                                      |
| `r`                     | Cycle repeat: off → all → one                       |
| `/`                     | Filter (Enter keeps it, Esc clears it)              |
| `Esc`                   | Clear filter                                        |
| `d`                     | Remove track from queue (queue focus)               |
| `J` / `K`               | Move track down / up in queue (queue focus)         |
| `q`                     | Quit                                                |

## Files

- `~/.local/state/alaska/state.json` – volume, modes, queue and position
- `~/.cache/alaska/tags.json` – tag cache (invalidated by file modification time)
