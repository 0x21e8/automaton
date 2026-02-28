#![allow(dead_code)]

#[cfg(not(target_arch = "wasm32"))]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::cell::RefMut;

type SqliteResult<T> = Result<T, String>;

pub trait SqliteConnection {
    fn execute_batch(&mut self, sql: &str) -> SqliteResult<()>;
}

pub fn with_connection<F, R>(f: F) -> SqliteResult<R>
where
    F: FnOnce(&mut dyn SqliteConnection) -> SqliteResult<R>,
{
    #[cfg(target_arch = "wasm32")]
    {
        ic_rusqlite::with_connection(|connection| {
            let mut adapter = WasmSqliteConnection { connection };
            f(&mut adapter)
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        HOST_CONNECTION_OPEN.with(|open| {
            *open.borrow_mut() = true;
        });
        let mut adapter = HostSqliteConnection;
        f(&mut adapter)
    }
}

pub fn init() -> SqliteResult<()> {
    with_connection(|connection| {
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS sqlite_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
        )
    })
}

pub fn close() {
    #[cfg(target_arch = "wasm32")]
    {
        ic_rusqlite::close_connection();
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        HOST_CONNECTION_OPEN.with(|open| {
            *open.borrow_mut() = false;
        });
    }
}

#[cfg(target_arch = "wasm32")]
struct WasmSqliteConnection<'a> {
    connection: RefMut<'a, ic_rusqlite::Connection>,
}

#[cfg(target_arch = "wasm32")]
impl SqliteConnection for WasmSqliteConnection<'_> {
    fn execute_batch(&mut self, sql: &str) -> SqliteResult<()> {
        self.connection
            .execute_batch(sql)
            .map_err(|error| error.to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static HOST_CONNECTION_OPEN: RefCell<bool> = const { RefCell::new(false) };
}

#[cfg(not(target_arch = "wasm32"))]
struct HostSqliteConnection;

#[cfg(not(target_arch = "wasm32"))]
impl SqliteConnection for HostSqliteConnection {
    fn execute_batch(&mut self, _sql: &str) -> SqliteResult<()> {
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn host_connection_is_open() -> bool {
    HOST_CONNECTION_OPEN.with(|open| *open.borrow())
}

#[cfg(test)]
mod tests {
    use super::{close, host_connection_is_open, init, with_connection};

    #[test]
    fn connection_lifecycle() {
        close();
        assert!(!host_connection_is_open());

        init().expect("sqlite init should succeed");
        assert!(host_connection_is_open());

        close();
        assert!(!host_connection_is_open());

        with_connection(|_| Ok(())).expect("with_connection should open connection");
        assert!(host_connection_is_open());
    }
}
