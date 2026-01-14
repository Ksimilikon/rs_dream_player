pub enum PlayerCommands {
    Play,
    Pause,
    Stop,
    Next,
    Prev,
    SetVolume(f32),
    Seek(std::time::Duration),
}
