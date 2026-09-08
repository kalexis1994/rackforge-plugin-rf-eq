//! The equaliser: a high-pass, a low shelf, two peaks and a high shelf in
//! series, then a trim. One set of coefficients serves both channels.
//!
//! A knob does not land where it is put; it travels there. Every setting
//! has a target and a value on its way to it, moved by a one-pole ramp of a
//! few milliseconds, and the sections are redesigned from the travelling
//! values a few times a millisecond while anything is still moving. That is
//! what keeps automation from zippering, and it is the whole reason the
//! engine has a ramp per parameter rather than a coefficient per band.
//!
//! A band at zero decibels is not a filter at zero decibels; it is skipped,
//! and so is the high-pass when it is off. With the trim at zero, the
//! default state is the input bit for bit. Bypass is a hard switch that
//! returns the input, so a comparison against it is honest.

use rf_eq_contract::index::*;
use rf_eq_contract::{
    Kind, PARAMETER_COUNT, PARAMETERS, PREVIOUS_PARAMETER_COUNTS, Settings, Taper, preset,
};

use crate::biquad::{Biquad, Coefficients};
use crate::math::{abs, clamp, db_to_gain, one_pole, sanitise};

/// Length of the serialised state, in bytes.
pub const STATE_BYTES: usize = PARAMETER_COUNT * 4;

/// The ramp under every knob: one time constant. Five milliseconds is short
/// enough that a fader move feels immediate and long enough that a thirty
/// decibel jump arrives as a slope rather than a step.
const SMOOTHING_S: f32 = 0.005;

/// How many samples pass between redesigns while a value is travelling. Eight
/// at 48 kHz is a sixth of a millisecond: the coefficients step far more
/// finely than the ear can follow, at an eighth of the cost of every sample.
const DESIGN_INTERVAL: u32 = 8;

/// Nothing is designed above this fraction of the sample rate, where the
/// bilinear transform folds a band into a shape it was not asked for.
const NYQUIST_FRACTION: f32 = 0.45;

/// Where a ramp is close enough to its target to snap onto it: a thousandth
/// of a decibel, or a ten-thousandth of a frequency or a Q. A snapped value
/// is exactly the setting, which is what makes a band at zero skippable.
const SETTLE_LINEAR: f32 = 0.001;
const SETTLE_RELATIVE: f32 = 1.0e-4;

/// The high-pass is one Butterworth section: minus three decibels at the
/// cutoff, twelve per octave below it.
const BUTTERWORTH_Q: f32 = core::f32::consts::FRAC_1_SQRT_2;

/// The shelves' slope, the cookbook's S: one is the steepest that stays
/// monotonic, with the corner frequency halfway up the shelf.
const SHELF_SLOPE: f32 = 1.0;

const BAND_COUNT: usize = 5;
const HPF: usize = 0;
const LOW_SHELF: usize = 1;
const PEAK1: usize = 2;
const PEAK2: usize = 3;
const HIGH_SHELF: usize = 4;

/// A setting on its way to where it was put.
#[derive(Clone, Copy, Default)]
struct Ramp {
    current: f32,
    target: f32,
}

/// The closeness at which a ramp for this parameter snaps to its target.
fn settle_distance(index: usize, target: f32) -> f32 {
    match PARAMETERS[index].kind {
        Kind::Float {
            taper: Taper::Logarithmic,
            ..
        } => abs(target) * SETTLE_RELATIVE,
        _ => SETTLE_LINEAR,
    }
}

pub struct Engine {
    settings: Settings,
    sample_rate: f32,
    smoothing: f32,

    /// One ramp per parameter, addressed by parameter index. Only the
    /// continuous ones and the high-pass switch travel; bypass snaps.
    ramps: [Ramp; PARAMETER_COUNT],
    moving: bool,
    until_design: u32,

    coefficients: [Coefficients; BAND_COUNT],
    active: [bool; BAND_COUNT],
    sections: [[Biquad; 2]; BAND_COUNT],
    /// How much of the high-pass is in the signal: one when it is on, a ramp
    /// between dry and filtered while it is switching, so the switch does
    /// not click.
    hpf_mix: f32,
    output_gain: f32,
    bypass: bool,
}

impl Default for Engine {
    fn default() -> Self {
        let mut engine = Self {
            settings: Settings::default(),
            sample_rate: 48_000.0,
            smoothing: 1.0,
            ramps: [Ramp::default(); PARAMETER_COUNT],
            moving: false,
            until_design: DESIGN_INTERVAL,
            coefficients: [Coefficients::IDENTITY; BAND_COUNT],
            active: [false; BAND_COUNT],
            sections: [[Biquad::default(); 2]; BAND_COUNT],
            hpf_mix: 0.0,
            output_gain: 1.0,
            bypass: false,
        };
        engine.snap();
        engine
    }
}

impl Engine {
    /// Sets the sample rate and puts every stage at rest. Refuses a rate that
    /// is not a positive, finite number.
    pub fn prepare(&mut self, sample_rate: f64) -> bool {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return false;
        }
        self.sample_rate = sample_rate as f32;
        self.smoothing = one_pole(SMOOTHING_S, self.sample_rate);
        self.snap();
        self.reset();
        true
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Clears every section's memory. The settings, and where their ramps
    /// are, stay.
    pub fn reset(&mut self) {
        for band in &mut self.sections {
            for section in band {
                section.clear();
            }
        }
    }

    /// A new setting starts its ramp travelling; the sections follow.
    pub fn set_parameter(&mut self, index: u32, value: f64) -> bool {
        if !self.settings.set(index, value) {
            return false;
        }
        self.retarget();
        true
    }

    pub fn parameter(&self, index: u32) -> Option<f64> {
        self.settings.get(index)
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Minimum-phase sections, no lookahead: nothing is held back.
    pub fn latency(&self) -> usize {
        0
    }

    /// Loads a preset as a state, not as a fader move: the ramps snap and
    /// the sections start clean.
    pub fn load_preset(&mut self, id: &str) -> bool {
        let Some(settings) = preset::settings_for(id) else {
            return false;
        };
        self.settings = settings;
        self.snap();
        self.reset();
        true
    }

    pub fn save_state(&self, destination: &mut [u8]) -> Option<usize> {
        let target = destination.get_mut(..STATE_BYTES)?;
        for (slot, value) in target
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(self.settings.as_array())
        {
            *slot = value.to_le_bytes();
        }
        Some(STATE_BYTES)
    }

    /// Restores a saved block, including one written by an earlier build.
    /// Its length identifies its layout; anything else is refused whole.
    pub fn load_state(&mut self, state: &[u8]) -> bool {
        if !state.len().is_multiple_of(4) {
            return false;
        }
        let count = state.len() / 4;
        if count != PARAMETER_COUNT && !PREVIOUS_PARAMETER_COUNTS.contains(&count) {
            return false;
        }
        let mut values = [0.0_f32; PARAMETER_COUNT];
        for (value, word) in values.iter_mut().zip(state.as_chunks::<4>().0) {
            *value = f32::from_le_bytes(*word);
        }
        let Some(settings) = Settings::from_slice(&values[..count]) else {
            return false;
        };
        self.settings = settings;
        self.snap();
        self.reset();
        true
    }

    /// Points every ramp at its setting and starts the travel. Bypass is not
    /// a ramp: it takes effect at once, and entering it clears the sections
    /// so leaving it does not replay what was there.
    fn retarget(&mut self) {
        let values = self.settings.as_array();
        for (ramp, value) in self.ramps.iter_mut().zip(values) {
            ramp.target = value;
        }
        self.ramps[BYPASS as usize].current = values[BYPASS as usize];
        let bypass = self.settings.engaged(BYPASS);
        if bypass && !self.bypass {
            self.reset();
        }
        self.bypass = bypass;
        self.moving = self.ramps.iter().any(|ramp| ramp.current != ramp.target);
        if self.moving {
            self.until_design = DESIGN_INTERVAL;
        }
    }

    /// Puts every ramp at its setting and designs from there: a preset, a
    /// restored state and a fresh sample rate all land at once.
    fn snap(&mut self) {
        for (ramp, value) in self.ramps.iter_mut().zip(self.settings.as_array()) {
            ramp.current = value;
            ramp.target = value;
        }
        self.bypass = self.settings.engaged(BYPASS);
        self.moving = false;
        self.design();
    }

    /// One step of every travelling ramp, and a redesign when it is due or
    /// when the last ramp has arrived.
    #[inline]
    fn advance(&mut self) {
        let mut still = false;
        for (index, ramp) in self.ramps.iter_mut().enumerate() {
            if ramp.current == ramp.target {
                continue;
            }
            ramp.current += (ramp.target - ramp.current) * self.smoothing;
            if abs(ramp.target - ramp.current) <= settle_distance(index, ramp.target) {
                ramp.current = ramp.target;
            } else {
                still = true;
            }
        }
        self.until_design -= 1;
        if !still || self.until_design == 0 {
            self.design();
            self.until_design = DESIGN_INTERVAL;
        }
        self.moving = still;
    }

    #[inline]
    fn travelling(&self, index: u32) -> f32 {
        self.ramps[index as usize].current
    }

    /// A travelling frequency, held under the design ceiling.
    #[inline]
    fn frequency(&self, index: u32) -> f32 {
        clamp(
            self.travelling(index),
            1.0,
            NYQUIST_FRACTION * self.sample_rate,
        )
    }

    /// Designs every band from the travelling values. A band whose gain is
    /// exactly zero is switched off rather than designed at unity, and its
    /// memory cleared, so switching it back on starts from silence.
    fn design(&mut self) {
        let rate = self.sample_rate;

        let mix = clamp(self.travelling(HPF_ENABLE), 0.0, 1.0);
        self.hpf_mix = mix;
        self.set_band(
            HPF,
            mix > 0.0,
            Coefficients::high_pass(self.frequency(HPF_FREQUENCY), BUTTERWORTH_Q, rate),
        );

        let low_gain = self.travelling(LOW_SHELF_GAIN);
        self.set_band(
            LOW_SHELF,
            low_gain != 0.0,
            Coefficients::low_shelf(
                self.frequency(LOW_SHELF_FREQUENCY),
                low_gain,
                SHELF_SLOPE,
                rate,
            ),
        );

        let peak1_gain = self.travelling(PEAK1_GAIN);
        self.set_band(
            PEAK1,
            peak1_gain != 0.0,
            Coefficients::peak(
                self.frequency(PEAK1_FREQUENCY),
                peak1_gain,
                self.travelling(PEAK1_Q),
                rate,
            ),
        );

        let peak2_gain = self.travelling(PEAK2_GAIN);
        self.set_band(
            PEAK2,
            peak2_gain != 0.0,
            Coefficients::peak(
                self.frequency(PEAK2_FREQUENCY),
                peak2_gain,
                self.travelling(PEAK2_Q),
                rate,
            ),
        );

        let high_gain = self.travelling(HIGH_SHELF_GAIN);
        self.set_band(
            HIGH_SHELF,
            high_gain != 0.0,
            Coefficients::high_shelf(
                self.frequency(HIGH_SHELF_FREQUENCY),
                high_gain,
                SHELF_SLOPE,
                rate,
            ),
        );

        self.output_gain = db_to_gain(self.travelling(OUTPUT));
    }

    fn set_band(&mut self, band: usize, active: bool, coefficients: Coefficients) {
        if active {
            self.coefficients[band] = if coefficients.is_finite() {
                coefficients
            } else {
                Coefficients::IDENTITY
            };
        } else if self.active[band] {
            for section in &mut self.sections[band] {
                section.clear();
            }
            self.coefficients[band] = Coefficients::IDENTITY;
        }
        self.active[band] = active;
    }

    /// One stereo frame in, one out. A mono input arrives on both sides.
    #[inline]
    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        let inputs = [sanitise(left), sanitise(right)];
        if self.moving {
            self.advance();
        }
        if self.bypass {
            return (inputs[0], inputs[1]);
        }
        let mut samples = inputs;
        for (band, sections) in self.sections.iter_mut().enumerate() {
            if !self.active[band] {
                continue;
            }
            let coefficients = &self.coefficients[band];
            for (section, sample) in sections.iter_mut().zip(samples.iter_mut()) {
                let filtered = section.process(coefficients, *sample);
                *sample = if band == HPF && self.hpf_mix < 1.0 {
                    *sample + self.hpf_mix * (filtered - *sample)
                } else {
                    filtered
                };
            }
        }
        (
            sanitise(samples[0] * self.output_gain),
            sanitise(samples[1] * self.output_gain),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{PI, gain_to_db, sin};
    use crate::measure::response_db;

    const RATE: f32 = 48_000.0;

    fn prepared() -> Engine {
        let mut engine = Engine::default();
        assert!(engine.prepare(f64::from(RATE)));
        engine
    }

    fn set(engine: &mut Engine, index: u32, value: f32) {
        assert!(
            engine.set_parameter(index, f64::from(value)),
            "parameter {index} refused {value}"
        );
    }

    /// A signal with something at every band: four sines and a slow one.
    fn programme(n: usize) -> f32 {
        let t = n as f32 / RATE;
        0.2 * sin(2.0 * PI * 30.0 * t)
            + 0.2 * sin(2.0 * PI * 220.0 * t + 0.3)
            + 0.15 * sin(2.0 * PI * 1_750.0 * t + 1.1)
            + 0.1 * sin(2.0 * PI * 6_400.0 * t + 2.0)
            + 0.05 * sin(2.0 * PI * 15_000.0 * t + 0.7)
    }

    fn assert_near(actual: f32, expected: f32, tolerance: f32, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{what}: {actual:.3} dB, expected {expected:.3} ± {tolerance}"
        );
    }

    #[test]
    fn it_refuses_a_rate_that_is_not_a_rate() {
        let mut engine = Engine::default();
        assert!(!engine.prepare(0.0));
        assert!(!engine.prepare(-48_000.0));
        assert!(!engine.prepare(f64::NAN));
        assert!(engine.prepare(192_000.0));
        assert_eq!(engine.latency(), 0);
    }

    #[test]
    fn the_default_state_is_transparent() {
        let mut engine = prepared();
        for n in 0..(RATE as usize) {
            let x = programme(n);
            let (l, r) = engine.process(x, x * 0.5);
            assert!((l - x).abs() <= 1.0e-6, "left {l} for {x} at {n}");
            assert!((r - x * 0.5).abs() <= 1.0e-6, "right {r} for {x} at {n}");
        }
    }

    #[test]
    fn bypass_returns_the_input_exactly() {
        let mut engine = prepared();
        set(&mut engine, HPF_ENABLE, 1.0);
        set(&mut engine, HPF_FREQUENCY, 200.0);
        set(&mut engine, LOW_SHELF_GAIN, 9.0);
        set(&mut engine, PEAK1_GAIN, -12.0);
        set(&mut engine, PEAK2_GAIN, 15.0);
        set(&mut engine, HIGH_SHELF_GAIN, -6.0);
        set(&mut engine, OUTPUT, 12.0);
        set(&mut engine, BYPASS, 1.0);
        for n in 0..(RATE as usize) {
            let x = programme(n);
            let (l, r) = engine.process(x, -x);
            assert_eq!(l, x);
            assert_eq!(r, -x);
        }
        // Leaving bypass brings the curve back.
        set(&mut engine, BYPASS, 0.0);
        let boosted = response_db(&mut engine, 3_000.0, 0.1);
        assert!(boosted > 20.0, "peak 2 plus the trim read {boosted}");
    }

    #[test]
    fn a_peak_band_reads_its_gain_at_the_centre_and_nothing_four_octaves_away() {
        for gain in [6.0, -12.0, 15.0] {
            let mut engine = prepared();
            set(&mut engine, PEAK1_FREQUENCY, 500.0);
            set(&mut engine, PEAK1_Q, 1.0);
            set(&mut engine, PEAK1_GAIN, gain);
            let what = std::format!("peak 1 at {gain} dB");
            assert_near(
                response_db(&mut engine, 500.0, 0.25),
                gain,
                0.15,
                &std::format!("{what}, centre"),
            );
            assert_near(
                response_db(&mut engine, 500.0 / 16.0, 0.25),
                0.0,
                0.3,
                &std::format!("{what}, four octaves below"),
            );
            assert_near(
                response_db(&mut engine, 500.0 * 16.0, 0.25),
                0.0,
                0.3,
                &std::format!("{what}, four octaves above"),
            );
        }
        // A narrow band at the top of the second peak's range.
        let mut engine = prepared();
        set(&mut engine, PEAK2_FREQUENCY, 3_000.0);
        set(&mut engine, PEAK2_Q, 10.0);
        set(&mut engine, PEAK2_GAIN, 15.0);
        assert_near(
            response_db(&mut engine, 3_000.0, 0.1),
            15.0,
            0.15,
            "narrow peak 2, centre",
        );
        assert_near(
            response_db(&mut engine, 3_000.0 / 16.0, 0.1),
            0.0,
            0.3,
            "narrow peak 2, four octaves below",
        );
    }

    #[test]
    fn a_shelf_reaches_its_gain_two_octaves_in() {
        for gain in [6.0, -9.0] {
            let mut engine = prepared();
            set(&mut engine, LOW_SHELF_FREQUENCY, 120.0);
            set(&mut engine, LOW_SHELF_GAIN, gain);
            let what = std::format!("low shelf at {gain} dB");
            assert_near(
                response_db(&mut engine, 30.0, 0.25),
                gain,
                0.3,
                &std::format!("{what}, two octaves below"),
            );
            assert_near(
                response_db(&mut engine, 480.0, 0.25),
                0.0,
                0.3,
                &std::format!("{what}, two octaves above"),
            );

            let mut engine = prepared();
            set(&mut engine, HIGH_SHELF_FREQUENCY, 2_000.0);
            set(&mut engine, HIGH_SHELF_GAIN, gain);
            let what = std::format!("high shelf at {gain} dB");
            assert_near(
                response_db(&mut engine, 8_000.0, 0.25),
                gain,
                0.3,
                &std::format!("{what}, two octaves above"),
            );
            assert_near(
                response_db(&mut engine, 500.0, 0.25),
                0.0,
                0.3,
                &std::format!("{what}, two octaves below"),
            );
        }
    }

    #[test]
    fn the_high_pass_is_a_butterworth_at_twelve_decibels_an_octave() {
        let mut engine = prepared();
        set(&mut engine, HPF_FREQUENCY, 100.0);
        // Off, it is not there at all.
        assert_eq!(response_db(&mut engine, 50.0, 0.25), 0.0);
        set(&mut engine, HPF_ENABLE, 1.0);
        assert_near(
            response_db(&mut engine, 100.0, 0.25),
            -3.0,
            0.3,
            "high-pass at its cutoff",
        );
        let octave_below = response_db(&mut engine, 50.0, 0.25);
        assert!(
            (-14.0..=-11.0).contains(&octave_below),
            "an octave below the cutoff read {octave_below} dB"
        );
        assert_near(
            response_db(&mut engine, 1_600.0, 0.25),
            0.0,
            0.1,
            "high-pass four octaves up",
        );
    }

    #[test]
    fn the_trim_is_a_gain() {
        let mut engine = prepared();
        set(&mut engine, OUTPUT, 6.0);
        assert_near(
            response_db(&mut engine, 1_000.0, 0.25),
            6.0,
            0.01,
            "trim +6",
        );
        // Doubling is 6.0206 dB; at that trim the amplitude doubles, sample
        // for sample, to within a hundredth of a decibel.
        let doubling = 20.0 * libm::log10f(2.0);
        set(&mut engine, OUTPUT, doubling);
        assert_near(
            response_db(&mut engine, 1_000.0, 0.25),
            doubling,
            0.01,
            "trim at doubling",
        );
        let x = 0.3_f32;
        let (l, r) = engine.process(x, -x);
        assert_near(
            gain_to_db(l / (2.0 * x)),
            0.0,
            0.01,
            "left against twice the input",
        );
        assert_near(
            gain_to_db(-r / (2.0 * x)),
            0.0,
            0.01,
            "right against twice the input",
        );
    }

    #[test]
    fn both_channels_receive_the_same_curve() {
        let mut engine = prepared();
        assert!(engine.load_preset("piano_air"));
        set(&mut engine, PEAK1_GAIN, 4.0);
        for n in 0..(RATE as usize / 2) {
            let x = programme(n);
            let (l, r) = engine.process(x, x);
            assert_eq!(l, r, "at {n}");
        }
    }

    #[test]
    fn ten_seconds_of_silence_leave_every_band_finite_and_at_rest() {
        let mut engine = prepared();
        set(&mut engine, HPF_ENABLE, 1.0);
        set(&mut engine, HPF_FREQUENCY, 20.0);
        set(&mut engine, LOW_SHELF_GAIN, 15.0);
        set(&mut engine, PEAK1_GAIN, -15.0);
        set(&mut engine, PEAK1_Q, 10.0);
        set(&mut engine, PEAK2_GAIN, 15.0);
        set(&mut engine, PEAK2_Q, 0.3);
        set(&mut engine, HIGH_SHELF_GAIN, -15.0);
        set(&mut engine, OUTPUT, 24.0);
        // A burst first, so there is a tail to decay.
        for n in 0..(RATE as usize / 10) {
            let x = programme(n);
            engine.process(x, x);
        }
        let mut last = (1.0_f32, 1.0_f32);
        for _ in 0..(10 * RATE as usize) {
            last = engine.process(0.0, 0.0);
            assert!(last.0.is_finite() && last.1.is_finite());
        }
        assert_eq!(last, (0.0, 0.0));
    }

    #[test]
    fn a_thirty_decibel_jump_arrives_as_a_ramp() {
        let mut engine = prepared();
        set(&mut engine, PEAK1_FREQUENCY, 500.0);
        set(&mut engine, PEAK1_Q, 1.0);
        set(&mut engine, PEAK1_GAIN, -15.0);
        let amplitude = 0.5_f32;
        let tone = |n: usize| amplitude * sin(2.0 * PI * 500.0 * n as f32 / RATE);
        let mut n = 0;
        let mut previous = 0.0_f32;
        for _ in 0..(RATE as usize / 4) {
            previous = engine.process(tone(n), tone(n)).0;
            n += 1;
        }
        set(&mut engine, PEAK1_GAIN, 15.0);
        let mut largest_step = 0.0_f32;
        for _ in 0..(RATE as usize / 4) {
            let (l, _) = engine.process(tone(n), tone(n));
            largest_step = largest_step.max((l - previous).abs());
            previous = l;
            n += 1;
        }
        assert!(
            largest_step <= amplitude,
            "the jump stepped by {largest_step} against a peak of {amplitude}"
        );
        // And it does arrive.
        assert_near(
            response_db(&mut engine, 500.0, 0.25),
            15.0,
            0.15,
            "after the jump",
        );
    }

    #[test]
    fn a_frequency_above_the_ceiling_is_held_under_it() {
        let mut engine = Engine::default();
        assert!(engine.prepare(8_000.0));
        set(&mut engine, PEAK2_FREQUENCY, 16_000.0);
        set(&mut engine, PEAK2_GAIN, 12.0);
        set(&mut engine, HIGH_SHELF_FREQUENCY, 16_000.0);
        set(&mut engine, HIGH_SHELF_GAIN, 6.0);
        for n in 0..8_000 {
            let x = 0.5 * sin(2.0 * PI * 1_000.0 * n as f32 / 8_000.0);
            let (l, r) = engine.process(x, x);
            assert!(l.is_finite() && r.is_finite(), "at {n}");
        }
        // Both landed at the ceiling, 3.6 kHz: the peak reads its gain there
        // and the shelf, whose corner sits halfway up, reads half of its.
        assert_near(
            response_db(&mut engine, 3_600.0, 0.1),
            12.0 + 3.0,
            0.5,
            "peak 2 and the high shelf at the ceiling",
        );
    }

    #[test]
    fn state_round_trips_and_a_bad_length_is_refused() {
        let mut engine = prepared();
        set(&mut engine, PEAK2_FREQUENCY, 2_500.0);
        set(&mut engine, HPF_ENABLE, 1.0);
        let mut block = [0_u8; STATE_BYTES];
        assert_eq!(engine.save_state(&mut block), Some(STATE_BYTES));
        let mut other = prepared();
        assert!(other.load_state(&block));
        assert_eq!(other.parameter(PEAK2_FREQUENCY), Some(2_500.0));
        assert_eq!(other.parameter(HPF_ENABLE), Some(1.0));
        assert!(!other.load_state(&block[..7]));
        assert!(!other.load_state(&block[..8]));
    }

    #[test]
    fn every_preset_loads_and_lands_at_once() {
        let mut engine = prepared();
        for preset in rf_eq_contract::PRESETS.iter() {
            assert!(engine.load_preset(preset.id), "{}", preset.id);
            assert!(!engine.moving, "{} left a ramp travelling", preset.id);
        }
        assert!(!engine.load_preset("nowhere"));
    }

    #[test]
    fn piano_air_reads_as_described() {
        let mut engine = prepared();
        assert!(engine.load_preset("piano_air"));
        assert_near(
            response_db(&mut engine, 16_000.0, 0.25),
            2.5,
            0.3,
            "an octave into the air",
        );
        assert_near(
            response_db(&mut engine, 40.0, 0.25),
            -1.0,
            0.3,
            "under the low shelf",
        );
        assert_near(
            response_db(&mut engine, 1_000.0, 0.25),
            0.0,
            0.2,
            "the middle",
        );
    }
}
