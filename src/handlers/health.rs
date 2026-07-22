// Health check handler.
//
// A simple liveness endpoint. Load balancers and monitoring tools
// hit this to verify the server is running.

use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct Health {
    status: &'static str,
}

/// `GET /health` — Always returns `{"status": "ok"}`.
///
/// This endpoint does not touch the database or any external service.
/// It's a simple liveness check, not a readiness check.
pub async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}
