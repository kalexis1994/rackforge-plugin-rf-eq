# Notices

RF-EQ is an independent RackForge plugin implemented in Rust and distributed
under GPL-3.0-only.

It implements a parametric equaliser from first principles: second-order
sections designed with the public-domain formulae of the Audio EQ Cookbook,
run in the transposed direct form II, with a one-pole ramp under each
setting. These are textbook signal-processing constructions; no third-party
code, artwork, trademark or brand name is included in this repository or in
the plugin package.

Third-party code: the plugin depends on `rackforge-plugin-sdk` (MIT OR
Apache-2.0) and on `libm` (MIT/Apache-2.0).
