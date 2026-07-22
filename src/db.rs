// Database connection setup.
//
// This file handles creating the SQLite connection pool.
// SQLite is used as an embedded database — no separate server process is needed.

use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

/// Create a connection pool to the SQLite database.
///
/// The database URL is read from the `DATABASE_URL` environment variable.
/// For SQLite this is a file path, e.g. `sqlite:platform.db`.
///
/// The pool manages up to 5 concurrent connections. SQLite supports
/// multiple readers but only one writer at a time, so 5 is plenty
/// for most workloads.
pub async fn connect() -> Result<SqlitePool, sqlx::Error> {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL not set");

    SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
}
