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
}
