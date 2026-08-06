# CLIWAV.X

<img width="1254" height="1254" alt="CLIWAVEX-LOGO" src="https://github.com/user-attachments/assets/9d426c9c-2841-4c1b-9327-c9f5e73d90e7" />

A low-overhead CLI music player for Windows 11. Play local files, YouTube, SoundCloud, and use Spotify for track discovery. Designed to stay out of the way while gaming.

## Why not just use a browser?

Browsers are heavy. `CLIWAV.X` uses a native Rust TUI and offloads audio decoding to `mpv`, so the CPU/memory footprint stays tiny.

## Features

- **Dashboard home tab**: the landing screen shows what's playing (artwork + track info), your SoundCloud Tracks/Likes/Reposts (switch with `←` `→`), and the queue — all at a glance. Without a username configured, the SoundCloud pane seeds genre buckets (Trending, Lo-Fi, House, ...) backed by live search, so it's useful out of the box.
- **Local playback**: Scan your music folders and play FLAC, MP3, OGG, OPUS, M4A, AAC, WAV, WMA.
- **YouTube & SoundCloud**: Search and stream via `yt-dlp`. Pasting a share link directly into search resolves it as a single track instead of running it through keyword search.
- **SoundCloud user browsing**: Set your SoundCloud username and browse their Tracks / Likes / Reposts (category list first, then tracks within it). Loaded 100 tracks at a time in the background — press `m` for more, so collections with thousands of entries never block the UI.
- **Album artwork**: The Now Playing tab renders the track's artwork directly in the terminal (via `ratatui-image`), centered with the track info and a progress bar under it. Fidelity depends on your terminal — full color in Kitty/iTerm2/WezTerm/Sixel-capable terminals, a blockier Unicode "halfblock" rendition elsewhere (e.g. plain Windows Terminal), still recognizable either way.
- **Cover-driven accent color**: a vibrant color is pulled from each track's own artwork (via `color-thief`) and used to tint the Now Playing panel and volume meter, so the screen feels like it belongs to what's playing. Press `t` to override it with a curated palette (teal/magenta/amber/violet) — handy for monochrome covers that extract to a muddy accent.
- **Spotify discovery**: Search tracks via the official Spotify Web API; playback is resolved through YouTube or local files.
- **Unified queue**: Mix tracks from any source in one queue.
- **Repeat & shuffle**: Repeat off/track/all, plus shuffle for queue playback; tracks auto-advance when they finish — including through whatever list you played from (Search results, a playlist, or a SoundCloud category), not just the queue. Running off the end of a paginated SoundCloud page fetches the next one automatically and keeps going.
- **Keyboard-driven TUI**: Fast navigation with vim-style keys.

## Requirements

- Windows 11 (primary target; cross-platform builds possible)
- [mpv](https://mpv.io/)
- [yt-dlp](https://github.com/yt-dlp/yt-dlp)
- Rust toolchain

Optional:
- Spotify Web API credentials for search: https://developer.spotify.com/dashboard
- `cookies_from_browser = "firefox"` (or chrome/edge/...) in `config.toml` reuses your logged-in browser session for yt-dlp — needed for private Likes and subscriber/region-gated tracks.

## Build

```bash
cargo build --release   # optimized (LTO, stripped): target/release/CLIWAVX.exe
```

## Usage

```bash
# Launch interactive TUI (binary: CLIWAVX.exe)
CLIWAVX

# Launch with a search query
CLIWAVX -q "lofi hip hop"
```

### Keybindings

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Switch tabs. Dashboard: toggle pane. Search: toggle typing vs. browsing results. Library/SoundCloud: toggle pane. |
| `1` `2` `3` `4` `5` `6` | Jump to Dashboard / Search / Queue / Library / SoundCloud / Now Playing |
| `↑` `↓` / `k` `j` | Navigate list |
| `←` `→` (Dashboard) | Switch SoundCloud category (Tracks / Likes / Reposts), reloads the list |
| `Enter` (Search, typing) | Run search, then switch focus to the results list |
| `Enter` (elsewhere) | Play selected track, or (SoundCloud categories) load the selected category |
| `a` | Add selected track to queue |
| `p` | Add selected search track to current playlist |
| `Space` | Pause / resume |
| `n` | Next track |
| `n` (Library tab) | Create a new playlist |
| `d` (Library tab) | Delete selected playlist |
| `Ctrl+Z` | Undo the last playlist deletion |
| `+` / `-` | Volume up / down |
| `f` | Cycle source filter (local → YouTube → SoundCloud → Spotify → all) |
| `g` | Toggle game mode (reduces TUI refresh rate) |
| `l` | Cycle repeat mode (Off → Track → All) |
| `x` | Toggle shuffle |
| `t` | Cycle accent palette (auto from artwork → teal → magenta → amber → violet) |
| `m` (SoundCloud/Dashboard) | Load the next page of Tracks/Likes/Reposts |
| `S` | Set up Spotify credentials |
| `C` | Set your SoundCloud username |
| `r` | Rescan local library |
| `?` | Toggle in-app keybindings help (only outside the search box) |
| `Q` / `Ctrl+Q` / `Ctrl+C` | Quit |

While the Search box has focus (the default when you land on the Search
tab, or after pressing `Tab` to go back to it), every printable key
including letters, spaces, and pasted text is typed into the query —
none of the single-letter shortcuts above fire. This is what makes
pasting a share link safe; it used to be captured piecemeal by shortcuts
like `n` (next track) and, on error, could exit the whole app.

### SoundCloud user browsing

Press `C` to set a SoundCloud username, then switch to the SoundCloud tab
(`5`). You'll see three categories — **Tracks**, **Likes**, **Reposts** —
first; press `Enter` on one to load it, then `Enter` again to play a track.
Pages load 100 entries at a time in the background; press `m` while
browsing tracks to fetch the next page (repeatable — a collection with
thousands of Likes just takes a few more presses, never a long freeze).

### Playback bar

Directly under the current tab's content:

- A borderless "hero" row — bold, on its own gray background (lightly
  tinted by the accent color, never solid black) — always shows the
  playback state (Stopped / Playing / Paused) and the currently loaded
  track, so it reads as the focal point of the screen without competing
  with the boxed panel below it.
- Below it, a bordered panel shows volume as a level meter plus
  percentage and approximate dB (100% = 0 dB reference), and the current
  repeat/shuffle settings.

Both, plus the artwork panel on the Now Playing tab, tint to the current
track's cover-art accent color when one is available.

## Design credit

The visual design took cues from
[Myx](https://github.com/HaseebKhalid1507/Myx), a Spotify TUI player —
specifically its approach to caching a terminal-graphics protocol behind
interior mutability (so rendering doesn't need `&mut App`) and deriving
an accent color from cover art via `color-thief`. CLIWAV.X's version is
a smaller, single-accent take on the latter; Myx builds a full multi-layer
reactive theme from the same idea.

## Configuration

On first run, a default config is created at:

```
%APPDATA%\climusic\climusic\config\config.toml
```

Example:

```toml
[local]
paths = [
    "~/Music",
    "D:/Music",
]

[spotify]
client_id = "your-spotify-client-id"
client_secret = "your-spotify-client-secret"

[soundcloud]
username = "your-soundcloud-username"

[player]
mpv_path = "mpv"
yt_dlp_path = "yt-dlp"
volume = 80
```

## Architecture

- `mpv` runs as a persistent background process and receives commands over a named pipe.
- `yt-dlp` extracts direct audio URLs for YouTube and SoundCloud.
- Local metadata is indexed in SQLite using `lofty`.
- The TUI is built with `ratatui`.

## License

MIT
