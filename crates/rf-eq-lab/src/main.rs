//! The RF-EQ bench.
//!
//! Three jobs, all of them about keeping the project honest:
//!
//! * `metadata` renders the package's JSON from the contract, so the schema the
//!   host validates and the engine's behaviour come from one place;
//! * `render` puts a file, or a sweep, through the equaliser and writes a
//!   file you can listen to;
//! * `response` measures what the tests measure: the gain at every third
//!   octave from 20 Hz to 20 kHz, for a preset or a setting.

mod manifest;
mod metadata;

use std::path::{Path, PathBuf};

use rf_eq_contract::{PARAMETERS, PRESETS};
use rf_eq_dsp::Engine;
use rf_eq_dsp::math::gain_to_db;
use rf_eq_dsp::measure::{THIRD_OCTAVES, response_db};

const SAMPLE_RATE: u32 = 48_000;

/// The level the response is measured at: a quarter of full scale, so a
/// fifteen decibel band and a twenty-four decibel trim together stay finite
/// in a WAV without saying anything about the engine, which has no ceiling.
const MEASURE_AMPLITUDE: f64 = 0.25;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let command = arguments.first().map(String::as_str).unwrap_or("help");
    let result = match command {
        "metadata" => metadata_command(&arguments[1..]),
        "render" => render_command(&arguments[1..]),
        "response" => response_command(&arguments[1..]),
        "presets" => presets_command(),
        "parameters" => parameters_command(),
        _ => {
            usage();
            Ok(())
        }
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn usage() {
    println!(
        "rf-eq-lab <command>

  metadata [--check]              render package metadata from the contract
  parameters                      list every parameter and its range
  presets                         list the factory settings
  render --out <file.wav>         render a signal through the equaliser
         [--preset <id>] [--input <file.wav>] [--seconds <n>]
         [--set <id>=<value> ...]
  response [--preset <id>] [--set <id>=<value> ...]
                                  the gain at every third octave, 20 Hz to
                                  20 kHz, measured with a sine at 48 kHz

Values may be given by parameter identifier or index:
  --set peak1.frequency=800 --set peak1.gain=3"
    );
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .expect("the lab crate sits two levels below the repository root")
}

fn metadata_command(arguments: &[String]) -> Result<(), String> {
    let check = arguments.iter().any(|argument| argument == "--check");
    let package = repository_root().join("plugin").join("package");
    let identity = manifest::read(&package.join("rackforge-plugin.toml"))?;
    metadata::write(&package, &identity, check)?;
    if check {
        println!("metadata matches the contract");
    }
    Ok(())
}

fn parameters_command() -> Result<(), String> {
    for parameter in PARAMETERS.iter() {
        println!(
            "{:>3}  {:<22} {:<10} {}",
            parameter.index, parameter.id, parameter.page, parameter.name
        );
    }
    Ok(())
}

fn presets_command() -> Result<(), String> {
    for preset in PRESETS.iter() {
        println!(
            "{:<14} {:<14} {}",
            preset.id, preset.name, preset.description
        );
    }
    Ok(())
}

struct Options {
    preset: Option<String>,
    overrides: Vec<(u32, f64)>,
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    seconds: f32,
}

fn parse_options(arguments: &[String]) -> Result<Options, String> {
    let mut options = Options {
        preset: None,
        overrides: Vec::new(),
        input: None,
        output: None,
        seconds: 4.0,
    };
    let mut index = 0;
    while index < arguments.len() {
        let flag = arguments[index].as_str();
        let value = || {
            arguments
                .get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag {
            "--preset" => options.preset = Some(value()?),
            "--set" => options.overrides.push(parse_assignment(&value()?)?),
            "--input" => options.input = Some(PathBuf::from(value()?)),
            "--out" => options.output = Some(PathBuf::from(value()?)),
            "--seconds" => {
                options.seconds = value()?
                    .parse()
                    .map_err(|_| "--seconds needs a number".to_owned())?
            }
            other => return Err(format!("unknown option {other}")),
        }
        index += 2;
    }
    Ok(options)
}

fn parse_assignment(assignment: &str) -> Result<(u32, f64), String> {
    let (key, value) = assignment
        .split_once('=')
        .ok_or_else(|| format!("expected id=value, got {assignment}"))?;
    let index = if let Ok(index) = key.parse::<u32>() {
        index
    } else {
        PARAMETERS
            .iter()
            .find(|parameter| parameter.id == key)
            .map(|parameter| parameter.index)
            .ok_or_else(|| format!("no parameter is called {key}"))?
    };
    let value = value
        .parse::<f64>()
        .map_err(|_| format!("{value} is not a number"))?;
    Ok((index, value))
}

fn engine_for(options: &Options) -> Result<Engine, String> {
    let mut engine = Engine::default();
    if !engine.prepare(f64::from(SAMPLE_RATE)) {
        return Err("the engine refused the sample rate".into());
    }
    if let Some(preset) = &options.preset
        && !engine.load_preset(preset)
    {
        return Err(format!("no preset is called {preset}"));
    }
    for (index, value) in &options.overrides {
        if !engine.set_parameter(*index, *value) {
            return Err(format!("parameter {index} refused {value}"));
        }
    }
    Ok(engine)
}

/// Reads a WAV as stereo frames at the bench rate, or, with no file, makes a
/// test signal: a logarithmic sweep from 20 Hz to 20 kHz at a quarter of
/// full scale, which plays the curve back as a glissando.
fn source(options: &Options) -> Result<Vec<(f32, f32)>, String> {
    if let Some(path) = &options.input {
        let mut reader = hound::WavReader::open(path).map_err(|error| error.to_string())?;
        let spec = reader.spec();
        let channels = spec.channels.max(1) as usize;
        let scale = match spec.sample_format {
            hound::SampleFormat::Float => 1.0,
            hound::SampleFormat::Int => 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32,
        };
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .map(|sample| sample.map_err(|error| error.to_string()))
                .collect::<Result<_, _>>()?,
            hound::SampleFormat::Int => reader
                .samples::<i32>()
                .map(|sample| {
                    sample
                        .map(|value| value as f32 * scale)
                        .map_err(|error| error.to_string())
                })
                .collect::<Result<_, _>>()?,
        };
        return Ok(samples
            .chunks(channels)
            .map(|frame| (frame[0], frame.get(1).copied().unwrap_or(frame[0])))
            .collect());
    }
    let total = (options.seconds * SAMPLE_RATE as f32) as usize;
    let (low, high) = (20.0_f64, 20_000.0_f64);
    let octaves = (high / low).log2();
    let rate = f64::from(SAMPLE_RATE);
    let mut phase = 0.0_f64;
    Ok((0..total)
        .map(|n| {
            let progress = n as f64 / total.max(1) as f64;
            let frequency = low * 2.0_f64.powf(octaves * progress);
            phase += 2.0 * std::f64::consts::PI * frequency / rate;
            let x = (MEASURE_AMPLITUDE * phase.sin()) as f32;
            (x, x)
        })
        .collect())
}

fn render_command(arguments: &[String]) -> Result<(), String> {
    let options = parse_options(arguments)?;
    let output = options
        .output
        .clone()
        .ok_or("render needs --out <file.wav>")?;
    let mut engine = engine_for(&options)?;
    let frames = source(&options)?;
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&output, spec).map_err(|error| error.to_string())?;
    let mut peak = 0.0_f32;
    for (left, right) in frames {
        let (l, r) = engine.process(left, right);
        peak = peak.max(l.abs()).max(r.abs());
        writer.write_sample(l).map_err(|error| error.to_string())?;
        writer.write_sample(r).map_err(|error| error.to_string())?;
    }
    writer.finalize().map_err(|error| error.to_string())?;
    println!(
        "wrote {} (peak {:.2} dBFS)",
        output.display(),
        gain_to_db(peak)
    );
    Ok(())
}

/// The response table: one line per third octave, the gain in decibels and
/// a bar either side of zero, a character per half decibel.
fn response_command(arguments: &[String]) -> Result<(), String> {
    let options = parse_options(arguments)?;
    let mut engine = engine_for(&options)?;
    let setting = options.preset.as_deref().unwrap_or("defaults");
    println!("{setting}, {SAMPLE_RATE} Hz, sine at {MEASURE_AMPLITUDE} of full scale");
    println!("{:>9} {:>9}   {:<15}|{:<15}", "Hz", "dB", "-", "+");
    for frequency in THIRD_OCTAVES {
        let gain = response_db(&mut engine, frequency, MEASURE_AMPLITUDE);
        let cells = (gain.abs() * 2.0).round().min(15.0) as usize;
        let (left, right) = if gain < 0.0 {
            (format!("{:>15}", "#".repeat(cells)), String::new())
        } else {
            (" ".repeat(15), "#".repeat(cells))
        };
        println!("{frequency:>9.1} {gain:>9.2}   {left}|{right}");
    }
    Ok(())
}
