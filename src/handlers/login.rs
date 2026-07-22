// Login handler.
//
// This is the entry point for authentication. The client sends an email
// and password, and the server returns a signed JWT that the client uses
// for all subsequent requests.
//
// ## Flow
//
//   1. Look up the user by email in the database.
//   2. Verify the password against the stored Argon2 hash.
//   3. Parse the user's role (admin / teacher / student).
//   4. Create a JWT containing the user's id, school_id, and role.
//   5. Return the token plus basic user info.
//
// The client should store the token and send it as:
//   Authorization: Bearer <token>
// on every authenticated request.

use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};

use crate::{
    auth,
    handlers::users::{UserRole, verify_password},
    state::AppState,
};

// ---------------------------------------------------------------------------
// DTOs (Data Transfer Objects)
// ---------------------------------------------------------------------------

/// Expected JSON body for the login request.
#[derive(Deserialize)]
pub struct LoginRequest {
    /// The user's email address (used as the login identifier).
    pub email: String,
    /// The user's plain-text password.
    pub password: String,
}

/// JSON response returned on successful login.
#[derive(Serialize)]
pub struct LoginResponse {
    /// The JWT access token. Valid for 24 hours.
    pub token: String,
    /// Basic information about the authenticated user.
    pub user: UserInfo,
}

/// Public user info returned in the login response.
#[derive(Serialize)]
pub struct UserInfo {
    pub id: i64,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub role: String,
    pub school_id: i64,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /auth/login` — Authenticate with email + password, receive a JWT.
///
/// Returns a 401 Unauthorized if the email doesn't exist or the password
/// is wrong. The error message is deliberately vague ("Invalid email or
/// password") to avoid leaking whether an email is registered.
pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, &'static str)> {
    // Step 1: Look up the user by email.
    //
    // `query!` is a compile-time checked SQL macro. It verifies the query
    // against the actual database schema so you get type errors at build
    // time instead of runtime.
    let row = sqlx::query!(
        r#"
        SELECT id, school_id, email, first_name, last_name, password_hash, role
        FROM users
        WHERE email = ?
        "#,
        body.email
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    // `fetch_optional` returns `None` if no row matched, so we convert
    // that to a 401. We use the same error message as a wrong password
    // to prevent user enumeration attacks.
    let row = row.ok_or((StatusCode::UNAUTHORIZED, "Invalid email or password"))?;

    // Step 2: Verify the password against the stored Argon2 hash.
    //
    // `verify_password` returns `Ok(true)` if the password matches,
    // `Ok(false)` if it doesn't, and `Err(...)` if the hash is corrupted.
    let valid = verify_password(&body.password, &row.password_hash)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Authentication error"))?;

    if !valid {
        return Err((StatusCode::UNAUTHORIZED, "Invalid email or password"));
    }

    // Step 3: Parse the role string from the database into our `UserRole` enum.
    //
    // The database stores roles as strings ("admin", "teacher", "student").
    // We convert to the enum so we can embed a typed value in the JWT.
    let role = match row.role.as_str() {
        "admin" => UserRole::Admin,
        "teacher" => UserRole::Teacher,
        "student" => UserRole::Student,
        _ => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Invalid role in database",
            ));
        }
    };

    // Step 4: Create the JWT.
    //
    // The token embeds the user's id, school, and role. This means
    // subsequent requests can be authorised without a database lookup.
    //
    // NOTE: sqlx infers `id` (INTEGER PRIMARY KEY) as `Option<i64>` but
    // `school_id` (INTEGER NOT NULL) as plain `i64` — an inconsistency in
    // sqlx's SQLite driver. We handle the `Option` with `.expect()` because
    // PRIMARY KEY columns are implicitly NOT NULL and can never be missing.
    let token = auth::create_token(
        row.id.expect("user.id is NOT NULL (PRIMARY KEY) in schema"),
        row.school_id,
        role,
    )
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create token"))?;

    // Step 5: Return the token and user info.
    //
    // The client should store the `token` field and send it as an
    // `Authorization: Bearer <token>` header on every subsequent request.
    Ok(Json(LoginResponse {
        token,
        user: UserInfo {
            id: row.id.expect("user.id is NOT NULL (PRIMARY KEY) in schema"),
            email: row.email,
            first_name: row.first_name,
            last_name: row.last_name,
            role: row.role,
            school_id: row.school_id,
        },
    }))
}
