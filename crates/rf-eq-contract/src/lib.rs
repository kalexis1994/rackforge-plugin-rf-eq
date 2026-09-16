//! The public contract of RF-EQ: every parameter the host can see and the
//! flat settings block that becomes plugin state.
//!
//! Nothing here performs audio work. The engine reads this table, the packager
//! renders `metadata/parameters.json` from it, and the web surface receives the
//! same schema back from the host, so the three can never drift apart.

#![no_std]

pub mod index;
pub mod preset;

pub use preset::{PRESET_COUNT, PRESETS, Preset, settings_for};

/// Number of public parameters. Also the length of the state block in `f32`s.
pub const PARAMETER_COUNT: usize = 22;

/// The parameter count of each earlier state layout, so a block saved by an
/// older build can still be read. Length is the only thing that identifies a
/// layout here, which is why parameters are only ever appended.
pub const PREVIOUS_PARAMETER_COUNTS: [usize; 1] = [14];

/// Editor pages. RackForge renders them in `order`; the web surface uses the
/// same identifiers to group its controls.
pub struct PageSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub order: i32,
}

pub const PAGES: [PageSpec; 2] = [
    PageSpec {
        id: "filters",
        name: "Filters",
        order: 0,
    },
    PageSpec {
        id: "output",
        name: "Output",
        order: 1,
    },
];

/// How a continuous control travels. Logarithmic needs a minimum above zero.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Taper {
    Linear,
    Logarithmic,
}

/// The parameter kinds the contract can express: a subset of the RackForge
/// parameter schema. RF-EQ uses floats and booleans; the enum and meter kinds
/// stay so the packager renders them correctly should a band ever grow one.
/// A meter is read-only and reports what the engine is doing.
#[derive(Clone, Copy)]
pub enum Kind {
    Float {
        minimum: f32,
        maximum: f32,
        default: f32,
        step: f32,
        unit: Option<&'static str>,
        taper: Taper,
    },
    Boolean {
        default: bool,
    },
    Enum {
        default: u32,
        choices: &'static [&'static str],
    },
    Meter {
        minimum: f32,
        maximum: f32,
        unit: Option<&'static str>,
    },
}

/// Control hint published to RackForge surfaces (LITTLE, controller mappings).
#[derive(Clone, Copy)]
pub enum Control {
    Knob,
    Toggle,
    List,
    Meter,
}

impl Control {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Knob => "knob",
            Self::Toggle => "toggle",
            Self::List => "list",
            Self::Meter => "meter",
        }
    }
}

pub struct ParameterSpec {
    pub index: u32,
    pub id: &'static str,
    pub name: &'static str,
    pub page: &'static str,
    pub order: i32,
    pub kind: Kind,
    pub control: Control,
}

impl ParameterSpec {
    pub const fn default_value(&self) -> f32 {
        match self.kind {
            Kind::Float { default, .. } => default,
            Kind::Boolean { default } => {
                if default {
                    1.0
                } else {
                    0.0
                }
            }
            Kind::Enum { default, .. } => default as f32,
            Kind::Meter { maximum, .. } => maximum,
        }
    }

    pub const fn is_meter(&self) -> bool {
        matches!(self.kind, Kind::Meter { .. })
    }

    /// Accepts a host value and returns the canonical stored value, or `None`
    /// when the value is outside the declared contract. RackForge validates
    /// first; this is the engine's own guard so a broken caller cannot poison
    /// the audio thread.
    pub fn canonicalize(&self, value: f64) -> Option<f32> {
        if !value.is_finite() {
            return None;
        }
        match self.kind {
            Kind::Float {
                minimum, maximum, ..
            }
            | Kind::Meter {
                minimum, maximum, ..
            } => {
                let value = value as f32;
                (value >= minimum && value <= maximum).then_some(value)
            }
            Kind::Boolean { .. } => {
                if value == 0.0 {
                    Some(0.0)
                } else if value == 1.0 {
                    Some(1.0)
                } else {
                    None
                }
            }
            Kind::Enum { choices, .. } => {
                let rounded = value as i64;
                if value != rounded as f64 || rounded < 0 {
                    return None;
                }
                (rounded < choices.len() as i64).then_some(rounded as f32)
            }
        }
    }
}

/// A frequency in hertz on the Filters page, travelling logarithmically, to
/// the nearest hertz.
const fn hertz(
    index: u32,
    id: &'static str,
    name: &'static str,
    order: i32,
    minimum: f32,
    maximum: f32,
    default: f32,
) -> ParameterSpec {
    ParameterSpec {
        index,
        id,
        name,
        page: "filters",
        order,
        kind: Kind::Float {
            minimum,
            maximum,
            default,
            step: 1.0,
            unit: Some("Hz"),
            taper: Taper::Logarithmic,
        },
        control: Control::Knob,
    }
}

/// A gain in decibels, to a tenth, at rest at zero.
const fn decibels(
    index: u32,
    id: &'static str,
    name: &'static str,
    page: &'static str,
    order: i32,
    minimum: f32,
    maximum: f32,
) -> ParameterSpec {
    ParameterSpec {
        index,
        id,
        name,
        page,
        order,
        kind: Kind::Float {
            minimum,
            maximum,
            default: 0.0,
            step: 0.1,
            unit: Some("dB"),
            taper: Taper::Linear,
        },
        control: Control::Knob,
    }
}

/// A peak band's width as a quality factor, from a bell more than two
/// octaves wide to a narrow notch, travelling logarithmically.
const fn quality(index: u32, id: &'static str, name: &'static str, order: i32) -> ParameterSpec {
    ParameterSpec {
        index,
        id,
        name,
        page: "filters",
        order,
        kind: Kind::Float {
            minimum: 0.3,
            maximum: 10.0,
            default: 1.0,
            step: 0.01,
            unit: None,
            taper: Taper::Logarithmic,
        },
        control: Control::Knob,
    }
}

const fn switch(
    index: u32,
    id: &'static str,
    name: &'static str,
    page: &'static str,
    order: i32,
    default: bool,
) -> ParameterSpec {
    ParameterSpec {
        index,
        id,
        name,
        page,
        order,
        kind: Kind::Boolean { default },
        control: Control::Toggle,
    }
}

/// The furthest a band's gain travels, either way.
pub const BAND_GAIN_DB: f32 = 15.0;

pub const PARAMETERS: [ParameterSpec; PARAMETER_COUNT] = [
    switch(0, "hpf.enable", "High-pass", "filters", 0, false),
    hertz(1, "hpf.frequency", "HPF Freq", 1, 20.0, 500.0, 40.0),
    hertz(
        2,
        "low_shelf.frequency",
        "Low Freq",
        10,
        40.0,
        1_000.0,
        120.0,
    ),
    decibels(
        3,
        "low_shelf.gain",
        "Low Gain",
        "filters",
        11,
        -BAND_GAIN_DB,
        BAND_GAIN_DB,
    ),
    hertz(
        4,
        "peak1.frequency",
        "Peak 1 Freq",
        20,
        100.0,
        8_000.0,
        500.0,
    ),
    decibels(
        5,
        "peak1.gain",
        "Peak 1 Gain",
        "filters",
        21,
        -BAND_GAIN_DB,
        BAND_GAIN_DB,
    ),
    quality(6, "peak1.q", "Peak 1 Q", 22),
    hertz(
        7,
        "peak2.frequency",
        "Peak 2 Freq",
        30,
        200.0,
        16_000.0,
        3_000.0,
    ),
    decibels(
        8,
        "peak2.gain",
        "Peak 2 Gain",
        "filters",
        31,
        -BAND_GAIN_DB,
        BAND_GAIN_DB,
    ),
    quality(9, "peak2.q", "Peak 2 Q", 32),
    hertz(
        10,
        "high_shelf.frequency",
        "High Freq",
        60,
        1_000.0,
        16_000.0,
        8_000.0,
    ),
    decibels(
        11,
        "high_shelf.gain",
        "High Gain",
        "filters",
        61,
        -BAND_GAIN_DB,
        BAND_GAIN_DB,
    ),
    decibels(12, "output.trim", "Output", "output", 0, -24.0, 24.0),
    switch(13, "output.bypass", "Bypass", "output", 1, false),
    hertz(
        14,
        "peak3.frequency",
        "Peak 3 Freq",
        40,
        300.0,
        20_000.0,
        6_000.0,
    ),
    decibels(
        15,
        "peak3.gain",
        "Peak 3 Gain",
        "filters",
        41,
        -BAND_GAIN_DB,
        BAND_GAIN_DB,
    ),
    quality(16, "peak3.q", "Peak 3 Q", 42),
    hertz(
        17,
        "peak4.frequency",
        "Peak 4 Freq",
        50,
        500.0,
        20_000.0,
        12_000.0,
    ),
    decibels(
        18,
        "peak4.gain",
        "Peak 4 Gain",
        "filters",
        51,
        -BAND_GAIN_DB,
        BAND_GAIN_DB,
    ),
    quality(19, "peak4.q", "Peak 4 Q", 52),
    switch(20, "lpf.enable", "Low-pass", "filters", 70, false),
    hertz(
        21,
        "lpf.frequency",
        "LPF Freq",
        71,
        500.0,
        20_000.0,
        20_000.0,
    ),
];

/// The flat settings block: one `f32` per parameter, in index order. It is
/// the plugin's state, so its layout is the contract's.
#[derive(Clone, Copy)]
pub struct Settings {
    values: [f32; PARAMETER_COUNT],
}

impl Default for Settings {
    fn default() -> Self {
        let mut values = [0.0_f32; PARAMETER_COUNT];
        let mut index = 0;
        while index < PARAMETER_COUNT {
            values[index] = PARAMETERS[index].default_value();
            index += 1;
        }
        Self { values }
    }
}

impl Settings {
    /// Sets a control. A meter is the engine's to write, never the host's.
    pub fn set(&mut self, index: u32, value: f64) -> bool {
        let Some(spec) = PARAMETERS.get(index as usize) else {
            return false;
        };
        if spec.is_meter() {
            return false;
        }
        let Some(canonical) = spec.canonicalize(value) else {
            return false;
        };
        self.values[index as usize] = canonical;
        true
    }

    pub fn get(&self, index: u32) -> Option<f64> {
        self.values.get(index as usize).map(|value| *value as f64)
    }

    pub fn value(&self, index: u32) -> f32 {
        self.values[index as usize]
    }

    pub fn engaged(&self, index: u32) -> bool {
        self.values[index as usize] >= 0.5
    }

    pub fn as_array(&self) -> [f32; PARAMETER_COUNT] {
        self.values
    }

    pub fn from_array(values: [f32; PARAMETER_COUNT]) -> Option<Self> {
        Self::from_slice(&values)
    }

    /// Reads a state block that may have been written by an earlier build.
    ///
    /// Layouts are identified by length, and parameters are only ever appended,
    /// so a shorter block is an older one: its values are applied and anything
    /// added since keeps its default. A length that belongs to no layout, or a
    /// value outside the contract, is refused outright rather than partially
    /// applied. A meter's stored value is ignored: it is the engine's reading,
    /// not a setting.
    pub fn from_slice(values: &[f32]) -> Option<Self> {
        if values.len() != PARAMETER_COUNT && !PREVIOUS_PARAMETER_COUNTS.contains(&values.len()) {
            return None;
        }
        let mut settings = Self::default();
        for (index, value) in values.iter().enumerate() {
            if PARAMETERS[index].is_meter() {
                continue;
            }
            if !settings.set(index as u32, *value as f64) {
                return None;
            }
        }
        Some(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_parameter_declares_its_own_index_and_a_known_page() {
        for (position, spec) in PARAMETERS.iter().enumerate() {
            assert_eq!(spec.index as usize, position, "{}", spec.id);
            assert!(
                PAGES.iter().any(|page| page.id == spec.page),
                "{} refers to an undeclared page",
                spec.id
            );
        }
    }

    #[test]
    fn parameter_identifiers_are_unique_and_host_legal() {
        for (position, spec) in PARAMETERS.iter().enumerate() {
            assert!(
                spec.id.bytes().all(|byte| byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || b".-_".contains(&byte)),
                "{} is not a legal RackForge identifier",
                spec.id
            );
            for other in &PARAMETERS[position + 1..] {
                assert_ne!(spec.id, other.id);
            }
        }
    }

    #[test]
    fn a_logarithmic_taper_never_starts_at_zero() {
        for spec in PARAMETERS.iter() {
            if let Kind::Float {
                taper: Taper::Logarithmic,
                minimum,
                ..
            } = spec.kind
            {
                assert!(
                    minimum > 0.0,
                    "{} tapers logarithmically from zero",
                    spec.id
                );
            }
        }
    }

    #[test]
    fn every_default_sits_inside_its_range() {
        for spec in PARAMETERS.iter() {
            assert!(
                spec.canonicalize(f64::from(spec.default_value())).is_some(),
                "{} defaults outside its range",
                spec.id
            );
        }
    }

    #[test]
    fn defaults_round_trip_through_validation() {
        let settings = Settings::default();
        let restored = Settings::from_array(settings.as_array()).expect("defaults are valid");
        assert_eq!(settings.as_array(), restored.as_array());
    }

    #[test]
    fn a_value_outside_the_contract_is_refused() {
        let mut settings = Settings::default();
        assert!(settings.set(index::PEAK1_GAIN, -15.0));
        assert!(!settings.set(index::PEAK1_GAIN, 15.1));
        assert!(!settings.set(index::HPF_FREQUENCY, 10.0));
        assert!(settings.set(index::HPF_FREQUENCY, 500.0));
        assert!(!settings.set(index::BYPASS, 0.5));
        assert!(settings.set(index::BYPASS, 1.0));
        assert!(!settings.set(index::PEAK2_Q, f64::NAN));
        assert!(!settings.set(PARAMETER_COUNT as u32, 0.0));
    }

    #[test]
    fn named_indexes_point_at_the_identifiers_they_claim() {
        let expect = |index: u32, id: &str| assert_eq!(PARAMETERS[index as usize].id, id);
        expect(index::HPF_ENABLE, "hpf.enable");
        expect(index::HPF_FREQUENCY, "hpf.frequency");
        expect(index::LOW_SHELF_FREQUENCY, "low_shelf.frequency");
        expect(index::LOW_SHELF_GAIN, "low_shelf.gain");
        expect(index::PEAK1_FREQUENCY, "peak1.frequency");
        expect(index::PEAK1_GAIN, "peak1.gain");
        expect(index::PEAK1_Q, "peak1.q");
        expect(index::PEAK2_FREQUENCY, "peak2.frequency");
        expect(index::PEAK2_GAIN, "peak2.gain");
        expect(index::PEAK2_Q, "peak2.q");
        expect(index::HIGH_SHELF_FREQUENCY, "high_shelf.frequency");
        expect(index::HIGH_SHELF_GAIN, "high_shelf.gain");
        expect(index::OUTPUT, "output.trim");
        expect(index::BYPASS, "output.bypass");
        expect(index::PEAK3_FREQUENCY, "peak3.frequency");
        expect(index::PEAK3_GAIN, "peak3.gain");
        expect(index::PEAK3_Q, "peak3.q");
        expect(index::PEAK4_FREQUENCY, "peak4.frequency");
        expect(index::PEAK4_GAIN, "peak4.gain");
        expect(index::PEAK4_Q, "peak4.q");
        expect(index::LPF_ENABLE, "lpf.enable");
        expect(index::LPF_FREQUENCY, "lpf.frequency");
    }
}
