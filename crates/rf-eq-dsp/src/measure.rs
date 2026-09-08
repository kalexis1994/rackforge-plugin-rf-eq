//! Reads the gain of an engine at one frequency by running a sine through it.
//!
//! The tests and the bench share this so that "the peak band reads its gain
//! at its centre" means the same thing in both. The measurement is a
//! projection onto the sine and cosine at the test frequency over a whole
//! number of cycles, which is blind to harmonics, to direct current and to
//! where the samples fall on the waveform — a peak detector at 16 kHz with
//! three samples a cycle would read low.

use crate::engine::Engine;

/// How long the engine is given to settle before the window opens. Two
/// hundred milliseconds is more than fifty time constants of the slowest
/// band the contract allows.
pub const SETTLE_S: f64 = 0.2;

/// The shortest window: at least this long, rounded up to whole cycles.
pub const WINDOW_S: f64 = 0.3;

/// The gain of the left channel at `frequency`, in decibels relative to the
/// sine that went in. The right channel receives the same sine; a stereo
/// engine with one set of coefficients returns the same reading on both, and
/// the tests say so.
pub fn response_db(engine: &mut Engine, frequency: f64, amplitude: f64) -> f32 {
    let rate = f64::from(engine.sample_rate());
    let step = 2.0 * core::f64::consts::PI * frequency / rate;
    let settle = (SETTLE_S * rate) as usize;
    let cycles = libm::ceil(WINDOW_S * frequency);
    let window = libm::round(cycles * rate / frequency) as usize;

    let mut phase = 0.0_f64;
    for _ in 0..settle {
        let x = (amplitude * libm::sin(phase)) as f32;
        engine.process(x, x);
        phase += step;
    }

    // The input's own projection calibrates the output's, so the half-sample
    // the window is short or long of whole cycles cancels.
    let mut in_sin = 0.0_f64;
    let mut in_cos = 0.0_f64;
    let mut out_sin = 0.0_f64;
    let mut out_cos = 0.0_f64;
    for _ in 0..window {
        let (s, c) = (libm::sin(phase), libm::cos(phase));
        let x = (amplitude * s) as f32;
        let (y, _) = engine.process(x, x);
        in_sin += f64::from(x) * s;
        in_cos += f64::from(x) * c;
        out_sin += f64::from(y) * s;
        out_cos += f64::from(y) * c;
        phase += step;
    }
    let input = libm::sqrt(in_sin * in_sin + in_cos * in_cos);
    let output = libm::sqrt(out_sin * out_sin + out_cos * out_cos);
    if output <= 1.0e-12 * input {
        return -120.0;
    }
    (20.0 * libm::log10(output / input)) as f32
}

/// The nominal third-octave centres from 20 Hz to 20 kHz, as the bench
/// prints them.
pub const THIRD_OCTAVES: [f64; 31] = [
    20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0,
    500.0, 630.0, 800.0, 1_000.0, 1_250.0, 1_600.0, 2_000.0, 2_500.0, 3_150.0, 4_000.0, 5_000.0,
    6_300.0, 8_000.0, 10_000.0, 12_500.0, 16_000.0, 20_000.0,
];
