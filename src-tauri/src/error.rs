//! `AppError` — the single error type every Tauri command returns.
//!
//! It exists so commands can use `?` instead of `.map_err(|e| e.to_string())`
//! on every fallible line. It serializes to a plain string, preserving the
//! existing JS error contract across the bundler-free Rust↔JS boundary
//! (lookup.js compares the rejection against ERR_ACCESSIBILITY; manage.js
//! concatenates it into an alert; backup code matches the literal
//! "no_backup_dir"). The inner `String` is public so that match — `e.0 !=
//! "no_backup_dir"` in the backup scheduler — keeps working across modules.

use serde::Serialize;

#[derive(Debug)]
pub struct AppError(pub String);

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError(s)
    }
}

impl From<&str> for AppError {
    fn from(s: &str) -> Self {
        AppError(s.to_string())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError(e.to_string())
    }
}

impl<T> From<std::sync::PoisonError<T>> for AppError {
    fn from(e: std::sync::PoisonError<T>) -> Self {
        AppError(e.to_string())
    }
}

impl From<arboard::Error> for AppError {
    fn from(e: arboard::Error) -> Self {
        AppError(e.to_string())
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        AppError(e.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError(e.to_string())
    }
}
