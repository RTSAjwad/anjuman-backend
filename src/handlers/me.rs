// "Me" handler — get the authenticated user's profile.
//
// This is the canonical "who am I?" endpoint. The client calls this
// to verify their token is still valid and to get their user info.
//
// It also serves as the reference implementation for how to use the
// `AuthUser` extractor in any future handler.

use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;

use crate::{auth::AuthUser, state::AppState};

// ---------------------------------------------------------------------------
// Response DTO
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct MeResponse {
    pub id: i64,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub role: String,
    pub school_id: i64,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// `GET /me` — Return the authenticated user's profile.
///
/// This endpoint is protected by the `AuthUser` extractor. If the request
/// has no valid JWT, the handler is never called — Axum returns 401 instead.
///
/// The `claims` come from the JWT payload: `claims.sub` is the user's id,
/// `claims.school_id` is their school, and `claims.role` is their role.
pub async fn me(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<MeResponse>, StatusCode> {
    // Look up the user by the id embedded in the JWT.
    // We re-fetch from the database rather than using the JWT claims alone
    // because the token might be valid but the user might have been deleted.
    let row = sqlx::query!(
        r#"
        SELECT id, email, first_name, last_name, role, school_id
        FROM users
        WHERE id = ?
        "#,
        claims.sub
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(MeResponse {
        id: row.id,
        email: row.email,
        first_name: row.first_name,
        last_name: row.last_name,
        role: row.role,
        school_id: row.school_id,
    }))
}
