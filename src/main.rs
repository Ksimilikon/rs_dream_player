use std::io::Cursor;

use rodio::{Decoder, OutputStream};

use crate::audio::track::Track;

mod audio;
mod cmd_docmsg;
mod traits;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 2 {
        // show --help
        println!("Welcome dream player");
        return;
    }

    let song_path = &args[1];
    let track = match Track::from_file(song_path) {
        Ok(track) => track,
        Err(e) => {
            println!("Error::read file={}, {}", song_path, e.to_string());
            return;
        }
    };
    println!(
        "Трек загружен: {} Mбайт",
        track.get().len() as f32 / 1024. / 1024.
    );

    let stream_handle = rodio::OutputStreamBuilder::open_default_stream().unwrap();
    let sink = rodio::Sink::connect_new(&stream_handle.mixer());
    let cursor = Cursor::new(track.get().clone());
    let source = Decoder::new(cursor).unwrap();

    sink.append(source);
    sink.sleep_until_end();
}
