use std::path::PathBuf;

use libloading::{Library, Symbol};

pub mod input;
pub mod output;

#[derive(Debug)]
pub enum ModStatus {
    Loaded,
    Failed(String),
}

#[derive(Debug)]
pub struct Mod {
    pub name: String,
    pub path: PathBuf,
    pub status: ModStatus,
    _lib: Option<Library>,
}

#[derive(Debug)]
pub struct ModManager {
    pub mods: Vec<Mod>,
}
impl ModManager {
    pub fn new() -> Self {
        Self { mods: Vec::new() }
    }

    /// arg: mods_dir - name dir of mods
    pub fn load_mods(&mut self, mods_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
        let path = std::env::current_dir().unwrap().join(mods_dir);
        if !path.exists() {
            std::fs::create_dir_all(&path)?;
        }
        let entries = std::fs::read_dir(&path)?;
        println!("path mods {}", &path.display());
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && is_library(&path) {
                let mod_name = path.file_stem().unwrap().to_string_lossy().into_owned();

                match self.try_launch_mod(&path) {
                    Ok(lib) => {
                        self.mods.push(Mod {
                            name: mod_name,
                            path,
                            status: ModStatus::Loaded,
                            _lib: Some(lib),
                        });
                    }
                    Err(e) => {
                        self.mods.push(Mod {
                            name: mod_name,
                            path,
                            status: ModStatus::Failed(e.to_string()),
                            _lib: None,
                        });
                    }
                }
            }
        }

        Ok(())
    }
    pub fn try_launch_mod(&self, path: &PathBuf) -> Result<Library, Box<dyn std::error::Error>> {
        unsafe {
            let lib = Library::new(path)?;
            // marker
            let launch: Symbol<unsafe extern "C" fn()> = lib.get(b"launch")?;
            launch();
            Ok(lib)
        }
    }
}

fn is_library(path: &PathBuf) -> bool {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    ext == "so" || ext == "dll" || ext == "dylib"
}
