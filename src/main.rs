use std::io::Cursor;

use rodio::Decoder;

use crate::audio::{playlist::Playlist, song::track::Track};

mod audio;
mod cmd_docmsg;
mod traits;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        // show --help
        println!("Welcome dream player");
        return;
    }

    let song_path = &args[1];
    let mut playlist = Playlist::from_dir(song_path).unwrap();
    playlist.debug_songs_size();
    playlist.next();

    let stream_handle = rodio::OutputStreamBuilder::open_default_stream().unwrap();
    let sink = rodio::Sink::connect_new(&stream_handle.mixer());

    playlist.play(&sink);
    playlist.debug_songs_size();

    sink.sleep_until_end();
}
