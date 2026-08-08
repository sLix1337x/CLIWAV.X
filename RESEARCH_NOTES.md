# Research notes: EQ, visualizer, and architecture ideas

Working notes from a research pass done before implementing a 10-band EQ and a
terminal audio visualizer. Nothing here is copied from any other project —
these are our own conclusions, written to plan an independent implementation
against our own architecture (mpv as a persistent external process controlled
over JSON IPC; nothing decodes or touches raw audio in-process today, aside
from the one-shot waveform analysis added separately). Two general-purpose TUI
libraries (bubbletea and lipgloss, both Go) were also read for architectural
and styling ideas — not for audio-specific code, just general TUI design
patterns, described here in our own words for our own Rust/ratatui codebase.

## 1. Ten-band EQ

**Recommendation: drive mpv's own filter chain over IPC. Don't write our own
DSP.** Unlike a player that decodes audio itself in-process, we don't own the
sample stream — mpv does, in its own process. That's actually the easier
position to be in here: mpv (via ffmpeg's `libavfilter`) already ships
proper, tested biquad EQ filters, so there's no reason to hand-roll our own
IIR filter math when the player we already run natively supports this.

**Mechanism**: mpv's `--af` audio-filter option supports an `lavfi` wrapper
that gives access to any `libavfilter` filter, including `equalizer`,
`anequalizer`, and `superequalizer`. `anequalizer` looks like the right fit
for a *10-band* EQ specifically — it takes an explicit list of bands (each
with channel, frequency, width, gain, filter type) rather than a fixed preset
band count, so we can define our own 10 center frequencies.

**Runtime adjustment**: ffmpeg filters that support live parameter changes do
so via a filter "command" mechanism (the general shape is
`<filter-name> <command> <argument>`, sent through mpv's `af-command <label>
<command> <argument>` input command over IPC). `anequalizer`'s documented
command syntax for changing an existing band looks like
`fN|f=<freq>|w=<width>|g=<gain>` (change band N's frequency/width/gain) —
this needs a final empirical check against a live mpv instance before we
build on it (the primary ffmpeg docs site was unreachable during this
research pass; the above is reconstructed from secondary sources and mpv's
own `af.rst`, which confirms the general `af-command` mechanism exists via a
different filter's example, but didn't give us `anequalizer`'s exact command
name in front of us directly). **First implementation step: verify this
against a real mpv process** (add the filter with a label via `af-add`, then
try `af-command` with a few argument shapes and see what mpv accepts) rather
than assuming the syntax above is exactly right.

**Suggested design**:
- `PlayerConfig`-style new fields or a dedicated `EqConfig`: 10 bands, each
  `{ freq: f64, gain_db: f64 }` — frequencies fixed (not user-editable),
  gains adjustable ±12 dB (a common, sane range).
- A handful of built-in presets (Flat, Bass Boost, Treble Boost, Vocal, Rock,
  Pop, ...) as a plain Rust array of `[f64; 10]` gain sets, matched against
  our chosen frequency list — cheap to define, no config-file format needed
  for the built-ins. Custom user adjustment away from a preset just flips a
  "Custom" indicator (mirrors how loop mode / accent palette already track a
  small enum of named states in `App`).
- On startup (or track load), send one `af-add` with all 10 `anequalizer`
  bands at the current gains (0 dB = inaudible passthrough, so an unused EQ
  costs nothing extra). Band/preset changes go out as `af-command` deltas,
  not a full filter reload — reloading the filter chain on every knob turn
  would likely produce an audible click/gap.
- Debounce persistence to `config.toml` (e.g. 500ms–1s after the last
  change) rather than writing on every keypress while someone is tuning a
  band — a UI can feel instant while still not thrashing disk I/O; flush any
  pending write on quit/track-change so nothing gets lost.
- UI: a dedicated small panel (new tab, or a modal like the existing
  keybindings help overlay) showing 10 vertical sliders/bars with the
  current gain, current preset name (or "Custom"), navigable with existing
  vim-style keys.

## 2. Visualizer

**The core problem**: mpv doesn't expose a clean "give me the current
spectrum/levels" property over its JSON IPC. Audio-analysis filters like
`astats`/`showvolume` exist in ffmpeg but are designed to produce logged text
or video output, not a value we can poll cleanly from outside. Because
playback happens in mpv's own process, we can't just "tap the PCM stream"
the way an in-process player could.

**Recommendation: capture system audio output independently via WASAPI
loopback, instead of trying to extract data from mpv/ffmpeg's filter
graph.** This is the standard technique real terminal audio visualizers use
on Windows, and it has a real advantage for us specifically: it captures
whatever is *actually* coming out of the speakers, decoupled entirely from
which source is playing (local/YouTube/SoundCloud all end up as the same
system audio output) and unaffected by whatever mpv/ffmpeg internals do
between now and any future player change.

- Rust crate: `wasapi` (HEnquist/wasapi-rs, actively maintained, MSRV 1.76,
  already has a documented loopback-capture example) is the natural choice —
  safe wrapper over the raw WASAPI COM interfaces (`IAudioClient` opened with
  the loopback flag), no need to hand-roll COM/FFI ourselves the way the
  Job-object fix did for a much smaller Win32 surface.
- This is a genuinely bigger addition than the EQ: a new native Windows
  dependency, a background capture thread feeding a lock-free ring buffer
  (single writer = capture thread, single reader = UI/analysis thread — an
  `AtomicUsize` write cursor is enough, no mutex needed on the hot path),
  and our own FFT.
- FFT: rather than hand-rolling a Cooley-Tukey implementation, a small pure-Rust
  FFT crate (e.g. `rustfft`) is the pragmatic choice — this isn't a place
  where reinventing the wheel buys us anything, and a maintained crate will
  be more correct and likely faster than a first attempt at one.
- Band mapping: 10ish bars spread **logarithmically** across 20 Hz–20 kHz
  (perceptually, humans hear pitch logarithmically, so a linear FFT-bin split
  looks bass-heavy/treble-empty on screen) — average FFT bin power within
  each band's frequency range, convert to a dB-like scale, normalize/clamp to
  a 0–1 range for rendering.
- **Decouple analysis rate from render rate.** Running a full FFT at 60fps is
  wasteful when the terminal itself is only usefully redrawing a few times a
  second in a plain SSH/RDP-friendly TUI; analyze at a lower fixed rate
  (e.g. 20–30 Hz) and smooth/ease the rendered bar heights between updates.
  This single idea (decoupling "how often do we recompute" from "how often
  do we redraw") is probably the highest-leverage performance choice for a
  visualizer, independent of any particular smoothing formula.
- **Fast-attack/slow-decay smoothing per bar** (ease toward a rising target
  quickly, fall back down slowly) is what makes a bar-chart spectrum read as
  musical instead of flickery — a simple per-frame exponential blend with two
  different weights depending on whether the new value is higher or lower
  than the current one.
- **Skip the FFT entirely during silence** (a cheap max-abs scan of the
  latest capture buffer before bothering to transform) — near-free check that
  avoids burning CPU while paused or between tracks, and just decays bars
  toward zero instead.
- Rendering: reuse the eighth-block Unicode approach already built for the
  waveform (`src/ui/waveform.rs`'s `EIGHTHS` table) for bar-chart-style
  meters — the sub-row-precision technique generalizes directly to a
  spectrum display, no new rendering primitive needed.
- Reasonable v1 scope: one visualizer mode (a mirrored or single-direction
  bar spectrum), not a whole gallery of modes — can always grow later once
  the WASAPI capture + FFT + smoothing plumbing exists, since that's the
  expensive part to build once, cheap to reuse per additional mode.
- This should be an **opt-in** feature (a keybinding to toggle it on/off, off
  by default, maybe a config flag too) — WASAPI loopback capture plus a
  continuous FFT is real, constant background CPU/battery cost that the
  project's own "low overhead, stay out of the way while gaming" positioning
  argues against forcing on unconditionally.

## 3. Architecture: message-passing instead of poll-every-tick

Our current async pattern (search, artwork, waveform) is: spawn a
`tokio::task`, store its `JoinHandle` in an `App` field, and every tick call
a `poll_*` method that does `handle.is_finished()` + `now_or_never()` to drain
it into a plain `Option<T>` state field. It works, but every new async
feature needs its own new field(s) + its own new `poll_*` call wired into the
fixed tick sequence in `main.rs` — linear boilerplate growth, and a real bug
class ("did I remember to add the new poll call in the right place").

A cleaner shape worth considering for the EQ/visualizer work (and anything
async added after it): one `enum AppMsg { SearchDone(...), ArtworkDone(...),
WaveformDone(...), EqCommandAcked(...), ... }`, an `mpsc::UnboundedSender<AppMsg>`
cloned into every spawned task, and each task calls `.send()` on completion
instead of writing into a polled field. The main loop becomes "select over
crossterm key events and this one channel," feeding both into a single
`update(msg, app)` dispatch point. Concretely this would:
- Remove the `poll_*` fan-out — nothing to add per new async feature except
  one new enum variant, not a new field + a new call site.
- Cut latency from "next tick" to "as soon as the message arrives," since the
  channel isn't bound to the fixed 250ms/1s tick rate the way current polling is.
- Give us one place (the `update` function) to reason about "what happens
  when X finishes" instead of it being spread across N separate `poll_*`
  method bodies.

This is a genuine refactor, not a quick win — not a prerequisite for the
EQ/visualizer work, but worth doing *before* adding several more async
sources of state (EQ apply confirmations, visualizer frame data) if we want
to avoid the `poll_*` list growing even longer. Could be introduced
incrementally: add the channel and `AppMsg` enum, migrate one existing
`poll_*` (e.g. artwork, the simplest one) as a proof of concept, then decide
whether to migrate the rest or leave them be.

Two smaller, lower-risk ideas from the same research, independent of the
message-passing question:
- **Separate "what key was pressed" from "what it means."** `handle_key` in
  `main.rs` is one large match on `KeyCode` that both interprets keys and
  calls `App` methods inline. Splitting that into a small per-mode "resolve
  key to an `Action` enum" step, then a second dispatcher that matches
  `Action -> App` method, would make "what does key X do in state Y"
  testable without a terminal, and shrinks one large match into two smaller
  ones. Worth doing opportunistically (e.g. when touching key handling for
  the new EQ panel) rather than as a standalone refactor.
- **Extract reusable sub-components for list navigation and search input.**
  Several tabs (Search, Library, SoundCloud, Dashboard) each hand-roll
  similar "selected index + scroll + up/down navigation" and "text input with
  cursor" logic independently. A small `ListPane` / `SearchInput` struct
  (own state, own `handle_key`, own `render`) that each tab just owns an
  instance of would cut duplication — an incremental extraction, not a
  rewrite, and a natural fit if/when the EQ panel needs its own list-like
  band selector.

## 4. Styling: a thin theme layer, and better gradients

Our `src/ui/mod.rs` already has color helpers (`lerp_rgb`, `brighten_rgb`,
`dim_rgb`, `gradient_meter_spans`) but no shared "style presets" — every
`ui/*.rs` module hand-builds `Block::default().borders(Borders::ALL)
.border_type(BorderType::Rounded).border_style(Style::default().fg(color))`
inline, repeated with slight variations at many call sites.

Two concrete, low-risk ideas:
- **A small `theme.rs` of named, reusable style-building functions** — e.g.
  `fn panel_border(accent: Color) -> Block`, `fn muted_label() -> Style`,
  `fn accent_value() -> Style` — one definition per visual role instead of
  retyping the same `Block`/`Style` construction at every call site. Ratatui's
  `Style` already has a `.patch()` method (overlay only the fields explicitly
  set on the patch, leave the rest) that gives us most of what a
  "base style + variant" pattern needs, without introducing any new
  abstraction — it's already there, just underused.
- **Interpolate gradients in a perceptual color space, not raw RGB.** Our
  `lerp_rgb` almost certainly lerps straight in RGB, which tends to produce a
  muddy/grey dip partway through a gradient between two saturated colors
  (e.g. cyan → magenta passing through a dull grey) and uneven-looking
  brightness steps. Interpolating via HSL (or a proper Lab space, if we pull
  in a small color crate like `palette`) avoids that — worth doing for the
  EQ panel's band-gain bars and any new visualizer gradient coloring, since
  both will lean on this helper a lot and visual quality matters more there
  than for the existing thin meter bars.

No changes suggested to layout/`Layout::vertical(...).split(...)` boilerplate
— that pattern exists to solve a different problem for us (computing `Rect`s
for widgets that render straight into a shared buffer) than it does for a
string-composition-based renderer, so there's no real win porting that idea
over; if the repeated layout boilerplate becomes a real pain point later, the
right fix is small ratatui-native helpers like `fn split_header_body(area) ->
(Rect, Rect)` for our own common panel shapes, not a borrowed abstraction.

## 5. Suggested priority

1. **EQ first** — smaller, self-contained, and the mechanism (drive mpv's
   existing filter support over IPC) is already well understood; the main
   remaining unknown is the exact `af-command` argument syntax, which needs
   one quick empirical check against a live mpv process before writing the
   real implementation.
2. **Visualizer second** — bigger lift (new native dependency, a capture
   thread, an FFT pipeline), but the waveform feature already gave us the
   Unicode block-rendering building block, and 20–30 Hz analysis + eased
   rendering keeps it cheap once it exists.
3. **Message-passing refactor** — worth doing before or alongside the
   visualizer if we want to avoid a fourth (and fifth, ...) `poll_*` method
   piling onto the existing pattern; otherwise fine to defer.
4. **Theme/gradient polish** — lowest urgency, but cheap, and worth folding
   into whichever of the above touches styling first (the EQ panel's band
   bars are a natural first beneficiary of better gradient interpolation).
