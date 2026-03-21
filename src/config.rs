#[derive(Clone)]
pub enum Themes {
    Dark,
    Light,
}

pub const NAME_APPLICATION: &str = "Core::dream_player";
pub struct Config {
    theme: Themes,
    count_cache: u32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
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

    pub fn set_theme(&mut self, theme: Themes) {
        self.theme = theme;
    }
    pub fn set_count_cache(&mut self, count: u32) {
        self.count_cache = count;
    }
}
