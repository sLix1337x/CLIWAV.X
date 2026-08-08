/// 10-band EQ driven entirely through mpv's own `af` audio-filter chain over
/// IPC — see `MpvPlayer::set_eq`. No in-process DSP: mpv (via ffmpeg's
/// libavfilter) already ships a proper peaking-biquad `equalizer` filter, so
/// each band is just one chained instance of it.
pub const BAND_COUNT: usize = 10;

/// Classic doubling-octave 10-band spacing (31 Hz to 16 kHz), the same
/// layout long used by consumer graphic EQs.
pub const FREQUENCIES: [u32; BAND_COUNT] =
    [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

pub const MAX_GAIN_DB: f64 = 12.0;

/// Q factor shared by every band — a single, fairly wide setting rather than
/// per-band tuning, matching how consumer 10-band EQs behave.
const BAND_Q: f64 = 1.4;

/// mpv `af` filter label used for the whole chain — added/replaced as one
/// unit via `af set` rather than tracked per band.
pub const FILTER_LABEL: &str = "cliwavx_eq";

pub type Gains = [f64; BAND_COUNT];

pub const FLAT: Gains = [0.0; BAND_COUNT];

pub struct Preset {
    pub name: &'static str,
    pub gains: Gains,
}

pub const PRESETS: &[Preset] = &[
    Preset { name: "Flat", gains: FLAT },
    Preset { name: "Bass Boost", gains: [7.0, 6.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] },
    Preset { name: "Treble Boost", gains: [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 5.0, 6.0, 7.0] },
    Preset { name: "Vocal", gains: [-2.0, -2.0, -1.0, 1.0, 3.0, 3.0, 2.0, 1.0, 0.0, -1.0] },
    Preset { name: "Rock", gains: [5.0, 3.0, -2.0, -3.0, -1.0, 2.0, 4.0, 5.0, 5.0, 5.0] },
    Preset { name: "Pop", gains: [-1.0, 1.0, 3.0, 3.0, 1.0, -1.0, -2.0, -2.0, -1.0, -1.0] },
    Preset { name: "Electronic", gains: [6.0, 5.0, 1.0, 0.0, -2.0, 1.0, 0.0, 1.0, 4.0, 5.0] },
    Preset { name: "Loudness", gains: [6.0, 4.0, 0.0, -1.0, -1.0, 0.0, 0.0, 2.0, 5.0, 6.0] },
];

/// Builds the `af set` filter-graph value: 10 chained `equalizer` biquads,
/// one per band, comma-chained inside a single `lavfi` wrapper so the whole
/// chain is added/replaced as one filter in mpv's `af` list. Gains are
/// clamped to `MAX_GAIN_DB` here so a caller can't accidentally push an
/// out-of-range value into mpv.
pub fn build_af_graph(gains: &Gains) -> String {
    let chain = FREQUENCIES
        .iter()
        .zip(gains.iter())
        .map(|(freq, gain)| {
            let g = gain.clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
            format!("equalizer=f={freq}:width_type=q:w={BAND_Q}:g={g:.2}")
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("@{FILTER_LABEL}:lavfi=[{chain}]")
}

/// Converts a persisted gain list (a plain `Vec<f64>` in config.toml, so a
/// hand-edited or older-version file can have the wrong length) into a
/// fixed-size, clamped array — missing entries default to 0 dB, extra ones
/// are dropped.
pub fn gains_from_slice(values: &[f64]) -> Gains {
    let mut gains = FLAT;
    for (slot, v) in gains.iter_mut().zip(values.iter()) {
        *slot = v.clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
    }
    gains
}

/// Whether `gains` matches a preset exactly (used to decide whether the UI
/// shows a preset name or "Custom" after a manual band edit).
pub fn matching_preset(gains: &Gains) -> Option<&'static str> {
    PRESETS
        .iter()
        .find(|p| p.gains == *gains)
        .map(|p| p.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_graph_has_ten_chained_bands() {
        let graph = build_af_graph(&FLAT);
        assert_eq!(graph.matches("equalizer=").count(), BAND_COUNT);
        assert!(graph.starts_with(&format!("@{FILTER_LABEL}:lavfi=[")));
        assert!(graph.ends_with(']'));
    }

    #[test]
    fn gains_are_clamped_into_the_graph() {
        let mut gains = FLAT;
        gains[0] = 999.0;
        gains[1] = -999.0;
        let graph = build_af_graph(&gains);
        assert!(graph.contains(&format!("g={:.2}", MAX_GAIN_DB)));
        assert!(graph.contains(&format!("g={:.2}", -MAX_GAIN_DB)));
    }

    #[test]
    fn flat_matches_the_flat_preset() {
        assert_eq!(matching_preset(&FLAT), Some("Flat"));
    }

    #[test]
    fn short_slice_pads_with_zero() {
        let gains = gains_from_slice(&[1.0, 2.0]);
        assert_eq!(gains[0], 1.0);
        assert_eq!(gains[1], 2.0);
        assert_eq!(gains[2], 0.0);
    }

    #[test]
    fn long_slice_is_truncated() {
        let values = vec![1.0; BAND_COUNT + 5];
        let gains = gains_from_slice(&values);
        assert_eq!(gains.len(), BAND_COUNT);
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let gains = gains_from_slice(&[999.0, -999.0]);
        assert_eq!(gains[0], MAX_GAIN_DB);
        assert_eq!(gains[1], -MAX_GAIN_DB);
    }

    #[test]
    fn arbitrary_gains_match_no_preset() {
        let mut gains = FLAT;
        gains[3] = 1.23;
        assert_eq!(matching_preset(&gains), None);
    }
}
