// Shared application state.
//
// `AppState` is injected into every request handler by Axum.
// It holds long-lived resources that all handlers need — currently
// just the database connection pool.
//
// Because `SqlitePool` is `Clone` (it's backed by an `Arc` internally),
// `AppState` can also be `Clone`. Axum clones it once per request so
// handlers can access it without any locking.

use sqlx::SqlitePool;

/// The application state available to every request handler.
#[derive(Clone)]
pub struct AppState {
    /// The SQLite connection pool.
    ///
    /// Handlers borrow connections from this pool to run queries.
    /// The pool manages up to 5 concurrent connections — more than
    /// that will queue until a connection is free.
    pub db: SqlitePool,
}
