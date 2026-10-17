//! JNI bindings of the core for Android: a thin layer over the C ABI of the
//! `api` crate, so Kotlin can call the core. The crate compiles only for
//! `target_os = "android"` and is linked into the core `.so` (the root crate
//! `dream_player`), so the app loads a single library:
//! `System.loadLibrary("dream_player")`.
//!
//! # contract
//!
//! every native method is a static method of `dev.dreamplayer.core.Native`
//! (symbols `Java_dev_dreamplayer_core_Native_<method>`). The Kotlin side of
//! the contract — the `external fun` declarations and the data classes these
//! functions construct — lives in `bindings/android/` of this repository and
//! must match this crate exactly: a renamed class, method or constructor
//! parameter breaks the binding at runtime (`UnsatisfiedLinkError` /
//! `NoSuchMethodError`).
//!
//! # conventions
//!
//! - every function upgrades `EnvUnowned` with `with_env` (panics and JNI
//!   errors don't unwind into the JVM) and resolves errors with
//!   `ThrowRuntimeExAndDefault`: a JNI failure becomes a Java
//!   `RuntimeException`, core failures are returned as values (`false`,
//!   `null`, status codes) exactly like in the C ABI.
//! - large data (packets, audio files) never goes through the Java heap: it is
//!   read from / written to file descriptors passed from Kotlin
//!   (`ParcelFileDescriptor.fd`). The descriptor is borrowed — Kotlin closes it.
//! - `cdc_*` statuses are returned as `Int` with the same codes as
//!   `api::cdc::FFICdcStatus` (`CdcStatus` in Kotlin).

#![cfg(target_os = "android")]

mod cdc;
mod engine;
mod idx;
mod util;
