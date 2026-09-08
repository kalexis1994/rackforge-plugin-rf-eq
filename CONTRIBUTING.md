# Contributing to RF-EQ

## The rule that matters most

A constant that shapes the behaviour should be traceable to something outside
somebody's taste: a time, a slope, a fraction of the sample rate, a
measurement. When that is not yet true, say so where the code is.

## Working on it

```bash
cargo test --workspace          # everything
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

Before opening a pull request, build the package once — it regenerates the
metadata, builds the component, runs the tests and proves the component still
packs:

```bash
pwsh tools/build-package.ps1     # or: bash tools/build-package.sh
```

On a machine whose default Rust is windows-gnu, pass
`+stable-x86_64-pc-windows-msvc`: the RackForge runtime only builds under
MSVC (the build script does it for you).

## Generated files are generated

`plugin/package/metadata/*.json` and `plugin/package/branding/*.png` are
outputs. Edit the contract or the generator, never the JSON:

```bash
cargo run -p rf-eq-lab -- metadata
cargo run -p rf-eq-lab -- metadata --check    # what CI runs
python tools/generate-branding.py
```

The version lives in `plugin/package/rackforge-plugin.toml` and nowhere else;
the runtime descriptor is generated from it.

## Adding a control

1. Add it to `rf-eq-contract` (`lib.rs`, `index.rs`) at the end and bump
   `PARAMETER_COUNT`. State is a flat block of `f32`s keyed by length, so
   adding parameters is a state-version change: bump `state_version` in the
   manifest and add the old count to `PREVIOUS_PARAMETER_COUNTS`.
2. Read it in `engine.rs`'s `design`, through its ramp, so it travels like
   the others.
3. Tests: what the equaliser is supposed to *do*, not what the code currently
   returns. "A peak reads its gain at its centre", "the default is
   transparent", "a jump arrives as a ramp". Measure with
   `rf_eq_dsp::measure::response_db`, the way the bench does.
4. `cargo run -p rf-eq-lab -- metadata`.

The interface needs no change: it builds itself from the schema. A parameter
whose identifier starts with a band's prefix (`peak1.`, `high_shelf.`) joins
that band's strip on the surface; a new band needs a name in `STRIP_NAMES`
in `web/play.js`.

## Style

The house voice is descriptive, never boastful. Comments explain *why* a value
is what it is, not what the line does. No trademarks, no brand names, no
product names.
