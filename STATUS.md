# climusic — continuation summary

Last updated: 2026-08-06

## Goal status

Core next-step features are implemented and verified on Windows 11:

- [x] Library tab + playlist CRUD
- [x] Spotify credential setup wizard (`S`)
- [x] SoundCloud username browsing: Tracks / Likes / Reposts (`C` to set username, SoundCloud tab)
- [x] YouTube audio URL caching (10-minute TTL)
- [x] Pasted YouTube/SoundCloud share links resolve directly instead of being run through keyword search
- [x] Repeat (Off/Track/All) and shuffle, with auto-advance when a track finishes
- [x] In-app keybindings help overlay (`?`)
- [x] Volume shown as % and approximate dB; Stopped/Playing/Paused indicator
- [x] Game Mode toggle (`g`)
- [x] Fixed: key-triggered actions (e.g. mpv "next" with nothing queued) could silently exit the whole app — all now degrade to a status-bar error instead
- [x] Fixed: typing/pasting into the Search box was largely broken — most letters, and Space, were captured by global shortcuts instead of becoming query text (see "Search focus" below)
- [x] Local playback verified through mpv
- [x] YouTube playback verified through mpv
- [x] SoundCloud playback verified through mpv
- [x] `cargo build` succeeds, `cargo check --all-targets` clean (no warnings)
- [x] SoundCloud Tracks/Likes/Reposts paginated (100/page, background-loaded via `m`) instead of a hard 30-item cap — needed for accounts with thousands of Likes
- [x] Visual redesign: accent-bar section headers, block-character volume meter, zebra-striped lists, bullet-style source tags
- [x] Now Playing line (state + track) moved to its own unboxed row above the boxed volume/repeat/shuffle panel
- [x] Album artwork rendered in the Now Playing tab via `ratatui-image` (see dependency note below)
- [x] Fixed: artwork never showed for *any* track from YouTube or SoundCloud — `parse_entry` only checked a `thumbnail` field that yt-dlp's flat-playlist output never populates; the actual data lives in a `thumbnails` array. See `sources::best_thumbnail`.
- [x] Rewrote artwork rendering after studying github.com/HaseebKhalid1507/Myx's `cover.rs`: cache the encoded terminal-graphics protocol behind a `RefCell` keyed by render area, re-encoding only on resize, rendered via the static `Image` widget. Avoids threading `&mut App` through the whole `ui` module (the original `StatefulImage` approach required it) and is closer to how a shipped ratatui-image app actually does it.
- [x] Cover-driven accent color (`app.artwork_accent`, via `color-thief`): the Now Playing panel, its volume meter, and the boxed "hero" status line all tint to a vibrant swatch pulled from the current track's own artwork — a smaller, single-accent version of the "reactive theme" idea in Myx's `reactive.rs`/`color.rs` (which derives a full multi-layer theme; this only derives one accent color).
- [x] Fixed (for real this time): artwork still didn't show for SoundCloud Likes/Reposts/Tracks-page entries even after the `thumbnails`-array fix above — those flat-playlist entries carry *no* thumbnail data at all, only search results do. Added a fallback to SoundCloud's public oEmbed endpoint (`sources::soundcloud::fetch_oembed_thumbnail`, per developers.soundcloud.com/docs/oembed), used only for whichever single track is actually playing — cheap even for a 9,000-entry list.
- [x] Autoplay through a browsed list: playing a track directly from Search/Library/SoundCloud (not the queue) now auto-advances to the next entry in that same list when it ends, the same way queue playback already did. For a paginated SoundCloud category, running off the end of the loaded page triggers a background fetch of the next page and resumes automatically once it lands — see `PlaybackOrigin`.
- [x] Now Playing hero row: removed the border (background-fill only, no outline), and the background is now a neutral gray lightly tinted by the accent color (25% accent / 75% gray) instead of a near-black dimmed-accent color that read as plain black.
- [x] Now Playing hero line: state + track now on their own bold, background-tinted boxed row (previously an unboxed line) so it reads as the focal point of the screen.
- [x] Artist/uploader name is now visually distinct from the track title everywhere a track is listed (muted gray vs. bold white) — previously rendered as one same-colored span.
- [x] Gradient headlines: the "♫ climusic" tab-bar title and the hero row's "P L A Y I N G" state word render as per-character RGB gradients (`ui::gradient_spans`).
- [x] Animated Braille spinner (`⠋⠙⠹…`, advanced per UI tick via `app.tick`) for background work: SoundCloud page fetches (list header + empty-state hint) and local library scans (status bar). *Not* wired to search — `run_search` still blocks the event loop, so a spinner there couldn't animate; making search background is a separate change.
- [x] Gradient volume meters: the filled portion interpolates from a dimmed to a brightened shade of the accent color (`ui::gradient_meter_spans`) in both the Now Playing panel and the persistent controls bar.
- [x] Curated accent palette presets (`t`): Auto (artwork-driven) → Teal → Magenta → Amber → Violet, as an override for monochrome/washed-out covers that extract to a muddy accent (from the cliamp notes). Shown in the status bar; not persisted across restarts.
- [x] Per-source glyphs instead of the same colored dot everywhere: ♪ Local, ▶ YouTube, ☁ SoundCloud, ✳ Spotify (`ui::source_glyph`; single-width glyphs only, so list columns stay aligned). Source colors were also consolidated into one `ui::source_color` helper — the same match was copy-pasted in four files.
- [x] Full-width dim divider rules between the tab bar and content, and between content and the playback hero row.
- [x] Fixed: auto-advance on track end never fired, for *any* source — when a track ended, mpv (`--idle`) unloaded the file and every playback property (`eof-reached`, `time-pos`) became "property unavailable", so `poll_playback`'s `is_eof_reached()` returned `Err` and the `if let Ok(true)` guard never matched. mpv now runs with `--keep-open=yes` (pauses on the finished file instead, leaving `eof-reached=true` readable), and `MpvPlayer::load` explicitly clears the keep-open pause flag (`loadfile` doesn't reset it, or the track after an auto-advance would start paused).
- [x] Fixed: mpv IPC race — `send_and_read` read exactly one line per command, but mpv broadcasts async events (`{"event":"end-file",...}`) on every connection, and one landing ahead of the command response was misparsed as the response ("error running command"). Responses are now identified by their `error` field; event lines are skipped. This race was likeliest exactly around track end — i.e. on the autoplay path.
- [x] Verified end-to-end (throwaway `autoplay_check` bin, deleted after use): played entry 0 of a real SoundCloud user's Tracks list, sought to the end, and `poll_playback` auto-advanced to entry 1 on the first poll. EOF detection probed for both a local file and a SoundCloud HLS stream.

## Audit-fix round (critical + major findings)

- [x] **Critical — rescan no longer corrupts playlists containing local tracks.** Root cause was threefold: `PRAGMA foreign_keys` was never enabled (so the schema's `ON DELETE CASCADE`/`SET NULL` were dead text), `scan_library` deleted all local rows and re-inserted them under new AUTOINCREMENT ids (orphaning every `playlist_tracks.track_id`), and `get_playlist_tracks` read the then-NULL `path` with `row.get::<_, String>` which errored the *entire* playlist load — up to aborting `App::new` at startup. Now: foreign keys + a 5s `busy_timeout` are enabled at open; `scan_library` does one atomic `sync_local_tracks` (path-keyed upserts keep ids stable; only genuinely-deleted files are removed, which cleanly NULLs the reference); playlist rows store the playable URL for *all* sources as a self-sufficient fallback; and a row with no playable source is skipped instead of failing the playlist. Covered by three `db.rs` unit tests (rescan keeps references, row survives track leaving index, delete cascades).
- [x] Startup no longer force-kills every `mpv.exe` on the machine (`taskkill /F /T /IM` removed — the pid+nanos pipe name made it unnecessary, and it killed the user's unrelated mpv windows).
- [x] Playing an entry from the Queue tab no longer wipes the whole queue (`clear_queue=true` on that path): it now dequeues just that entry and auto-advance continues through the rest. `queue_selected` is clamped on every queue shrink. Verified E2E (throwaway `queue_check` bin, deleted after use): played queue entry 0, remaining entry survived, natural EOF auto-advanced into it.
- [x] Timeouts everywhere a stall used to freeze the app: every yt-dlp invocation goes through `sources::run_yt_dlp` (30s, 120s for paginated category fetches) with `kill_on_drop` so a timed-out child can't be orphaned; all reqwest clients (Spotify, oEmbed, artwork) get explicit timeouts via `sources::http_client`; mpv IPC round-trips have a 5s deadline.
- [x] mpv's stderr is now drained by a background task — previously the piped stderr filled its ~64KB OS buffer and mpv blocked on write, silently freezing IPC mid-session.
- [x] `r` during a running library scan no longer spawns a second concurrent scan (two writers, one SQLite file → spurious "database is locked"); it reports "already in progress" instead.
- [x] mpv startup polls for the IPC pipe (50ms × up to 5s) instead of a blind 500ms sleep that raced slow machines into a startup abort.
- [x] A mpv that dies mid-session is restarted on the next command (`ensure_started` self-heals) instead of erroring until app restart.
- [x] `draw_prompt` no longer crashes on tiny terminals: `area.height / 2 - 2` underflowed `u16` below 4 rows, and at 4–5 rows the popup exceeded the buffer and ratatui's `Clear` panicked. The popup rect is now clamped to the frame.
- [x] Verification: `cargo check --all-targets` clean, `cargo test` 3/3 green, `cargo run --example verify` passes (local + YouTube + SoundCloud playback, URL cache hit). The audit's *minor* findings were explicitly left out of scope for this round.

## Minor-findings fix round

- [x] Partial/hand-edited `config.toml` no longer blocks startup: every section and field has a serde default (`src/config.rs`, defaults derived — the old `DEFAULT_CONFIG` template never actually reached the written file anyway).
- [x] anyhow-derived errors no longer mislabeled "config error" (`src/error.rs`).
- [x] Local search escapes LIKE metacharacters — typing `%`/`_` matches literally instead of wildcarding the whole library (`src/db.rs`).
- [x] `expand_path` handles bare `~` and Windows-style `~\...` in addition to `~/...` (`src/sources/local.rs`).
- [x] Ctrl+C/Ctrl+Q now quit even with an input prompt open; other Ctrl combos are swallowed instead of being typed into the field (`src/main.rs`).
- [x] `toggle_pause` no-ops with nothing loaded and syncs `is_playing` from mpv's real `pause` property instead of flipping a mirror flag (mpv can pause on its own, e.g. keep-open at EOF).
- [x] Auto-advance is peek-then-commit: a track is only removed from the queue / advanced past in its origin list once it actually starts. A failed next track sets an `autoplay_failed` latch (cleared by any successful play) instead of silently eating one list entry per tick — the status bar explains and offers `n` to skip.
- [x] `MpvPlayer::load` clears the keep-open pause flag *before* `loadfile`, so a failed command can't leave mpv playing a track the app believes failed.
- [x] SoundCloud stale-load races: the page-load handle is keyed by username+category and aborted (not detached) on replacement; category loads and username changes reset `soundcloud_autoplay_pending`/`playback_origin`; a manual play clears a pending autoplay so a landing page can't restart the track you just picked.
- [x] SoundCloud pagination no longer drifts: the next-page index is tracked explicitly (`soundcloud_next_start`) and `has_more` derives from yt-dlp's RAW entry count, so entries that fail to parse can't shift later pages or end paging early.
- [x] YouTube URL cache prunes expired entries on insert (bounded growth); Spotify reuses one shared 15s-timeout client instead of building one per call.
- [x] A timed-out yt-dlp is now killed process-TREE-wide on Windows (`taskkill /T /PID` scoped to our own child — under the cmd/python wrapper the real yt-dlp is a grandchild that `kill_on_drop` alone couldn't reach).
- [x] tracing logs to `climusic.log` in the cache dir instead of stdout, which the TUI owns (log lines used to corrupt the display).
- [x] Unix builds get a real socket path (`$XDG_RUNTIME_DIR`/`/tmp`) instead of the literal Windows `\\.\pipe\...` name in the cwd.
- [x] `best_thumbnail`: an "original"-tagged entry with no usable url no longer suppresses the max-width fallback. YouTube duration parsing is saturating (debug-build overflow panic on absurd JSON). Removed dead `Database::insert_local_track`.
- [x] Documented won't-fix (commented in code): mpv `Drop` not reaping the child (process is exiting; OS reaps), and IPC-timeout-mid-command ambiguity (bounded, self-correcting; request_id correlation not worth it).
- [x] Verification: `cargo check --all-targets` clean, `cargo test` 10/10 green (new: config defaults ×2, expand_path ×2, LIKE escape, best_thumbnail ×2), `cargo run --example verify` passes. Greps: no `.output()` in `src/sources/`, no bare `reqwest::Client::new` outside the shared helper, no stdout tracing writer in `main.rs`.

## Rename + Dashboard round

- [x] Renamed the app to **CLIWAV.X**: UI title gradient, and mpv is spawned with `--title=CLIWAV.X` so the Windows volume mixer shows the app name instead of the raw signed CDN stream URL (mpv pushes its title to the audio session). Cargo/rustc reject `.` in target names, so the binary target is the dotless `CLIWAVX` (`CLIWAVX.exe`); the library crate stays `climusic`.
- [x] Now Playing tab restyled as a centered hero page: capped-width artwork horizontally centered, gradient state headline, title/artist/source·album, gradient volume meter, and repeat/shuffle — the artwork+info group vertically centered as one block.
- [x] New **Dashboard** tab, now the first/default tab (`1`): Now Playing pane (artwork + info via the existing mini widget), a SoundCloud pane with a `◂ Tracks · Likes · Reposts ▸` selector (`←`/`→` switches category and reloads, `m` loads the next page, `Enter` plays with list autoplay, `a` queues), and a Queue pane (navigate, `Enter` dequeues-and-plays). `Tab` toggles between the two interactive panes; number keys are now `1`-`6`.
- [x] Fixed a real autoplay regression introduced by the minor-findings round: `MpvPlayer::load` had been changed to unpause *before* `loadfile`, which un-pauses the OLD at-EOF file — mpv resumes it, instantly re-hits EOF, and keep-open re-pauses, so every auto-advanced track started paused and never reached EOF (autoplay, Repeat-All, and shuffle-advance all looked dead; Repeat-Track via loop-file was unaffected). The unpause is back to *after* `loadfile` (the order the original keep-open fix was verified with). Verified with a probe that checks the auto-advanced track's playback position actually advances, not just that state says "Playing" — an assertion all earlier E2E checks were missing. Repeat-Track looping verified separately. Also: `default-run = "CLIWAVX"` so bare `cargo run` works, and the stale pre-fix `climusic.exe` was removed from `target/debug` (launching it would have shown exactly this pre-fix behavior).

## Audit round 3 (regression sweep after the rename/Dashboard round)

- [x] Fixed: **Repeat-All hijacked list playback** — `advance_to_next` pushed the finished track into the queue unconditionally, so playing from Search/Playlist/SoundCloud with repeat-all on replayed the same track forever and lost `playback_origin`. The recycle now only applies to queue-sourced playback, via an explicit recompute-per-advance path.
- [x] Fixed: **manual `n` couldn't skip a broken entry** despite the status bar saying "press n to skip" — peek-then-commit never committed on failure, so skip retried the same dead entry forever. Manual skip now drops the broken entry and tries the next candidate (the loop strictly shrinks the list, so it terminates); natural-EOF failure keeps the entry and latches.
- [x] Fixed: `is_track_loaded` misread a soft-fail as success when the next entry had the same source+id as the current one (duplicate entries) — `play_track` now returns `Result<bool>` and callers use that directly.
- [x] Fixed: a failed SoundCloud autoplay page fetch didn't set the `autoplay_failed` latch — an unbounded every-tick refetch storm. Both error branches latch now.
- [x] Fixed: `next_track` didn't clear `soundcloud_autoplay_pending` — a landing page could start a track the user had just skipped past.
- [x] Fixed: a mid-session mpv restart lost volume (reset to 100) and `loop-file` (wedging Repeat-Track playback, since the app trusts mpv to loop internally) — `MpvPlayer` remembers both and re-applies them after a respawn.
- [x] Fixed: **AltGr-typed characters were swallowed** in the search box and input prompts (crossterm reports AltGr as Ctrl+Alt; relevant on German/Nordic/French layouts — `@` = AltGr+Q). Only Ctrl-without-Alt is treated as a shortcut now.
- [x] Fixed: the help overlay swallowed the quit keys it advertises (Ctrl+C/Ctrl+Q did nothing with `?` open).
- [x] Fixed: on a standard 80x24 terminal the Now Playing tab clipped the Repeat/Shuffle line in exchange for a 1-row artwork sliver — artwork now collapses fully when there's no vertical budget.
- [x] Library scan skips are no longer silent: unreadable walk entries, metadata failures, and nonexistent configured paths all log `warn` to climusic.log (previously a transient metadata failure silently evicted the track from the index on that sync).
- [x] Docs/config cleanup: `cargo run` help and clap name say CLIWAV.X; README's SoundCloud tab key corrected to `5` and the unshipped "Spotify playlist import" claim reworded; `Config::save` prepends a guidance header (yt_dlp_path wrapper hint etc.) since pretty-serialization emits no comments; stale "climusic" title comment updated.
- [x] Verification: `cargo check --all-targets` clean, `cargo test` 10/10, `cargo run --example verify` passes, and a throwaway E2E probe (deleted after use) confirmed both rewritten advance paths with actual playback-position assertions: normal queue advance, and repeat-all recycle of a finished track on an empty queue.

## Feature round (post-audit roadmap 1–9)

- [x] **Progress bar + time display** on the Now Playing tab: position/duration are polled every tick (`playback_pos`/`playback_dur`) and rendered as a gradient bar with `2:34 / 4:10` (reuses `gradient_meter_spans`; `--:--` until mpv reports a duration).
- [x] **Friendly playback errors**: `ClimusicError::friendly()` translates common yt-dlp failure shapes (404/403/geo/not-available/timeout/empty-URL) into one plain sentence for the status bar, wired into the `run!` macro and the autoplay/page-load error paths.
- [x] **SoundCloud audio-URL caching** (10-minute TTL with prune-on-insert), same pattern as YouTube's — replaying a SoundCloud track no longer re-runs yt-dlp every time.
- [x] **Undo (`Ctrl+Z`)** for the one explicitly destructive action, playlist deletion: captured before delete, restored with all tracks and re-selected.
- [x] **Non-blocking search**: `run_search` (which froze the event loop for seconds) is now a background task (`start_search`/`poll_search`); newer searches abort in-flight ones, partial results come with a per-source failure summary, focus moves to Results when they land, and the status-bar spinner finally animates during searches. Also covers pasted-link resolves and the `-q` startup query.
- [x] **Genre-seeded SoundCloud browse** (cliamp notes): with no username configured, the category list/selector shows genre buckets (Trending, Hip-Hop, Electronic, House, Lo-Fi, Indie, Pop) backed by live `scsearch` — loads into the shared tracks pane with full list autoplay, on both the SoundCloud tab and the Dashboard (`←`/`→`).
- [x] **`cookies_from_browser` config key** (`[player]` section): threaded as `--cookies-from-browser` into every yt-dlp invocation (search/resolve/get_audio_url/user categories/genres) — unlocks private Likes and gated tracks via a logged-in browser session.
- [x] **Release packaging**: `[profile.release]` with LTO + codegen-units=1 + strip; `cargo build --release` produces `target/release/CLIWAVX.exe`.
- [x] **Small-terminal pass**: Library and SoundCloud collapse to the focused pane below 60 columns, Dashboard below 75; verified panic-free at 1x1 through 120x40 across all tabs plus help/prompt overlays via a TestBackend probe (deleted after use).

## Remaining next-steps round

- [x] **Queue persistence**: the queue is written to `queue.json` in the data dir (debounced — the tick loop saves only when it changed) and restored on startup. Verified E2E (probe added an entry, restarted the app, got it back; probe deleted, no residue in the user's queue file).
- [x] **Spotify playlist import**: pasting an `open.spotify.com/playlist/<id>` link into search fetches the playlist (name + up to 100 tracks, truncation noted) via the Web API, imports it as a library playlist, and lists the tracks in the results. Requires Spotify credentials (`S`).
- [x] **SoundCloud list caching**: loaded user categories and genre buckets are cached in memory for 10 minutes (keyed by username+category / genre, pruned on insert) — re-opening one no longer re-runs yt-dlp. `m` still pages forward from the cached state.
- Not done (environment-blocked): replacing the yt-dlp wrapper with a standalone exe — standalone binaries are blocked by Defender/SmartScreen on this machine; `tools/yt-dlp.cmd` + `yt_dlp_pkg/` remain the workaround.

## Stack

- Language: Rust (edition 2024)
- Player: mpv via Windows named-pipe IPC
- Stream resolver: yt-dlp (via Python wrapper in this environment)
- TUI: ratatui + crossterm
- Local metadata/indexing: SQLite (`rusqlite`) + `lofty`
- Spotify: Web API for search/playlist import only

## Key files

| File | Purpose |
|------|---------|
| `src/main.rs` | TUI event loop |
| `src/app.rs` | App state and actions |
| `src/player/mpv.rs` | mpv process + IPC client |
| `src/sources/youtube.rs` | YouTube search + audio URL extraction + cache |
| `src/sources/soundcloud.rs` | SoundCloud search, user Tracks/Likes/Reposts, direct-link resolve, audio URL extraction |
| `src/sources/spotify.rs` | Spotify Web API search |
| `src/sources/local.rs` | Local file scan |
| `src/db.rs` | SQLite playlists + local track index |
| `src/config.rs` | Config load/save |
| `examples/verify.rs` | Headless playback verifier (run with `cargo run --example verify`) |
| `tools/yt-dlp.cmd` | Windows wrapper for Python-based yt-dlp |

## Environment notes

- mpv installed at `C:/Program Files/MPV Player/mpv.exe`.
- Config lives at `%APPDATA%\climusic\climusic\config\config.toml`.
- yt-dlp standalone executables were blocked by Windows Defender/SmartScreen first-run scanning on this machine, so a wrapper is used:
  - `tools/yt-dlp.cmd` runs `python -m yt_dlp` from `yt_dlp_pkg/`.
  - Config: `yt_dlp_path = "C:/Users/Administrator/Desktop/DEV/CLI.wav/tools/yt-dlp.cmd"`.
  - On a normal machine you can point `yt_dlp_path` to any `yt-dlp.exe` on PATH.
- `yt_dlp_pkg/` and `tools/yt-dlp.cmd` are local workarounds; they are gitignored.

## Keybindings (from README)

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Switch tabs. Dashboard: toggle pane. Search: toggle typing vs. browsing results. Library/SoundCloud: toggle pane. |
| `1` `2` `3` `4` `5` `6` | Dashboard / Search / Queue / Library / SoundCloud / Now Playing |
| `↑` `↓` / `k` `j` | Navigate |
| `←` `→` (Dashboard) | Switch SoundCloud category (Tracks / Likes / Reposts), reloads |
| `Enter` (Search, typing) | Run search, switch focus to results |
| `Enter` (elsewhere) | Play selected track / load SoundCloud category |
| `a` | Add to queue |
| `p` | Add selected search track to current playlist |
| `Space` | Pause / resume |
| `n` | Next track |
| `n` (Library) | New playlist |
| `d` (Library) | Delete selected playlist |
| `+` / `-` | Volume up / down |
| `f` | Cycle source filter |
| `g` | Toggle game mode |
| `l` | Cycle repeat mode (Off → Track → All) |
| `x` | Toggle shuffle |
| `t` | Cycle accent palette (auto from artwork → teal → magenta → amber → violet) |
| `S` | Set up Spotify credentials |
| `C` | Set SoundCloud username |
| `r` | Rescan local library |
| `?` | Toggle in-app keybindings help (outside the search box) |
| `Q` / `Ctrl+Q` / `Ctrl+C` | Quit |

### Search focus (bugfix)

The Search tab now has two focus states, toggled with `Tab`, mirroring the
existing Library-pane-toggle pattern:

- **Input** (default): every printable key, including letters and spaces,
  is typed into the query. `Enter` runs the search and switches to Results.
- **Results**: the usual shortcuts apply (`a` add to queue, `p` add to
  playlist, `Enter` play, etc).

This was a real bug, not just a UX nit: previously *every* Search-tab
keystroke that matched a global shortcut (`n`, `f`, `g`, `r`, `l`, `x`,
`s`, `S`, `Space`, digits, `j`/`k`) fired that shortcut instead of typing
the character — Space couldn't be typed into a query at all. Combined
with the next bug, pasting something like a SoundCloud URL (which
contains `n`) would trigger `next_track()`, mpv would error because
nothing was queued, and that error used to propagate all the way out of
the event loop and silently exit the whole app.

### Fatal-error-on-keypress (bugfix)

`handle_key`/`handle_prompt_key` used to propagate any error from an
action (`?`) straight out of the event loop, which exited the whole TUI
on any transient mpv/yt-dlp/db failure — this is what looked like a
"crash" when pasting a link into search. All action calls now go through
a `run!` macro that converts an `Err` into a status-bar message instead.

## Verification results

`cargo run --example verify` successfully:

1. Starts mpv and sets volume.
2. Plays `test_audio.mp3` for 3 seconds.
3. Resolves and plays a YouTube audio URL for 5 seconds.
4. Resolves and plays a SoundCloud audio URL for 5 seconds.
5. Confirms the second YouTube URL lookup is served from cache in ~20–30 µs.

## Known warnings

None. `cargo check --all-targets` is clean.

## Dependency notes

- `ratatui-image` is pinned to **8.0.0** (not the latest 11.x) because 11.x
  requires `ratatui ^0.30.1` while this project is on `ratatui 0.29` (a
  0.29→0.30 upgrade would risk breaking every `ui/*.rs` file for a feature
  request that doesn't need it). 8.0.0 targets `ratatui ^0.29.0` exactly and
  has no native/pkg-config dependency (later versions gained an optional
  `chafa` C-library backend that 11.x enables by default and that isn't
  installed on this machine — not needed since sixel/kitty/iterm2/halfblock
  rendering are native to the crate regardless).
- On Windows, `Picker::from_query_stdio()` can't reliably detect terminal
  graphics capabilities, so `App::new()` falls back to
  `Picker::from_fontsize((8, 16))` (a guessed cell size) when it errors —
  this is the documented Windows path for this crate version. Actual
  rendering fidelity (crisp raster vs. blocky Unicode halfblocks) depends
  on what your terminal emulator supports; this hasn't been visually
  verified in this environment (no way to screenshot the running TUI here).
  What *is* verified end-to-end (via a throwaway `src/bin/artwork_check.rs`
  and `src/bin/oembed_check.rs`, both deleted after use): thumbnail URLs
  populate correctly (including via the oEmbed fallback for Likes/Reposts/
  Tracks entries), the image downloads and decodes, and `color-thief`
  extracts a usable palette from it.
- `color-thief = "0.2"` added for accent-color extraction. Pure Rust aside
  from `rgb` (also pure Rust); its `image` dependency is dev-only, not
  pulled into this build.

## Next possible steps

- Replace the yt-dlp wrapper with a proper dependency or signed binary once the environment allows it.
- Spotify playlist import currently fetches the first 100 tracks; paginate further if needed.
- Re-importing the same Spotify playlist appends duplicates (playlists are INSERT-OR-IGNORE by name); dedupe if that becomes annoying.
