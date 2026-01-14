use std::error::Error;

pub trait PlayerAgentOS {
    fn send_meta_data(&self) -> Result<(), Box<dyn Error>>;
    fn next_song(&self) -> Result<(), Box<dyn Error>>;
    fn prev_song(&self) -> Result<(), Box<dyn Error>>;
    fn pause_song(&self) -> Result<(), Box<dyn Error>>;
}
