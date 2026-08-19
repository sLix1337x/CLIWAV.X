![CLIWAV.X — a low-overhead CLI music player for Windows 11](banner.png)

Plays local files, YouTube and SoundCloud, with Spotify for track discovery.
Audio decoding is offloaded to [mpv](https://mpv.io/), so the player itself
stays light enough to leave running while gaming.

## Install

**Windows**

```powershell
irm https://sLix1337x.github.io/CLIWAV.X/install.ps1 | iex
```

**macOS**

```bash
curl -fsSL https://sLix1337x.github.io/CLIWAV.X/install.sh | bash
```

Both pull in [mpv](https://mpv.io/) and
[yt-dlp](https://github.com/yt-dlp/yt-dlp) if they're missing — via winget on
Windows, Homebrew on macOS. Re-run either one any time to update.

## Usage

```bash
cliwavx      # or: wavx
```

| Key | |
|---|---|
| `1`–`7` | Dashboard · Now Playing · Library · Queue · SoundCloud · EQ · Search |
| `Enter` `a` | Play · add to queue |
| `Space` `n` | Pause · next |
| `l` `x` | Repeat · shuffle |
| `v` `t` | Visualizer · accent color |
| `?` | All keybindings |

## Features

- **Every source in one queue** — local files, YouTube, SoundCloud and
  Spotify results mix freely, and playback auto-advances through whatever
  list you started from.
- **Browse a SoundCloud profile** — Tracks, Likes and Reposts, paged in the
  background so an account with thousands of likes never blocks the UI.
- **10-band EQ** driven through mpv's own filter chain, with presets.
- **Live spectrum visualizer** — digital rain and rising particles, fed by
  real system-audio capture rather than faked from the playback clock.
  Windows only: macOS has no equivalent way to capture system output without
  a virtual audio device.
- **Album art in the terminal**, full color where the terminal supports a
  graphics protocol, with an accent color pulled from each cover.
- **Real waveforms** instead of a plain progress bar, decoded per track.

## Configuration

Written on first run to `config.toml` —
`%APPDATA%\climusic\climusic\config\` on Windows,
`~/Library/Application Support/com.climusic.climusic/` on macOS. Local
folders, Spotify credentials and a SoundCloud username can all be set from
inside the app — press `S` or `C`.

## Credit

Artwork caching and cover-accent color took cues from
[Myx](https://github.com/HaseebKhalid1507/Myx) — independently implemented,
not copied.

## License

[MIT](LICENSE)
