// Deck options preset management.
//
// Teachers and admins can create, view, update, and delete deck options
// presets. Presets are scoped to a school, and decks reference a preset via
// `options_id`.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::{
    auth::AuthUser,
    deck_options::{self, DeckOptions},
    handlers::{reviews::parse_steps, users::UserRole},
    state::AppState,
};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateDeckOptions {
    pub name: String,
    /// Anki-style step string, e.g. "1m 10m". Parsed into seconds.
    #[serde(default = "default_learning_steps")]
    pub learning_steps: String,
    #[serde(default = "default_relearning_steps")]
    pub relearning_steps: String,
    #[serde(default = "default_retention")]
    pub desired_retention: f64,
    #[serde(default)]
    pub bury_new: bool,
    #[serde(default)]
    pub bury_review: bool,
    #[serde(default)]
    pub bury_interday: bool,
    #[serde(default = "default_new_per_day")]
    pub new_per_day: i64,
    #[serde(default = "default_review_per_day")]
    pub review_per_day: i64,
}

#[derive(Deserialize)]
pub struct UpdateDeckOptions {
    pub name: Option<String>,
    pub learning_steps: Option<String>,
    pub relearning_steps: Option<String>,
    pub desired_retention: Option<f64>,
    pub bury_new: Option<bool>,
    pub bury_review: Option<bool>,
    pub bury_interday: Option<bool>,
    pub new_per_day: Option<i64>,
    pub review_per_day: Option<i64>,
}

fn default_learning_steps() -> String {
    "1m 10m".to_string()
}
fn default_relearning_steps() -> String {
    "10m".to_string()
}
fn default_retention() -> f64 {
    0.9
}
fn default_new_per_day() -> i64 {
    20
}
fn default_review_per_day() -> i64 {
    200
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn check_teacher_or_admin(claims: &crate::auth::Claims) -> Result<(), (StatusCode, &'static str)> {
    match claims.role {
        UserRole::Admin | UserRole::Teacher => Ok(()),
        UserRole::Student => Err((
            StatusCode::FORBIDDEN,
            "Only teachers and admins can manage deck options",
        )),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /deck-options` — List all presets in the school.
pub async fn list_deck_options(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<DeckOptions>>, (StatusCode, &'static str)> {
    let rows = sqlx::query!(
        "SELECT id FROM deck_options WHERE school_id = ? ORDER BY name",
        claims.school_id
    )
    .fetch_all(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let mut options = Vec::new();
    for row in rows {
        let opt = deck_options::get_options(&state.db, row.id.expect("id is NOT NULL"))
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;
        options.push(opt);
    }

    Ok(Json(options))
}

/// `GET /deck-options/:id` — Get a single preset.
pub async fn get_deck_options(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<DeckOptions>, (StatusCode, &'static str)> {
    let opt = deck_options::get_options(&state.db, id)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Deck options not found"))?;

    if opt.school_id != claims.school_id {
        return Err((StatusCode::NOT_FOUND, "Deck options not found"));
    }

    Ok(Json(opt))
}

/// `POST /deck-options` — Create a new preset.
pub async fn create_deck_options(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateDeckOptions>,
) -> Result<(StatusCode, Json<DeckOptions>), (StatusCode, String)> {
    check_teacher_or_admin(&claims).map_err(|(s, m)| (s, m.to_string()))?;

    if body.name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Name is required".to_string()));
    }

    let learning_steps =
        parse_steps(&body.learning_steps).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let relearning_steps =
        parse_steps(&body.relearning_steps).map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let learning_csv = deck_options::steps_to_csv(&learning_steps);
    let relearning_csv = deck_options::steps_to_csv(&relearning_steps);
    let bury_new = body.bury_new as i64;
    let bury_review = body.bury_review as i64;
    let bury_interday = body.bury_interday as i64;

    let result = sqlx::query!(
        "INSERT INTO deck_options (school_id, name, learning_steps, relearning_steps, desired_retention, bury_new, bury_review, bury_interday, new_per_day, review_per_day) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        claims.school_id,
        body.name,
        learning_csv,
        relearning_csv,
        body.desired_retention,
        bury_new,
        bury_review,
        bury_interday,
        body.new_per_day,
        body.review_per_day,
    )
    .execute(&state.db)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            (
                StatusCode::CONFLICT,
                "A deck options preset with that name already exists".to_string(),
            )
        } else {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        }
    })?;

    let opt = deck_options::get_options(&state.db, result.last_insert_rowid())
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to fetch created deck options".to_string(),
            )
        })?;

    Ok((StatusCode::CREATED, Json(opt)))
}

/// `PATCH /deck-options/:id` — Update a preset.
pub async fn update_deck_options(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateDeckOptions>,
) -> Result<Json<DeckOptions>, (StatusCode, String)> {
    check_teacher_or_admin(&claims).map_err(|(s, m)| (s, m.to_string()))?;

    let existing = deck_options::get_options(&state.db, id)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Deck options not found".to_string()))?;
    if existing.school_id != claims.school_id {
        return Err((StatusCode::NOT_FOUND, "Deck options not found".to_string()));
    }

    if let Some(name) = &body.name {
        if name.is_empty() {
            return Err((StatusCode::BAD_REQUEST, "Name cannot be empty".to_string()));
        }
        sqlx::query!("UPDATE deck_options SET name = ? WHERE id = ?", name, id)
            .execute(&state.db)
            .await
            .map_err(|e| {
                if e.to_string().contains("UNIQUE") {
                    (
                        StatusCode::CONFLICT,
                        "A deck options preset with that name already exists".to_string(),
                    )
                } else {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Database error".to_string(),
                    )
                }
            })?;
    }

    if let Some(steps) = &body.learning_steps {
        let parsed = parse_steps(steps).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
        let csv = deck_options::steps_to_csv(&parsed);
        sqlx::query!(
            "UPDATE deck_options SET learning_steps = ? WHERE id = ?",
            csv,
            id
        )
        .execute(&state.db)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;
    }

    if let Some(steps) = &body.relearning_steps {
        let parsed = parse_steps(steps).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
        let csv = deck_options::steps_to_csv(&parsed);
        sqlx::query!(
            "UPDATE deck_options SET relearning_steps = ? WHERE id = ?",
            csv,
            id
        )
        .execute(&state.db)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;
    }

    if let Some(retention) = body.desired_retention {
        if !(0.0..=1.0).contains(&retention) {
            return Err((
                StatusCode::BAD_REQUEST,
                "desired_retention must be between 0 and 1".to_string(),
            ));
        }
        sqlx::query!(
            "UPDATE deck_options SET desired_retention = ? WHERE id = ?",
            retention,
            id
        )
        .execute(&state.db)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;
    }

    if let Some(v) = body.bury_new {
        let v_i64 = v as i64;
        sqlx::query!(
            "UPDATE deck_options SET bury_new = ? WHERE id = ?",
            v_i64,
            id
        )
        .execute(&state.db)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;
    }

    if let Some(v) = body.bury_review {
        let v_i64 = v as i64;
        sqlx::query!(
            "UPDATE deck_options SET bury_review = ? WHERE id = ?",
            v_i64,
            id
        )
        .execute(&state.db)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;
    }

    if let Some(v) = body.bury_interday {
        let v_i64 = v as i64;
        sqlx::query!(
            "UPDATE deck_options SET bury_interday = ? WHERE id = ?",
            v_i64,
            id
        )
        .execute(&state.db)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;
    }

    if let Some(v) = body.new_per_day {
        sqlx::query!(
            "UPDATE deck_options SET new_per_day = ? WHERE id = ?",
            v,
            id
        )
        .execute(&state.db)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;
    }

    if let Some(v) = body.review_per_day {
        sqlx::query!(
            "UPDATE deck_options SET review_per_day = ? WHERE id = ?",
            v,
            id
        )
        .execute(&state.db)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;
    }

    let opt = deck_options::get_options(&state.db, id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to fetch updated deck options".to_string(),
            )
        })?;

    Ok(Json(opt))
}

/// `DELETE /deck-options/:id` — Delete a preset.
///
/// Decks referencing this preset fall back to defaults (options_id = NULL).
pub async fn delete_deck_options(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;

    let existing = deck_options::get_options(&state.db, id)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Deck options not found"))?;
    if existing.school_id != claims.school_id {
        return Err((StatusCode::NOT_FOUND, "Deck options not found"));
    }

    sqlx::query!("DELETE FROM deck_options WHERE id = ?", id)
        .execute(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(Json(
        serde_json::json!({ "message": "Deck options deleted" }),
    ))
}
