use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{mpsc::Receiver, Arc, Mutex};

use midly::num::{u15, u24, u28, u4, u7};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

use crate::interpreter::Interpreter;
use crate::orchestrator::{EventValue, Orchestrator, Transport};
use crate::pipe::{new_pipe, Message as PipeMessage};

const SAMPLE_RATE: f64 = 44_100.0;
const MIDI_TICKS_PER_BEAT: u32 = 480;
const BLOCK_SIZE: usize = 128;
const MAX_EXPORT_SECONDS: f64 = 600.0;
const USAGE: &str = "Usage: kotoist midi <input.koto> <output.mid> [--bpm 120] [--bars 4]";

#[derive(Clone, Copy, Debug)]
struct MidiEvent {
    tick: u64,
    channel: u8,
    pitch: u8,
    velocity: u8,
    note_on: bool,
}

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{USAGE}

Defaults: 120 BPM, 4 bars in 4/4.");
        return Ok(());
    }
    if args.len() < 2 {
        return Err(USAGE.to_owned());
    }

    let input_path = PathBuf::from(&args[0]);
    let output_path = PathBuf::from(&args[1]);
    let mut bpm = 120.0_f64;
    let mut bars = 4_u32;
    let mut index = 2;
    while index < args.len() {
        let option = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("Missing value for {option}.
{USAGE}"))?;
        match option {
            "--bpm" => {
                bpm = value
                    .parse::<f64>()
                    .map_err(|_| format!("Invalid BPM: {value}"))?;
            }
            "--bars" => {
                bars = value
                    .parse::<u32>()
                    .map_err(|_| format!("Invalid bar count: {value}"))?;
            }
            _ => return Err(format!("Unknown option: {option}
{USAGE}")),
        }
        index += 2;
    }
    validate_timing(bpm, bars)?;

    let source = fs::read_to_string(&input_path)
        .map_err(|error| format!("Could not read {}: {error}", input_path.display()))?;
    let (midi, note_count) = render_midi(&source, bpm, bars)?;
    fs::write(&output_path, midi)
        .map_err(|error| format!("Could not write {}: {error}", output_path.display()))?;

    println!(
        "Wrote {note_count} note-on events to {} at {bpm} BPM for {bars} bars.",
        output_path.display()
    );
    Ok(())
}

fn validate_timing(bpm: f64, bars: u32) -> Result<(), String> {
    if !bpm.is_finite() || !(1.0..=1000.0).contains(&bpm) {
        return Err("BPM must be a finite value from 1 to 1000.".to_owned());
    }
    if !(1..=256).contains(&bars) {
        return Err("Bar count must be from 1 to 256.".to_owned());
    }
    if f64::from(bars) * 4.0 * 60.0 / bpm > MAX_EXPORT_SECONDS {
        return Err("An export cannot exceed 10 minutes; reduce the bar count or raise the tempo.".to_owned());
    }
    Ok(())
}

fn render_midi(source: &str, bpm: f64, bars: u32) -> Result<(Vec<u8>, usize), String> {
    validate_timing(bpm, bars)?;
    let (pipe_in, pipe_out) = new_pipe();
    let orchestrator = Arc::new(Mutex::new(Orchestrator::new(pipe_in.clone())));
    let mut interpreter = Interpreter::new(orchestrator.clone(), pipe_in);

    interpreter.eval_code(source);
    check_pipe(&pipe_out.receiver)?;
    interpreter.on_load();
    check_pipe(&pipe_out.receiver)?;
    interpreter.on_play(0.0, bpm);
    check_pipe(&pipe_out.receiver)?;

    let beat_length = SAMPLE_RATE * 60.0 / bpm;
    let total_samples = (f64::from(bars) * 4.0 * beat_length).ceil() as u64;
    let end_tick = u64::from(bars) * 4 * u64::from(MIDI_TICKS_PER_BEAT);
    let mut midi_events = Vec::new();
    let mut active_notes: HashMap<(u8, u8), usize> = HashMap::new();

    for block_start in (0..total_samples).step_by(BLOCK_SIZE) {
        let block_len = ((total_samples - block_start) as usize).min(BLOCK_SIZE);
        let transport = Transport {
            position: block_start as f64,
            beat_length,
        };
        let mut orchestrator = orchestrator
            .lock()
            .map_err(|_| "The Kotoist event scheduler is unavailable.".to_owned())?;

        for frame_offset in 0..block_len {
            let absolute_sample = block_start + frame_offset as u64;
            for event in orchestrator.tick(true, &transport, frame_offset) {
                for value in &event.value {
                    if let EventValue::Note(pitch, velocity, channel) = value {
                        let channel = (*channel).min(15);
                        let pitch = (*pitch).min(127);
                        let note_on = *velocity > 0;
                        midi_events.push(MidiEvent {
                            tick: sample_to_tick(absolute_sample, bpm).min(end_tick),
                            channel,
                            pitch,
                            velocity: *velocity,
                            note_on,
                        });
                        let active_key = (channel, pitch);
                        if note_on {
                            *active_notes.entry(active_key).or_default() += 1;
                        } else {
                            let remove = if let Some(count) = active_notes.get_mut(&active_key) {
                                *count = count.saturating_sub(1);
                                *count == 0
                            } else {
                                false
                            };
                            if remove {
                                active_notes.remove(&active_key);
                            }
                        }
                    }
                }
            }
        }
        drop(orchestrator);
        check_pipe(&pipe_out.receiver)?;
    }

    for ((channel, pitch), count) in active_notes {
        for _ in 0..count {
            midi_events.push(MidiEvent {
                tick: end_tick,
                channel,
                pitch,
                velocity: 0,
                note_on: false,
            });
        }
    }

    let note_count = midi_events.iter().filter(|event| event.note_on).count();
    if note_count == 0 {
        return Err("The script ran without producing MIDI notes.".to_owned());
    }
    let bytes = encode_midi(midi_events, bpm, end_tick)?;
    Ok((bytes, note_count))
}

fn sample_to_tick(sample: u64, bpm: f64) -> u64 {
    ((sample as f64 * bpm * f64::from(MIDI_TICKS_PER_BEAT)) / (60.0 * SAMPLE_RATE)).round() as u64
}

fn check_pipe(receiver: &Receiver<PipeMessage>) -> Result<(), String> {
    let mut errors = Vec::new();
    for message in receiver.try_iter() {
        match message {
            PipeMessage::Normal(text) if !text.trim().is_empty() => println!("{}", text.trim_end()),
            PipeMessage::Normal(_) => {}
            PipeMessage::Error(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("
"))
    }
}

fn encode_midi(mut events: Vec<MidiEvent>, bpm: f64, end_tick: u64) -> Result<Vec<u8>, String> {
    events.sort_by_key(|event| (event.tick, event.note_on));
    let micros_per_beat = (60_000_000.0 / bpm).round().clamp(1.0, 16_777_215.0) as u32;
    let mut track = vec![
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(micros_per_beat))),
        },
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::TimeSignature(4, 2, 24, 8)),
        },
    ];
    let mut previous_tick = 0_u64;
    for event in events {
        let tick = event.tick.min(end_tick).max(previous_tick);
        let delta = (tick - previous_tick) as u32;
        let message = if event.note_on {
            MidiMessage::NoteOn {
                key: u7::new(event.pitch),
                vel: u7::new(event.velocity.min(127)),
            }
        } else {
            MidiMessage::NoteOff {
                key: u7::new(event.pitch),
                vel: u7::new(0),
            }
        };
        track.push(TrackEvent {
            delta: u28::new(delta),
            kind: TrackEventKind::Midi {
                channel: u4::new(event.channel),
                message,
            },
        });
        previous_tick = tick;
    }
    track.push(TrackEvent {
        delta: u28::new((end_tick.saturating_sub(previous_tick)) as u32),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    let smf = Smf {
        header: Header::new(
            Format::SingleTrack,
            Timing::Metrical(u15::new(MIDI_TICKS_PER_BEAT as u16)),
        ),
        tracks: vec![track],
    };
    let mut bytes = Vec::new();
    smf.write_std(&mut bytes)
        .map_err(|error| format!("Could not encode MIDI: {error}"))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_export_runs_the_koto_scheduler_and_writes_note_events() {
        let script = "midiout({degree: 0}, 0)";
        let (bytes, note_count) = render_midi(script, 120.0, 1).unwrap();
        assert!(note_count >= 4);

        let smf = Smf::parse(&bytes).unwrap();
        let track = &smf.tracks[0];
        assert!(track.iter().any(|event| matches!(
            &event.kind,
            TrackEventKind::Midi {
                message: MidiMessage::NoteOn { .. },
                ..
            }
        )));
        assert!(track.iter().any(|event| matches!(
            &event.kind,
            TrackEventKind::Midi {
                message: MidiMessage::NoteOff { .. },
                ..
            }
        )));
    }
}
