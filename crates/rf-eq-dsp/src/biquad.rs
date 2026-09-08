//! The second-order section and the designs that fill it.
//!
//! Every band of RF-EQ is one biquad from the Audio EQ Cookbook, designed at
//! the host sample rate. The coefficients are normalised so the section runs
//! with five multiplies; the section is the transposed direct form II, whose
//! two states hold the filter's memory in a form that stays well behaved
//! while the coefficients move under it.

use crate::math::{PI, abs, cos, db_to_gain, sin, sqrt};

/// Below this, both states of a section are put to zero together. It is far
/// above the denormal range, so a tail never gets there, and far below
/// anything audible: minus four hundred decibels.
const FLUSH_BELOW: f32 = 1.0e-20;

/// A normalised biquad: `a0` is one.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Coefficients {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl Default for Coefficients {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// The intermediate terms every cookbook design starts from.
struct Prototype {
    cs: f32,
    sn: f32,
}

impl Prototype {
    fn at(frequency: f32, sample_rate: f32) -> Self {
        let w0 = 2.0 * PI * frequency / sample_rate;
        Self {
            cs: cos(w0),
            sn: sin(w0),
        }
    }
}

impl Coefficients {
    /// A wire: the output is the input, bit for bit.
    pub const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// Divides through by `a0`. Five divisions rather than one reciprocal
    /// and five multiplies: this runs at design time, not per sample, and
    /// a division keeps `b0` exactly one when `b0` equals `a0`.
    fn normalised(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// A second-order high-pass, minus three decibels at `frequency` when
    /// `q` is the Butterworth value.
    pub fn high_pass(frequency: f32, q: f32, sample_rate: f32) -> Self {
        let Prototype { cs, sn } = Prototype::at(frequency, sample_rate);
        let alpha = sn / (2.0 * q);
        let half = (1.0 + cs) * 0.5;
        Self::normalised(half, -(1.0 + cs), half, 1.0 + alpha, -2.0 * cs, 1.0 - alpha)
    }

    /// A peaking band: `gain_db` at `frequency`, `q` its width.
    pub fn peak(frequency: f32, gain_db: f32, q: f32, sample_rate: f32) -> Self {
        let Prototype { cs, sn } = Prototype::at(frequency, sample_rate);
        // The cookbook's A: the square root of the linear gain, so the band
        // reaches the full gain at its centre.
        let a = db_to_gain(gain_db * 0.5);
        let alpha = sn / (2.0 * q);
        Self::normalised(
            1.0 + alpha * a,
            -2.0 * cs,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cs,
            1.0 - alpha / a,
        )
    }

    /// The shelf designs share their terms; `slope` is the cookbook's S, with
    /// one the steepest that does not overshoot.
    fn shelf_terms(frequency: f32, gain_db: f32, slope: f32, sample_rate: f32) -> ShelfTerms {
        let Prototype { cs, sn } = Prototype::at(frequency, sample_rate);
        let a = db_to_gain(gain_db * 0.5);
        let alpha = sn * 0.5 * sqrt((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0);
        ShelfTerms {
            a,
            cs,
            root: 2.0 * sqrt(a) * alpha,
        }
    }

    /// A low shelf: `gain_db` below `frequency`, which sits halfway up it.
    pub fn low_shelf(frequency: f32, gain_db: f32, slope: f32, sample_rate: f32) -> Self {
        let ShelfTerms { a, cs, root } = Self::shelf_terms(frequency, gain_db, slope, sample_rate);
        Self::normalised(
            a * ((a + 1.0) - (a - 1.0) * cs + root),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
            a * ((a + 1.0) - (a - 1.0) * cs - root),
            (a + 1.0) + (a - 1.0) * cs + root,
            -2.0 * ((a - 1.0) + (a + 1.0) * cs),
            (a + 1.0) + (a - 1.0) * cs - root,
        )
    }

    /// A high shelf: `gain_db` above `frequency`, which sits halfway up it.
    pub fn high_shelf(frequency: f32, gain_db: f32, slope: f32, sample_rate: f32) -> Self {
        let ShelfTerms { a, cs, root } = Self::shelf_terms(frequency, gain_db, slope, sample_rate);
        Self::normalised(
            a * ((a + 1.0) + (a - 1.0) * cs + root),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
            a * ((a + 1.0) + (a - 1.0) * cs - root),
            (a + 1.0) - (a - 1.0) * cs + root,
            2.0 * ((a - 1.0) - (a + 1.0) * cs),
            (a + 1.0) - (a - 1.0) * cs - root,
        )
    }

    pub fn is_finite(&self) -> bool {
        self.b0.is_finite()
            && self.b1.is_finite()
            && self.b2.is_finite()
            && self.a1.is_finite()
            && self.a2.is_finite()
    }
}

struct ShelfTerms {
    a: f32,
    cs: f32,
    root: f32,
}

/// One channel's worth of a section: the two states of the transposed direct
/// form II.
///
/// The states are flushed every sample, so a decaying tail ends at zero
/// rather than in denormals, and a NaN cannot take up residence. They are
/// flushed *together*: zeroing one while the other is still alive turns the
/// flush into a source, and a high-pass near its cutoff would sit in a
/// sawtooth at the threshold forever.
#[derive(Clone, Copy, Default)]
pub struct Biquad {
    s1: f32,
    s2: f32,
}

impl Biquad {
    #[inline]
    pub fn process(&mut self, c: &Coefficients, x: f32) -> f32 {
        let y = c.b0 * x + self.s1;
        let s1 = c.b1 * x - c.a1 * y + self.s2;
        let s2 = c.b2 * x - c.a2 * y;
        let alive = abs(s1) > FLUSH_BELOW || abs(s2) > FLUSH_BELOW;
        if alive && s1.is_finite() && s2.is_finite() {
            self.s1 = s1;
            self.s2 = s2;
        } else {
            self.s1 = 0.0;
            self.s2 = 0.0;
        }
        y
    }

    pub fn clear(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    #[test]
    fn the_identity_is_a_wire() {
        let mut section = Biquad::default();
        for n in 0..1000 {
            let x = sin(n as f32 * 0.37) * 0.8;
            assert_eq!(section.process(&Coefficients::IDENTITY, x), x);
        }
    }

    #[test]
    fn a_band_at_zero_decibels_is_a_wire() {
        // At zero decibels the cookbook's numerator equals its denominator:
        // a zero on every pole. The engine skips such a band, but should it
        // ever run one, the section passes the signal bit for bit.
        for frequency in [40.0, 500.0, 3_000.0, 16_000.0] {
            for design in [
                Coefficients::peak(frequency, 0.0, 1.0, RATE),
                Coefficients::low_shelf(frequency, 0.0, 1.0, RATE),
                Coefficients::high_shelf(frequency, 0.0, 1.0, RATE),
            ] {
                assert_eq!(design.b0, 1.0, "{frequency} Hz");
                assert_eq!(design.b1, design.a1, "{frequency} Hz");
                assert_eq!(design.b2, design.a2, "{frequency} Hz");
                let mut section = Biquad::default();
                for n in 0..1000 {
                    let x = sin(n as f32 * 0.37) * 0.8;
                    assert_eq!(section.process(&design, x), x, "{frequency} Hz at {n}");
                }
            }
        }
    }

    #[test]
    fn the_high_pass_rejects_direct_current_outright() {
        for frequency in [20.0, 100.0, 500.0] {
            let c = Coefficients::high_pass(frequency, core::f32::consts::FRAC_1_SQRT_2, RATE);
            assert_eq!(c.b0 + c.b1 + c.b2, 0.0, "{frequency} Hz");
        }
    }

    #[test]
    fn every_design_is_finite_across_its_range_at_every_rate() {
        for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            for frequency in [20.0, 100.0, 1_000.0, 8_000.0, 16_000.0, 0.45 * rate] {
                for gain in [-15.0, -1.0, 0.0, 1.0, 15.0] {
                    for q in [0.3, 1.0, 10.0] {
                        assert!(Coefficients::peak(frequency, gain, q, rate).is_finite());
                    }
                    assert!(Coefficients::low_shelf(frequency, gain, 1.0, rate).is_finite());
                    assert!(Coefficients::high_shelf(frequency, gain, 1.0, rate).is_finite());
                }
                assert!(
                    Coefficients::high_pass(frequency, core::f32::consts::FRAC_1_SQRT_2, rate)
                        .is_finite()
                );
            }
        }
    }

    #[test]
    fn a_tail_ends_in_silence_rather_than_denormals() {
        let c = Coefficients::peak(500.0, 15.0, 10.0, RATE);
        let mut section = Biquad::default();
        section.process(&c, 1.0);
        let mut last = 1.0;
        for _ in 0..RATE as usize {
            last = section.process(&c, 0.0);
        }
        assert_eq!(last, 0.0);
        assert_eq!(section.s1, 0.0);
        assert_eq!(section.s2, 0.0);
    }
}
