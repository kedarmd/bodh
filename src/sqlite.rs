//! Shared SQLite connection setup for the local review workspace.
use rusqlite::{Connection, Error, ErrorCode};
use std::time::{Duration, Instant};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn configure(db: &Connection) -> Result<(), Error> {
    db.busy_timeout(BUSY_TIMEOUT)?;
    db.execute_batch("PRAGMA foreign_keys=ON;")?;

    // Changing journal mode is a database-wide operation. SQLite can return
    // BUSY immediately when two processes first open a new state file, even
    // with a busy timeout configured. Retry only this idempotent setup step;
    // all later writes use bounded SQLite transactions and their own timeout.
    let deadline = Instant::now() + BUSY_TIMEOUT;
    loop {
        match db.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0)) {
            Ok(_) => return Ok(()),
            Err(error) if is_busy(&error) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}

fn is_busy(error: &Error) -> bool {
    matches!(
        error,
        Error::SqliteFailure(failure, _)
            if matches!(failure.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}
