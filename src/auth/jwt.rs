// JWT (JSON Web Token) creation and verification.
//
// JWTs are the core of the authentication system. When a user logs in,
// the server creates a signed JWT containing their identity and role.
// The client stores this token and sends it back on every request.
//
// The server verifies the token's signature and expiration without
// needing a database lookup, which makes auth checks very fast.
//
// ## Token revocation
//
// JWTs are stateless, so "logout" requires a server-side blocklist.
// Each token has a unique `jti` (JWT ID) claim. When a user logs out,
// the `jti` is inserted into the `revoked_tokens` table. Every
// authenticated request checks this table before accepting the token.
//
// ## Token format (Claims)
//
// The JWT payload is a `Claims` struct serialised as JSON:
//
//   {
//     "jti": "a1b2c3...",  // unique token ID (for revocation)
//     "sub": 42,           // user id (standard JWT "subject" claim)
//     "school_id": 1,      // which school they belong to
//     "role": "teacher",   // for authorisation checks
//     "iat": 1690000000,   // issued-at timestamp
//     "exp": 1690086400    // expiration timestamp (24h after iat)
//   }
//
// Tokens are signed with HMAC-SHA256 using a secret key from the
// `JWT_SECRET` environment variable.

use std::env;

use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::handlers::users::UserRole;

// ---------------------------------------------------------------------------
// Claims (JWT payload)
// ---------------------------------------------------------------------------

/// The data embedded inside every JWT.
///
/// This is what gets signed and verified. The client never needs to
/// decode this — they just store the opaque token string and send it back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// JWT ID — a unique identifier for this specific token.
    /// Used for revocation: when the user logs out, this ID is added
    /// to the `revoked_tokens` table so the token can't be reused.
    pub jti: String,
    /// Subject — the user's database id (standard JWT claim).
    pub sub: i64,
    /// The school this user belongs to. Used for data isolation —
    /// queries should always filter by `school_id` to prevent
    /// cross-school data access.
    pub school_id: i64,
    /// The user's role. Embedded in the token so authorisation checks
    /// (e.g. "is this user a teacher?") don't need a database lookup.
    pub role: UserRole,
    /// Issued-at timestamp (Unix epoch seconds).
    pub iat: usize,
    /// Expiration timestamp (Unix epoch seconds). Default: 24 hours after `iat`.
    pub exp: usize,
}

// ---------------------------------------------------------------------------
// Token creation
// ---------------------------------------------------------------------------

/// Create a signed JWT for a user.
///
/// The token is valid for 24 hours from the time of creation.
/// Each token gets a unique `jti` (UUID v4) so it can be individually
/// revoked on logout.
///
/// Returns the encoded token string (e.g. `"eyJhbGciOiJIUzI1NiJ9..."`).
pub fn create_token(user_id: i64, school_id: i64, role: UserRole) -> Result<String, AuthError> {
    let secret = jwt_secret();

    // Set issued-at to now and expiration to 24 hours from now.
    let now = Utc::now();
    let exp = now + Duration::hours(24);

    let claims = Claims {
        // Generate a unique ID for this token so it can be revoked.
        jti: Uuid::new_v4().to_string(),
        sub: user_id,
        school_id,
        role,
        iat: now.timestamp() as usize,
        exp: exp.timestamp() as usize,
    };

    // Encode the claims into a JWT string using HMAC-SHA256.
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|_| AuthError::TokenCreation)
}

// ---------------------------------------------------------------------------
// Token verification
// ---------------------------------------------------------------------------

/// Verify a JWT's signature and expiration, then check the revocation
/// blocklist. Returns the decoded `Claims` only if the token is valid
/// AND has not been revoked.
pub async fn verify_token(token: &str, db: &SqlitePool) -> Result<Claims, AuthError> {
    let secret = jwt_secret();

    // Step 1: Verify signature and expiration.
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::TokenExpired,
        _ => AuthError::TokenInvalid,
    })?;

    let claims = token_data.claims;

    // Step 2: Check if this token has been revoked (logged out).
    // `EXISTS(...)` returns 0 or 1, which sqlx maps to a Rust integer.
    let revoked: i64 = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM revoked_tokens WHERE jti = ?) AS "exists!: i64""#,
        claims.jti
    )
    .fetch_one(db)
    .await
    .map_err(|_| AuthError::TokenInvalid)?;

    if revoked != 0 {
        return Err(AuthError::TokenRevoked);
    }

    Ok(claims)
}

/// Revoke a token by inserting its JWT ID into the blocklist.
///
/// After this call, the token will be rejected by `verify_token`
/// even if it hasn't expired yet.
pub async fn revoke_token(
    jti: &str,
    user_id: i64,
    expires_at: i64,
    db: &SqlitePool,
) -> Result<(), AuthError> {
    sqlx::query!(
        "INSERT OR IGNORE INTO revoked_tokens (jti, user_id, expires_at) VALUES (?, ?, ?)",
        jti,
        user_id,
        expires_at,
    )
    .execute(db)
    .await
    .map_err(|_| AuthError::TokenCreation)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Secret key management
// ---------------------------------------------------------------------------

/// Read the JWT signing secret from the environment.
///
/// Falls back to a hard-coded dev secret if `JWT_SECRET` is not set.
/// **Do not use the default secret in production** — it's easy to guess.
fn jwt_secret() -> String {
    env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret-change-in-production".to_string())
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur during token creation or verification.
#[derive(Debug)]
pub enum AuthError {
    /// Failed to encode the token (internal error).
    TokenCreation,
    /// The token's `exp` claim is in the past — user needs to log in again.
    TokenExpired,
    /// Signature verification failed or the token is malformed.
    TokenInvalid,
    /// The token is valid but has been explicitly revoked (user logged out).
    TokenRevoked,
}
