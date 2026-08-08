# CLIWAV.X

<img width="370" height="238" alt="CLIWAVEX-LOGO2" src="https://github.com/user-attachments/assets/ff58f445-2208-4c1d-9948-474d0725487c" />

A low-overhead CLI music player for Windows 11. Play local files, YouTube, SoundCloud, and use Spotify for track discovery. Designed to stay out of the way while gaming.

## Why not just use a browser?

Browsers are heavy. `CLIWAV.X` uses a native Rust TUI and offloads audio decoding to `mpv`, so the CPU/memory footprint stays tiny.

## Features

- **Dashboard home tab**: the landing screen shows what's playing (artwork + track info), a selector for Search / Tracks / Likes / Reposts / Library (switch with `←` `→`, `Enter` to act), and the queue — all at a glance. Search here queries every source (local library, YouTube, SoundCloud, Spotify), same as the main Search tab; Library is a quick jump to the Library tab. Without a username configured, the selector instead browses genre buckets (Trending, Lo-Fi, House, ...) backed by live search, so it's useful out of the box.
- **Local playback**: Scan your music folders and play FLAC, MP3, OGG, OPUS, M4A, AAC, WAV, WMA.
- **YouTube & SoundCloud**: Search and stream via `yt-dlp`. Pasting a share link directly into search resolves it as a single track instead of running it through keyword search.
- **SoundCloud user browsing**: Set your SoundCloud username and browse their Tracks / Likes / Reposts (category list first, then tracks within it). Loaded 100 tracks at a time in the background — press `m` for more, so collections with thousands of entries never block the UI.
- **Album artwork**: The Now Playing tab renders the track's artwork directly in the terminal (via `ratatui-image`), alongside the track info on wide terminals (stacked on narrow ones). Fidelity depends on your terminal — full color in Kitty/iTerm2/WezTerm/Sixel-capable terminals, a blockier Unicode "halfblock" rendition elsewhere (e.g. plain Windows Terminal), still recognizable either way.
- **Waveform timeline**: The Now Playing tab shows a real, mirrored amplitude waveform instead of a flat progress bar, colored played/unplayed like SoundCloud's or foobar2000's seekbars. Local files are decoded directly (via `symphonia`); SoundCloud tracks reuse the source's own precomputed waveform when available (no extra audio download); YouTube fetches a low-bitrate stream just for the analysis, separate from the playback-quality one, and every track's result is cached for the session so replaying it doesn't re-fetch. Falls back to the plain bar while loading or for a codec it can't decode (e.g. WMA, some Opus streams) — never an error, just no waveform for that track.
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

## Install

### One-liner (recommended)

Open PowerShell and run:

```powershell
irm https://sLix1337x.github.io/CLIWAV.X/install.ps1 | iex
```

The installer will:
1. Install [mpv](https://mpv.io/) and [yt-dlp](https://github.com/yt-dlp/yt-dlp) automatically if `winget` is available and they are missing.
2. Download the latest release binary — as both `cliwavx.exe` and `wavx.exe` (identical, just a shorter name) — and copy them to `%LOCALAPPDATA%\CLIWAV.X`.
3. Add that folder to your user PATH.

If `winget` is unavailable, yt-dlp is downloaded as a portable executable into the same install folder. mpv will need to be installed manually in that case.

After installation, **restart your terminal** (or run `refreshenv` if you have Chocolatey) so PATH changes are picked up. Then you can run `cliwavx` (or the shorter `wavx`) from any folder.

### Updating

Run the same one-liner again:

```powershell
irm https://sLix1337x.github.io/CLIWAV.X/install.ps1 | iex
```

Every push to `master` rebuilds both binaries and republishes them to the
`latest` GitHub release, so re-running the installer always fetches the
newest build and overwrites the ones in `%LOCALAPPDATA%\CLIWAV.X`. It skips
mpv/yt-dlp if they're already installed.

### Build from source

If you have the [Rust toolchain](https://rustup.rs/) installed, you can also build and install locally:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1
```

That script installs dependencies (when possible), builds optimized release binaries, copies them to `%LOCALAPPDATA%\CLIWAV.X`, and adds it to your user PATH.

## Build

```bash
cargo build --release   # optimized (LTO, stripped): target/release/cliwavx.exe and target/release/wavx.exe
```

## Usage

`wavx` is a shorter alias for `cliwavx` — same binary, same behavior, pick whichever's less typing.

```bash
# Launch interactive TUI
cliwavx
wavx

# Launch with a search query
cliwavx -q "lofi hip hop"
```

### Keybindings

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Switch tabs. Dashboard: toggle pane. Search: toggle typing vs. browsing results. Library/SoundCloud: toggle pane. |
| `1` `2` `3` `4` `5` `6` | Jump to Dashboard / Now Playing / Library / Queue / SoundCloud / Search |
| `↑` `↓` / `k` `j` | Navigate list |
| `←` `→` (Dashboard) | Switch Search / Tracks / Likes / Reposts / Library, reloads categories as you land on them |
| `Enter` (Search, typing) | Run search, then switch focus to the results list |
| `Enter` (elsewhere) | Play selected track, or (SoundCloud categories) load the selected category |
| `a` | Add selected track to queue |
| `p` | Add selected search track to current playlist |
| `Space` | Pause / resume |
| `Shift+←` / `Shift+→` | Rewind / fast-forward the current track by 5s |
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
audio_exclusive = false
```

## Audio quality

YouTube and SoundCloud streams are resolved via `yt-dlp` with an explicit
bitrate-first format sort, so CLIWAV.X always picks the highest-kbps audio
stream the source actually offers (which is capped by what YouTube/SoundCloud
publish — typically ~128-160kbps AAC/Opus for free-tier streams; this can't
exceed that).

Local files already play bit-perfect by default — nothing in CLIWAV.X
transcodes or resamples them. On Windows, mpv's default WASAPI *shared*-mode
output goes through the OS mixer, which can still resample everything to one
fixed format. Set `audio_exclusive = true` in `config.toml` to run mpv in
WASAPI *exclusive* mode instead, bypassing the mixer so lossless local files
(FLAC, etc.) play at their true native sample rate. Trade-off: exclusive mode
takes over the audio device, muting every other app's sound while a track
plays, and can click briefly on track/device changes — so it's off by default.

## Architecture

- `mpv` runs as a persistent background process and receives commands over a named pipe.
- `yt-dlp` extracts direct audio URLs for YouTube and SoundCloud.
- Local metadata is indexed in SQLite using `lofty`.
- The TUI is built with `ratatui`.
- Waveform amplitude data is decoded with `symphonia` (local files and, as a fallback, YouTube/SoundCloud streams) — this is separate from playback, which still goes through `mpv` unchanged.

## License

MIT
