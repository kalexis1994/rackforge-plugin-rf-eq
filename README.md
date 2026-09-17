# RF-EQ

An eight-band parametric equaliser for
[RackForge](https://github.com/kalexis1994/rackforge): high-pass and low-pass
filters, low and high shelves, four fully parametric peaks, output trim and
bypass. The PLAY surface includes a live response graph whose nodes edit the
same parameters as the compact control strips.

`v0.2.0` preserves every `v0.1.0` state and adds the two extra peaks and the
low-pass at their transparent defaults.

## How it shapes the signal

Every band is one second-order section from the Audio EQ Cookbook, designed
at the host's sample rate, and the eight run in series with one set of
coefficients for both channels. The high-pass and low-pass are Butterworth
sections: minus three decibels at the cutoff and twelve decibels per octave
into the stop band. The shelves
have the cookbook's slope of one, the steepest that stays monotonic, with the
corner frequency halfway up the shelf. A peak reads its full gain at its
centre; its Q is the usual one, a bell an octave and a bit wide at 1.0.

A knob does not land where it is put; it travels there. Every setting has a
target and a value on its way to it, moved by a five-millisecond one-pole
ramp, and the sections are redesigned from the travelling values every eight
samples while anything is still moving. A thirty-decibel jump in automation
arrives as a slope, never a step.

A band at zero decibels is skipped rather than designed at unity, as are the
high-pass and low-pass when they are off, so the default state is the input bit for bit.
Bypass is a hard switch that returns the input, which makes a comparison
against it honest. Frequencies are held under 0.45 of the sample rate; the
sections flush their states so a tail ends in silence rather than denormals,
and a NaN cannot take up residence. Nothing is held back: the latency is zero.

## Controls

| Control | Range | What it is |
| --- | --- | --- |
| High-pass | on/off | The high-pass section, off by default. |
| HPF Freq | 20 … 500 Hz | Its cutoff: minus three decibels here, twelve per octave below. |
| Low Freq | 40 … 1000 Hz | The low shelf's corner, halfway up the shelf. |
| Low Gain | −15 … +15 dB | What the low shelf adds below its corner. |
| Peak 1 Freq | 100 … 8000 Hz | The first bell's centre. |
| Peak 1 Gain | −15 … +15 dB | Its gain at the centre. |
| Peak 1 Q | 0.3 … 10 | Its width; higher is narrower. |
| Peak 2 Freq | 200 … 16000 Hz | The second bell's centre. |
| Peak 2 Gain | −15 … +15 dB | Its gain at the centre. |
| Peak 2 Q | 0.3 … 10 | Its width. |
| Peak 3 Freq | 300 … 20000 Hz | The third bell's centre. |
| Peak 3 Gain | −15 … +15 dB | Its gain at the centre. |
| Peak 3 Q | 0.3 … 10 | Its width. |
| Peak 4 Freq | 500 … 20000 Hz | The fourth bell's centre. |
| Peak 4 Gain | −15 … +15 dB | Its gain at the centre. |
| Peak 4 Q | 0.3 … 10 | Its width. |
| High Freq | 1000 … 16000 Hz | The high shelf's corner. |
| High Gain | −15 … +15 dB | What the high shelf adds above its corner. |
| Low-pass | on/off | The low-pass section, off by default. |
| LPF Freq | 500 … 20000 Hz | Its cutoff: minus three decibels here, twelve per octave above. |
| Output | −24 … +24 dB | Trim after the bands. |
| Bypass | on/off | Passes the input untouched. |

Frequencies and Q travel logarithmically on their knobs; gains are linear.

## Factory settings

| Setting | What it does |
| --- | --- |
| Flat | Every band at zero, the high-pass off. |
| Piano Air | High shelf +2.5 dB at 8 kHz, low shelf −1 dB at 120 Hz. |
| Warm | Low shelf +2 dB at 150 Hz, high shelf −2 dB at 6 kHz. |
| Presence | Peak 2 +3 dB at 3 kHz, Q 1.2. |
| Rumble Cut | High-pass on at 60 Hz. |
| Telephone | High-pass at 300 Hz and low-pass at 3 kHz. |
| Mix Cleanup | Subsonic and low-mid cleanup with a broad presence lift. |
| Master Polish | Four restrained, broad moves and compensated output. |

## Build and install

The RackForge checkout must sit next to this one, because the packager lives
there.

```bash
pwsh tools/build-package.ps1
```

```bash
bash tools/build-package.sh
```

Either script regenerates the package metadata from the contract, builds the
WebAssembly component, runs the tests and packs `artifacts/RF-EQ.rfplugin`.
Install it the way you install any RackForge plugin — the desktop's Plugin
Manager, or:

```bash
./target/release/rackforge-desktop.exe --install-plugin ../rackforge-plugin-rf-eq/artifacts/RF-EQ.rfplugin
```

In PLAY, open the FX drawer and add it after the instrument. In LIVE it is
an ordinary effect Slot.

## The bench

```bash
cargo run -p rf-eq-lab -- response --preset piano_air
cargo run -p rf-eq-lab -- response --set hpf.enable=1 --set hpf.frequency=80
cargo run -p rf-eq-lab -- render --out shaped.wav --input piano.wav --preset warm
```

`response` runs a sine through the equaliser at every third octave from
20 Hz to 20 kHz and prints the gain it reads, the way the tests read it;
`render` puts a file, or a logarithmic sweep, through the equaliser so it
can be listened to.

## What the tests hold

At 48 kHz, measured with a sine after 200 ms of settling: a peak reads its
gain at its centre within 0.15 dB and within 0.3 dB of nothing four octaves
away; a shelf reaches its gain within 0.3 dB two octaves in; the high-pass
reads −3 dB ± 0.3 at its cutoff and between −11 and −14 dB an octave below.
The low-pass has the same cutoff and octave tolerances in the opposite
direction. The default is transparent to within 1e-6, bypass is the input exactly, a
trim at the doubling gain doubles within 0.01 dB, ten seconds of silence with
every band engaged leave the output finite and at zero, and a jump from −15
to +15 dB never steps a sample by more than the input's own peak.
