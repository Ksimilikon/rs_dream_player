//! `extern "C"` query functions exported to mods. These return owned C strings
//! holding JSON; the caller must release them with [`free_string`].

use std::ffi::{CString, c_char};

use crate::bridge;

/// JSON describing the currently playing track:
/// `{"title":"..","artist":"..","duration_sec":N}`.
/// The returned pointer is owned by the caller and must be passed to
/// [`free_string`].
#[unsafe(no_mangle)]
pub extern "C" fn get_playlist() -> *mut c_char {
    let now = bridge::snapshot().now;
    let json = format!(
        "{{\"title\":\"{}\",\"artist\":\"{}\",\"duration_sec\":{}}}",
        escape(&now.title),
        escape(&now.artist),
        now.duration_sec
    );
    into_c_string(json)
}

/// JSON array of known playlist names, e.g. `["chill","work"]`.
/// The returned pointer is owned by the caller and must be passed to
/// [`free_string`].
#[unsafe(no_mangle)]
pub extern "C" fn get_playlists() -> *mut c_char {
    let names = bridge::snapshot().playlists;
    let body = names
        .iter()
        .map(|n| format!("\"{}\"", escape(n)))
        .collect::<Vec<_>>()
        .join(",");
    into_c_string(format!("[{body}]"))
}

/// frees a string previously returned by this API. Passing null is a no-op;
/// passing a pointer not obtained from this API is undefined behaviour.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: the pointer must come from `into_c_string` / `CString::into_raw`.
    drop(CString::from_raw(ptr));
}

fn into_c_string(s: String) -> *mut c_char {
    CString::new(s)
        .unwrap_or_else(|_| CString::new("{}").unwrap())
        .into_raw()
}

/// minimal JSON string escaping for the few fields we emit.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
