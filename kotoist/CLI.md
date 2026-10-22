# Kotoist MIDI command-line exporter

The Windows MIDI CLI uses the same Koto interpreter and pattern scheduler as the plugin. It writes a one-track Standard MIDI File; it does not render audio.

## Usage

    kotoist.exe midi input.koto output.mid [--bpm 120] [--bars 4]

Defaults are 120 BPM and four bars of 4/4. The bar count can be set from 1 to 256. A single export is limited to 10 minutes.

## Example

The included midi-export-demo.koto file emits a repeating MIDI note. From the extracted CLI folder, run:

    kotoist.exe midi midi-export-demo.koto demo.mid --bpm 120 --bars 4

The source script may use the same Kotoist midiout patterns as the VST. The generated file can be opened in a DAW or another MIDI editor.
