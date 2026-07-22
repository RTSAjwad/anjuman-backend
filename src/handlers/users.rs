// Users handler — registration and password management.
//
// This file handles user creation (registration) and provides the
// password hashing/verification utilities used by the login flow.

use axum::{Json, extract::State, http::StatusCode};

use serde::{Deserialize, Serialize};

use crate::state::AppState;

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString},
};
// Use the OsRng from password-hash's bundled rand_core to avoid version mismatch.
use password_hash::rand_core::OsRng;

// ---------------------------------------------------------------------------
// Role
// ---------------------------------------------------------------------------

/// A user's role in the system.
///
/// This enum mirrors the CHECK constraint on `users.role`:
///
///   CHECK (role IN ('admin', 'teacher', 'student'))
///
/// The `#[serde(rename_all = "lowercase")]` attribute means the JSON
/// representation uses lowercase strings:
///
///   Admin    → `"admin"`
///   Teacher  → `"teacher"`
///   Student  → `"student"`
///
/// The `Display` impl does the same conversion for database storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Admin,
    Teacher,
    Student,
}

impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserRole::Admin => write!(f, "admin"),
            UserRole::Teacher => write!(f, "teacher"),
            UserRole::Student => write!(f, "student"),
        }
    }
}

// ---------------------------------------------------------------------------
// DTOs (Data Transfer Objects)
// ---------------------------------------------------------------------------

/// Expected JSON body for creating a new user.
///
/// The `role` field uses the `UserRole` enum, so the client must send
/// one of `"admin"`, `"teacher"`, or `"student"`. Serde will reject
/// any other value with a 422 Unprocessable Entity.
#[derive(Deserialize)]
pub struct CreateUser {
    /// The school this user belongs to.
    pub school_id: i64,
    /// Unique email address (doubles as the login identifier).
    pub email: String,
    /// Plain-text password. This is never stored — only the hash is kept.
    pub password: String,
    /// The user's role: `"admin"`, `"teacher"`, or `"student"`.
    pub role: UserRole,
    /// First name.
    pub first_name: String,
    /// Last name.
    pub last_name: String,
}

/// User info returned after successful registration.
#[derive(Serialize)]
pub struct User {
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

/// `POST /users` — Create a new user (registration).
///
/// The password is hashed with Argon2 before storage. The plain-text
/// password is never written to the database or logged.
///
/// Returns 409 Conflict if a user with the given email already exists.
pub async fn create_user(
    State(state): State<AppState>,
    Json(body): Json<CreateUser>,
) -> Result<Json<User>, (StatusCode, String)> {
    // Hash the password with Argon2.
    //
    // Argon2 is the winner of the Password Hashing Competition and is
    // designed to be resistant to GPU/ASIC cracking. A random salt is
    // generated for each password, so two users with the same password
    // will have different hashes.
    let password_hash =
        hash_password(&body.password).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // Convert the UserRole enum to a string for database storage.
    let role_str = body.role.to_string();

    // Insert the new user row.
    //
    // The `created_at` field uses `unixepoch()` — SQLite's function
    // that returns the current Unix timestamp in seconds.
    let result = sqlx::query!(
        r#"
        INSERT INTO users
        (
            school_id,
            email,
            password_hash,
            role,
            first_name,
            last_name,
            created_at
        )
        VALUES
        (?, ?, ?, ?, ?, ?, unixepoch())
        "#,
        body.school_id,
        body.email,
        password_hash,
        role_str,
        body.first_name,
        body.last_name
    )
    .execute(&state.db)
    .await
    .map_err(|e| {
        // SQLite returns a UNIQUE constraint error when the email already
        // exists. We check for the word "UNIQUE" in the error message
        // because the exact error code varies by SQLite version.
        let msg = if e.to_string().contains("UNIQUE") {
            "A user with that email already exists".into()
        } else {
            format!("Database error: {e}")
        };
        (StatusCode::CONFLICT, msg)
    })?;

    // `last_insert_rowid()` returns the `INTEGER PRIMARY KEY` that SQLite
    // auto-generated for the new row.
    Ok(Json(User {
        id: result.last_insert_rowid(),
        email: body.email,
        first_name: body.first_name,
        last_name: body.last_name,
        role: role_str,
        school_id: body.school_id,
    }))
}

// ---------------------------------------------------------------------------
// Password helpers
// ---------------------------------------------------------------------------

/// Hash a plain-text password using Argon2.
///
/// Returns the encoded hash string suitable for database storage.
/// The hash includes the algorithm parameters and the random salt,
/// so everything needed for verification is in a single string.
pub fn hash_password(password: &str) -> Result<String, String> {
    // Generate a cryptographically random salt.
    let salt = SaltString::generate(&mut OsRng);

    // Hash the password with Argon2 using default parameters
    // (memory: 19 MiB, iterations: 2, parallelism: 1).
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("Password hashing failed: {e}"))
}

/// Verify a plain-text password against an Argon2 hash.
///
/// Returns:
///   `Ok(true)`  — password matches
///   `Ok(false)` — password does not match (wrong password)
///   `Err(...)`  — the stored hash is malformed (shouldn't happen)
///
/// This runs in constant time to prevent timing attacks.
pub fn verify_password(password: &str, hash: &str) -> Result<bool, String> {
    use argon2::password_hash::PasswordVerifier;

    // Parse the stored hash string (which includes algorithm parameters
    // and the salt) back into a structured `PasswordHash`.
    let parsed_hash = argon2::password_hash::PasswordHash::new(hash)
        .map_err(|e| format!("Invalid password hash: {e}"))?;

    // Verify the password against the hash.
    //
    // Argon2's `verify_password` is constant-time: it doesn't short-circuit
    // on the first wrong byte, which prevents attackers from measuring
    // response times to guess the password character by character.
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .map(|_| true)
        .or_else(|e| match e {
            argon2::password_hash::Error::Password => Ok(false),
            other => Err(format!("Password verification error: {other}")),
        })
}
