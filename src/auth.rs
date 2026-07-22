// Authentication module.
//
// This module provides everything related to authentication:
//
//   `jwt.rs`       — JWT token creation and verification.
//   `middleware.rs` — The `AuthUser` extractor that protects routes.
//
// Re-export both so callers can `use crate::auth::AuthUser` directly
// instead of reaching into submodules.

mod jwt;
mod middleware;

pub use jwt::*;
pub use middleware::*;
