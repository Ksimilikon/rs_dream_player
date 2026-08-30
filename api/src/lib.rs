use core::{Core, dirs::Dirs};
use std::{
    error::Error,
    ffi::{CStr, OsStr, c_char},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

pub mod idx;
pub mod mem;
pub mod player;

static GLOBAL_STATE: OnceLock<Mutex<Core>> = OnceLock::new();

pub(crate) fn ptr_to_path(ptr: *const c_char) -> Result<PathBuf, Box<dyn Error>> {
    unsafe {
        if ptr.is_null() {
            return Err("FFI::null pointer for string".into());
        }
        let c_str = CStr::from_ptr(ptr);
        let bytes = c_str.to_bytes();
        let os_str = OsStr::from_bytes(bytes);
        let path = PathBuf::from(os_str);
        Ok(path)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn init(
    cache_dir: *const c_char,
    config_dir: *const c_char,
    data_dir: *const c_char,
    music_dir: *const c_char,
    need_db: bool,
) {
    let dirs = Dirs {
        cache: ptr_to_path(cache_dir).unwrap(),
        config: ptr_to_path(config_dir).unwrap(),
        data: ptr_to_path(data_dir).unwrap(),
        music_dir: ptr_to_path(music_dir).unwrap(),
    };
    let _ = GLOBAL_STATE.set(Mutex::new(Core::new(dirs, need_db)));
}
