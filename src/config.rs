use std::path::{Path, PathBuf};

use directories::BaseDirs;

#[derive(Clone)]
pub enum Themes {
    Dark,
    Light,
}

pub const NAME_APPLICATION: &str = "Core::dream_player";
pub struct Config {
    path: PathBuf,
    theme: Themes,
    count_cache: u32,
}
impl Default for Config {
    fn default() -> Self {
        let path = match BaseDirs::new() {
            Some(p) => p.config_dir().join("dream_player"),
            None => panic!("your OS doesnt have VAR for configs"),
        };
        Self {
            path,
            theme: Themes::Dark,
            count_cache: 10,
        }
    }
}

impl Config {
    pub fn get_theme(&self) -> Themes {
        self.theme.clone()
    }
    pub fn get_count_cache(&self) -> u32 {
        self.count_cache
    }
    pub fn get_path(&self) -> &Path {
        &self.path
    }

    pub fn set_theme(&mut self, theme: Themes) {
        self.theme = theme;
    }
    pub fn set_count_cache(&mut self, count: u32) {
        self.count_cache = count;
    }
}
