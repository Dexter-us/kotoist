use nih_plug::prelude::*;

use kotoist::Kotoist;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "midi" || arg == "--midi")
    {
        if let Err(error) = kotoist::run_midi_cli(&args[1..]) {
            eprintln!("{error}");
            std::process::exit(2);
        }
        return;
    }

    nih_export_standalone::<Kotoist>();
}
