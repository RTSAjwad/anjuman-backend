// Auth middleware — the `AuthUser` extractor.
//
// This is the mechanism that protects routes. Any handler that takes
// `AuthUser(claims): AuthUser` as a parameter will automatically:
//
//   1. Extract the `Authorization: Bearer <token>` header from the request.
//   2. Verify the JWT signature and expiration.
//   3. Check the token hasn't been revoked (logged out).
//   4. Reject the request with 401 Unauthorized if anything is wrong.
//   5. Pass the decoded `Claims` to the handler if everything is valid.
//
// This means adding auth to a new route is as simple as adding one
// parameter to the handler function — no middleware configuration needed.
//
// ## Usage
//
// ```ignore
// use crate::auth::AuthUser;
//
// // This route is automatically protected:
// async fn my_handler(AuthUser(claims): AuthUser) -> impl IntoResponse {
//     // claims.sub is the authenticated user's id
//     // claims.school_id is their school
//     // claims.role is their role
// }
// ```

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};

use crate::state::AppState;

use super::jwt::{AuthError, Claims, verify_token};

// ---------------------------------------------------------------------------
// AuthUser extractor
// ---------------------------------------------------------------------------

/// An Axum extractor that authenticates a request via JWT bearer token.
///
/// `AuthUser` implements `FromRequestParts` for `AppState`, which means
/// it can access the database pool through Axum's state mechanism.
/// If extraction fails, the handler is never called and a 401 response
/// is returned instead.
///
/// The inner `Claims` value is available to the handler via destructuring:
/// `AuthUser(claims)` gives you `claims: Claims`.
#[derive(Debug, Clone)]
pub struct AuthUser(pub Claims);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AuthRejection;

    /// Extract and validate the JWT from the request's Authorization header.
    ///
    /// This runs before the handler. It:
    ///   1. Extracts the Bearer token from the Authorization header.
    ///   2. Verifies the JWT signature and expiration.
    ///   3. Checks the `revoked_tokens` table to see if the user logged out.
    ///
    /// If it succeeds, the handler receives the `Claims`.
    /// If it fails, Axum returns the `AuthRejection` response.
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Step 1: Extract the `Authorization: Bearer <token>` header.
        // `TypedHeader` parses the header value and validates that it
        // uses the Bearer scheme (not Basic, Digest, etc.).
        let TypedHeader(auth_header) =
            TypedHeader::<Authorization<Bearer>>::from_request_parts(parts, state)
                .await
                .map_err(|_| AuthRejection {
                    status: StatusCode::UNAUTHORIZED,
                    message: "Missing or malformed Authorization header".into(),
                })?;

        // Step 2: Verify the token's signature, expiration, and revocation
        // status. This now checks the database to see if the token has
        // been revoked (user logged out).
        let claims = verify_token(auth_header.token(), &state.db)
            .await
            .map_err(|e| match e {
                AuthError::TokenExpired => AuthRejection {
                    status: StatusCode::UNAUTHORIZED,
                    message: "Token has expired".into(),
                },
                AuthError::TokenRevoked => AuthRejection {
                    status: StatusCode::UNAUTHORIZED,
                    message: "Token has been revoked (logged out)".into(),
                },
                _ => AuthRejection {
                    status: StatusCode::UNAUTHORIZED,
                    message: "Invalid token".into(),
                },
            })?;

        // Step 3: Success — hand the claims to the handler.
        Ok(AuthUser(claims))
    }
}

// ---------------------------------------------------------------------------
// AuthRejection — the 401 response
// ---------------------------------------------------------------------------

/// The error response returned when authentication fails.
///
/// It produces a JSON body like `{"error": "Invalid token"}` with
/// the appropriate HTTP status code (401).
pub struct AuthRejection {
    status: StatusCode,
    message: String,
}

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": self.message,
        });

        (self.status, axum::Json(body)).into_response()
    }
}
