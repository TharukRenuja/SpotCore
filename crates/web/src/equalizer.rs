//! Six-band Spotify EQ presets. Gains are decibels, ordered by ascending frequency.
use serde::{Deserialize, Serialize};

/// Spotify's built-in desktop curves, independent of the receiver's enabled state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EqualizerPreset {
    Flat,
    Acoustic,
    BassBooster,
    BassReducer,
    Classical,
    Dance,
    Deep,
    Electronic,
    HipHop,
    Jazz,
    Latin,
    Loudness,
    Lounge,
    Piano,
    Pop,
    Rnb,
    Rock,
    SmallSpeakers,
    SpokenWord,
    TrebleBooster,
    TrebleReducer,
    VocalBooster,
}

impl EqualizerPreset {
    pub const ALL: [Self; 22] = [
        Self::Flat,
        Self::Acoustic,
        Self::BassBooster,
        Self::BassReducer,
        Self::Classical,
        Self::Dance,
        Self::Deep,
        Self::Electronic,
        Self::HipHop,
        Self::Jazz,
        Self::Latin,
        Self::Loudness,
        Self::Lounge,
        Self::Piano,
        Self::Pop,
        Self::Rnb,
        Self::Rock,
        Self::SmallSpeakers,
        Self::SpokenWord,
        Self::TrebleBooster,
        Self::TrebleReducer,
        Self::VocalBooster,
    ];

    /// 60, 150, 400, 1000, 2400 and 15000 Hz. Range: -12 through +12 dB.
    pub const fn gains(self) -> [f64; 6] {
        match self {
            Self::Flat => [0.; 6],
            Self::Acoustic => [4.9, 3.95, 2.15, 1.75, 3.5, 2.15],
            Self::BassBooster => [4.25, 3.5, 1.25, 0., 0., 0.],
            Self::BassReducer => [-4.25, -3.5, -1.25, 0., 0., 0.],
            Self::Classical => [3.75, 3., -1.5, -1.5, 0., 3.75],
            Self::Dance => [6.55, 4.99, 1.92, 3.65, 5.15, 0.],
            Self::Deep => [3.55, 1.75, 2.85, 2.5, 1.45, -2.15],
            Self::Electronic => [3.8, 1.2, -2.15, 2.25, 0.85, 4.8],
            Self::HipHop => [4.25, 1.5, -1., -1., 1.5, 3.],
            Self::Jazz => [3., 1.5, -1.5, -1.5, 0., 3.75],
            Self::Latin => [3., 0., -1.5, -1.5, -1.5, 4.5],
            Self::Loudness => [4., 0., -2., 0., -1., 1.],
            Self::Lounge => [-1.5, -0.5, 4., 2.5, 0., 1.],
            Self::Piano => [2., 0., 3., 1.5, 3.5, 3.5],
            Self::Pop => [-1., 0., 4., 4., 2., -1.5],
            Self::Rnb => [6.92, 5.65, -2.19, -1.5, 2.32, 3.75],
            Self::Rock => [4., 3., -0.5, -1., 0.5, 4.5],
            Self::SmallSpeakers => [4.25, 3.5, 1.25, 0., -1.25, -4.25],
            Self::SpokenWord => [-0.47, 0., 3.46, 4.61, 4.84, 0.],
            Self::TrebleBooster => [0., 0., 0., 1.25, 2.5, 5.5],
            Self::TrebleReducer => [0., 0., 0., -1.25, -2.5, -5.5],
            Self::VocalBooster => [-3., -3., 3.75, 3.75, 3., -1.5],
        }
    }

    /// Recognizes a stored curve. `None` represents Manual, not missing support.
    /// Spotify does not persist a preset identity; a matching manual curve is indistinguishable.
    pub fn from_gains(gains: [f64; 6]) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| {
            gains
                .iter()
                .zip(preset.gains())
                .all(|(a, b)| a.is_finite() && (a - b).abs() <= 0.001)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curves_round_trip_and_a_manual_edit_is_not_a_preset() {
        for preset in EqualizerPreset::ALL {
            let gains = preset.gains();
            assert!(gains.into_iter().all(|g| (-12.0..=12.0).contains(&g)));
            assert_eq!(EqualizerPreset::from_gains(gains), Some(preset));
        }
        let mut gains = EqualizerPreset::BassBooster.gains();
        gains[2] += 0.5;
        assert_eq!(EqualizerPreset::from_gains(gains), None);
        assert_eq!(EqualizerPreset::from_gains([f64::NAN; 6]), None);
        assert_eq!(EqualizerPreset::from_gains([f64::INFINITY; 6]), None);
    }
}
