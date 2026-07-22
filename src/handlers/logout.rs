// Logout handler.
//
// Since JWTs are stateless, "logout" means adding the token's unique
// ID (`jti`) to the `revoked_tokens` table. Any subsequent request
// with this token will be rejected even if the token hasn't expired yet.

use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;

use crate::{
    auth::{self, AuthUser},
    state::AppState,
};

#[derive(Serialize)]
pub struct LogoutResponse {
    message: &'static str,
}

/// `POST /auth/logout` — Revoke the current JWT.
///
/// The token is extracted from the Authorization header by `AuthUser`,
/// then its `jti` is inserted into `revoked_tokens`. The token is
/// invalidated immediately — it won't work even before its expiry time.
///
/// This endpoint requires authentication (the `AuthUser` extractor).
pub async fn logout(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<LogoutResponse>, StatusCode> {
    auth::revoke_token(&claims.jti, claims.sub, claims.exp as i64, &state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(LogoutResponse {
        message: "Logged out successfully",
    }))
}
