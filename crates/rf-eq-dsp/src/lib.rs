//! RF-EQ: an eight-band parametric equaliser.
//!
//! * [`biquad`] holds the second-order section and the designs that fill it —
//!   high-pass, low and high shelf, peak — from the Audio EQ Cookbook;
//! * [`Engine`] is the equaliser: the sections in series, one set of
//!   coefficients for both channels, and a short ramp under every knob;
//! * [`measure`] runs a sine through an engine and reads the gain back, so
//!   the tests and the bench measure the same way.
//!
//! Two rules hold everywhere. Nothing allocates after activation. And every
//! constant that shapes the behaviour is named for what it is — a time, a
//! slope, a fraction of the sample rate — so that changing it means changing
//! a decision rather than nudging a number.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod biquad;
pub mod engine;
pub mod math;
pub mod measure;

pub use engine::{Engine, STATE_BYTES};
