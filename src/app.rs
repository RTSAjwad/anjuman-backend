// Application router construction.
//
// This is a thin wrapper that keeps `main.rs` free of routing details.
// The actual route definitions live in `routes.rs`.

use crate::{routes, state::AppState};

/// Build the complete Axum router with all routes and state attached.
///
/// This is the single place where the application is assembled before
/// being passed to `axum::serve` in `main.rs`.
pub fn app(state: AppState) -> axum::Router {
    routes::router(state)
}
