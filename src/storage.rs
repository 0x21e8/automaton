#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod sqlite;
#[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
#[path = "storage/sqlite_unsupported.rs"]
pub mod sqlite;
pub mod stable;
