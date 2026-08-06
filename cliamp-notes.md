# Notes: cliamp (github.com/bjarneo/cliamp) — what's worth borrowing for climusic

Researched 2026-08-06 by cloning the repo and reading source/docs directly
(not from memory). cliamp is a Go TUI built on Bubbletea + Lip Gloss — a much
larger, more mature project (it supports Spotify, Qobuz, Navidrome, Plex,
Jellyfin, NetEase, podcasts, radio, and more, on top of YouTube/SoundCloud).
Most of its code doesn't transfer directly (different language, different
audio backend — it plays audio itself via the `beep` library rather than
shelling out to mpv), but several of its *decisions* are directly applicable.

## Things climusic already does the same way (validates our approach)

- **SoundCloud profile browsing is identical in shape**: cliamp's `[soundcloud]
  user = "..."` config exposes exactly three playlists — **Tracks**, **Likes**,
  **Reposts** — for that profile, via yt-dlp, same as our SoundCloud tab. Good
  sign we picked the right model.
- **They hit the same yt-dlp failure mode we did.** Their docs call out: *"Some
  tracks 404 on SoundCloud's per-track format API even though the page and
  search index still show them"* — subscriber-gated, region-blocked, deleted-
  but-cached, or transient extractor glitches. This is the same class of
  problem as our `get_audio_url` failures.

## Ideas worth adopting

### 1. Friendlier playback-failure messages
cliamp doesn't show raw yt-dlp stderr. It classifies the failure and shows:
*"Couldn't play X — track is gated, restricted, or unavailable."* Right now
`climusic`'s `get_audio_url` errors surface yt-dlp's raw stderr verbatim in
the status bar, which is noisy and not always meaningful to a user who isn't
reading yt-dlp source. **Suggestion**: pattern-match a few common yt-dlp
failure strings (HTTP 404/403, "This track is not available", "geo restricted")
in `soundcloud.rs`/`youtube.rs` and translate them into one plain sentence,
falling back to the raw message only when nothing matches. Low effort,
meaningful polish, and directly explains the exact failure mode both projects
hit.

### 2. Curated theme presets instead of (or alongside) per-track reactive color
cliamp ships ~19 named, hand-picked palettes as flat TOML files (catppuccin,
gruvbox, nord, dracula, tokyo-night, rose-pine, everforest, kanagawa,
vantablack, ...) — each just 6 semantic hex colors (`accent`, `bright_fg`,
`fg`, `green`, `yellow`, `red`). Users cycle/pick with `t`. This is a much
cheaper, more predictable design than a full reactive-theme engine: no
per-image color math, no risk of an ugly muddy accent from a bad cover, and
users who dislike the vibe just pick another preset.

We went a different direction (single accent color extracted live from the
current track's artwork via `color-thief`) — that's still worth keeping since
it's a nice "belongs to this track" touch and is already built, but a
**curated fallback palette** would fix the case that's currently ugly: no
artwork, or a washed-out/monochrome cover producing a muddy accent. Concretely:
ship 4-6 built-in accent presets (a teal, a magenta, an amber, a violet —
picked once by hand, not derived) and use one as the base instead of the
current flat gray whenever `artwork_accent` is `None` or its extracted
saturation is very low.

### 3. Undo for destructive list edits
cliamp has `Ctrl+Z` to undo the last playlist removal or queue clear. We
don't have anything like it — deleting a playlist (`d` in the Library tab) or
clearing the queue is currently unrecoverable. **Suggestion**: keep a
single-slot "last destructive action" (the removed playlist's rows, or the
cleared queue's contents) and wire `Ctrl+Z` to restore it. Cheap safety net,
matches an established convention.

### 4. Small-terminal degradation
cliamp explicitly documents behavior down to a **40×10** terminal (keeping
playback focus locked to the playlist there so EQ/source controls can't be
triggered by accident). We've never tested climusic below a "normal" terminal
size — the two-pane layouts (Library, SoundCloud) use fixed-ish proportions
that will likely break or clip badly well before 40×10. **Suggestion**: at
minimum, verify nothing panics or renders garbage under ~60×15, and consider
collapsing the two-pane SoundCloud/Library layouts to a single pane below
some width threshold. Haven't attempted this — flagging it as untested, not
implemented.

### 5. Genre-seeded fallback browse list
When no SoundCloud username is configured, cliamp doesn't just show an empty
state — it seeds the browse pane with curated *search-backed* virtual
playlists: Trending, Hip-Hop, Electronic, House, Lo-Fi, Indie, Pop (each is
just a live `scsearch:` query, not an editorial chart — SoundCloud's real
chart endpoints 404 through yt-dlp for them too). Our SoundCloud tab currently
just says "press C to set a username" when unset. **Suggestion**: same trick —
seed the Categories list with a few genre buckets that run `scsearch:<genre>`
through our existing `SoundCloudSource::search`, so the tab is useful before
you've configured anything.

### 6. Browser-cookie sign-in for gated/private content
cliamp supports `cookies_from = "firefox"` (etc.) to pass
`--cookies-from-browser <name>` to every yt-dlp call, which lets it access
private likes, hidden uploads, and SoundCloud Go+ subscriber-gated tracks by
reusing an already-logged-in browser session — no OAuth app needed (SoundCloud
closed that program in 2014, so this is the only real option). We don't
support this at all right now, so any gated/private content in someone's
Likes will just fail with "empty audio URL." **Suggestion**: add an optional
`cookies_from_browser` config key, threaded through as `--cookies-from-browser
<name>` on our existing yt-dlp invocations. Small, high-value if you (or
anyone using climusic) has private Likes or region/subscriber-gated tracks.

## Interesting, but explicitly not recommending right now

### Live FFT spectrum visualizer
cliamp has a genuinely large subsystem here — `ui/fft.go` plus **~25 distinct
visualizer styles** (bars, matrix rain, sakura petals, fireworks, scope,
stereo, mosaic...) driven by real-time FFT off the actual audio samples. This
is only possible because cliamp plays audio itself via the `beep` Go library,
so it has direct access to raw PCM. **climusic plays audio by shelling out to
mpv over JSON IPC** — we never see raw samples, only playback control
properties (position, pause state, volume, eof-reached). Building a real
visualizer would mean either replacing mpv with a Rust audio backend (a much
bigger architectural change than anything in this session) or trying to pull
FFT data out of mpv via its `af=drmeter`/audio-visualization filters over IPC,
which is fiddly and not really what those filters are for. Worth knowing this
exists as a "if we ever rearchitect playback" idea, not a near-term one.

### Parametric EQ
Same story — needs real audio-signal access we don't have through mpv's IPC
surface. Skipping for the same reason.

## Not investigated
cliamp is large (Spotify/Qobuz/Navidrome/Plex/Jellyfin providers, lyrics,
podcasts, radio, plugin system, remote control, mediactl/MPRIS integration).
Those are out of scope for climusic's current focus (local + YouTube +
SoundCloud + Spotify-discovery) and weren't reviewed in depth.
