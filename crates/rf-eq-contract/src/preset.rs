//! Factory settings.
//!
//! A preset here is nothing but a list of parameter values. The packager
//! renders the same table into `metadata/presets.json`, so the catalog
//! RackForge shows and the settings the engine loads cannot disagree.

use crate::Settings;
use crate::index::*;

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub values: &'static [(u32, f32)],
}

pub const PRESET_COUNT: usize = 6;

pub const PRESETS: [Preset; PRESET_COUNT] = [
    Preset {
        id: "flat",
        name: "Flat",
        description: "Every band at zero, the high-pass off: the signal as it came.",
        values: &[],
    },
    Preset {
        id: "piano_air",
        name: "Piano Air",
        description: "Two and a half decibels of air above eight kilohertz, a decibel less low shelf at 120 Hz.",
        values: &[
            (HIGH_SHELF_FREQUENCY, 8_000.0),
            (HIGH_SHELF_GAIN, 2.5),
            (LOW_SHELF_FREQUENCY, 120.0),
            (LOW_SHELF_GAIN, -1.0),
        ],
    },
    Preset {
        id: "warm",
        name: "Warm",
        description: "Two decibels more below 150 Hz, two less above six kilohertz.",
        values: &[
            (LOW_SHELF_FREQUENCY, 150.0),
            (LOW_SHELF_GAIN, 2.0),
            (HIGH_SHELF_FREQUENCY, 6_000.0),
            (HIGH_SHELF_GAIN, -2.0),
        ],
    },
    Preset {
        id: "presence",
        name: "Presence",
        description: "Three decibels at three kilohertz, a little over an octave wide.",
        values: &[
            (PEAK2_FREQUENCY, 3_000.0),
            (PEAK2_GAIN, 3.0),
            (PEAK2_Q, 1.2),
        ],
    },
    Preset {
        id: "rumble_cut",
        name: "Rumble Cut",
        description: "The high-pass on at 60 Hz: stage rumble and handling noise out, the low end kept.",
        values: &[(HPF_ENABLE, 1.0), (HPF_FREQUENCY, 60.0)],
    },
    Preset {
        id: "telephone",
        name: "Telephone",
        description: "Nothing below 300 Hz, twelve decibels less above three kilohertz: a narrow line.",
        values: &[
            (HPF_ENABLE, 1.0),
            (HPF_FREQUENCY, 300.0),
            (HIGH_SHELF_FREQUENCY, 3_000.0),
            (HIGH_SHELF_GAIN, -12.0),
        ],
    },
];

/// The settings a preset describes: the defaults, with the preset's values
/// over them. `None` for an id no preset carries.
pub fn settings_for(id: &str) -> Option<Settings> {
    let preset = PRESETS.iter().find(|preset| preset.id == id)?;
    let mut settings = Settings::default();
    for (index, value) in preset.values {
        if !settings.set(*index, *value as f64) {
            return None;
        }
    }
    Some(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_loads_and_stays_inside_the_contract() {
        for preset in PRESETS.iter() {
            assert!(
                settings_for(preset.id).is_some(),
                "{} does not load",
                preset.id
            );
        }
        assert!(settings_for("nowhere").is_none());
    }

    #[test]
    fn preset_identifiers_are_unique() {
        for (position, preset) in PRESETS.iter().enumerate() {
            for other in &PRESETS[position + 1..] {
                assert_ne!(preset.id, other.id);
            }
        }
    }

    #[test]
    fn flat_is_the_defaults() {
        let flat = settings_for("flat").expect("flat loads");
        assert_eq!(flat.as_array(), Settings::default().as_array());
    }
}
