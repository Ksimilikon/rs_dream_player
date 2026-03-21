use std::{path::PathBuf, sync::Arc, thread};

use libloading::{Library, Symbol};
pub mod player;

extern "C" fn on_mod_close() {
    println!("[Core] Модуль сообщил о закрытии окна.");
}

#[derive(Debug)]
pub enum ModStatus {
    Running,
    Failed(String),
}

#[derive(Debug)]
pub struct Mod {
    pub name: String,
    pub path: PathBuf,
    pub status: ModStatus,
    pub lib: Option<Arc<Library>>,
}

#[derive(Debug)]
pub struct ModManager {
    pub mods: Vec<Mod>,
}

impl ModManager {
    pub fn new() -> Self {
        Self { mods: Vec::new() }
    }

    pub fn load_mods(&mut self, mods_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
        let path = std::env::current_dir()?.join(mods_dir);
        if !path.exists() {
            std::fs::create_dir_all(&path)?;
        }

        let entries = std::fs::read_dir(&path)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && is_library(&path) {
                let mod_name = path.file_stem().unwrap().to_string_lossy().into_owned();

                match self.spawn_mod(path.clone()) {
                    Ok(lib_arc) => {
                        self.mods.push(Mod {
                            name: mod_name,
                            path,
                            status: ModStatus::Running,
                            lib: Some(lib_arc),
                        });
                    }
                    Err(e) => {
                        self.mods.push(Mod {
                            name: mod_name,
                            path,
                            status: ModStatus::Failed(e.to_string()),
                            lib: None,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn spawn_mod(&self, path: PathBuf) -> Result<Arc<Library>, Box<dyn std::error::Error>> {
        unsafe {
            let lib = Arc::new(Library::new(&path)?);
            let lib_for_thread = Arc::clone(&lib);

            type LaunchFn = unsafe extern "C" fn(extern "C" fn());
            let launch_symbol: Symbol<LaunchFn> = lib.get(b"launch")?;
            let launch_ptr = *launch_symbol;

            thread::spawn(move || {
                let _keep_alive = lib_for_thread;
                launch_ptr(on_mod_close);
                println!("[Core] {} is closed", &path.to_string_lossy());
            });

            Ok(lib)
        }
    }
}

fn is_library(path: &PathBuf) -> bool {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    ext == "so" || ext == "dll" || ext == "dylib"
}
