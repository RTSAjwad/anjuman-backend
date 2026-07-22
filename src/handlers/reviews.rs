// Review handler.
//
// A student submits a rating (1-4) for a card they just reviewed.
// The FSRS algorithm calculates the next review interval and updates
// the student's scheduling state. A review record is created for
// analytics and future parameter optimisation.
//
// ## Ratings
//
//  1 — Again (failed, show again soon)
//  2 — Hard  (recalled with significant difficulty)
//  3 — Good  (recalled with acceptable effort)
//  4 — Easy  (recalled effortlessly)

use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};

use crate::{auth::AuthUser, state::AppState};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct SubmitReview {
    pub card_id: i64,
    pub rating: i32,
    pub response_time_ms: Option<i64>,
}

#[derive(Serialize)]
pub struct ReviewResponse {
    pub card_id: i64,
    pub state: String,
    pub due_at: Option<i64>,
    pub stability: f64,
    pub difficulty: f64,
    pub reps: i64,
    pub lapses: i64,
    pub interval_days: i64,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// `POST /reviews` — Submit a card review rating.
pub async fn submit_review(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Json(body): Json<SubmitReview>,
) -> Result<Json<ReviewResponse>, (StatusCode, &'static str)> {
    if !(1..=4).contains(&body.rating) {
        return Err((StatusCode::BAD_REQUEST, "Rating must be between 1 and 4"));
    }

    // Fetch the current scheduling state.
    let current = sqlx::query!(
        r#"
        SELECT state, stability, difficulty, last_reviewed_at, reps, lapses
        FROM student_card_states
        WHERE student_id = ? AND card_id = ?
        "#,
        claims.sub,
        body.card_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "Card state not found"))?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // Calculate elapsed days since last review.
    let elapsed_days = if let Some(last_reviewed) = current.last_reviewed_at {
        ((now - last_reviewed).max(0) as f64 / 86400.0) as u32
    } else {
        0
    };

    // Build the previous memory state for FSRS.
    let previous_memory = if current.reps > 0 {
        Some(fsrs::MemoryState {
            stability: current.stability as f32,
            difficulty: current.difficulty as f32,
        })
    } else {
        None
    };

    // Run the FSRS scheduler.
    let fsrs = fsrs::FSRS::default();
    let desired_retention = 0.9;

    let next_states = fsrs
        .next_states(previous_memory, desired_retention, elapsed_days)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "FSRS scheduling failed"))?;

    // Choose the output based on the rating.
    let next = match body.rating {
        1 => next_states.again,
        2 => next_states.hard,
        3 => next_states.good,
        4 => next_states.easy,
        _ => unreachable!(),
    };

    let interval_days = next.interval.round().max(1.0) as i64;
    // Again (rating 1): set due_at to now so the card stays in the queue
    // across devices/sessions until the student gives a passing rating.
    // The FSRS stability/difficulty are still saved for future scheduling.
    let due_at = if body.rating == 1 {
        now
    } else {
        now + interval_days * 86400
    };

    // Determine the new state string.
    // Rating 1 (Again): review/relearning cards lapse to relearning.
    // New cards stay in learning (go through learning steps like Anki).
    let new_state = if body.rating == 1 {
        match current.state.as_str() {
            "review" | "relearning" => "relearning",
            _ => "learning",
        }
    } else {
        match current.state.as_str() {
            // For new cards: graduate to review if the FSRS interval is >= 1 day.
            // Otherwise stay in learning (matches Anki's FSRS behaviour).
            "new" => {
                if next.interval >= 1.0 {
                    "review"
                } else {
                    "learning"
                }
            }
            // Learning and relearning: only graduate to review when the
            // FSRS interval is >= 1 day, matching Anki's graduation logic.
            "learning" | "relearning" => {
                if next.interval >= 1.0 {
                    "review"
                } else {
                    current.state.as_str()
                }
            }
            other => other,
        }
    };

    let new_reps = current.reps + 1;
    let new_lapses = if body.rating == 1 {
        current.lapses + 1
    } else {
        current.lapses
    };

    // Update the scheduling state.
    sqlx::query!(
        r#"
        UPDATE student_card_states
        SET state = ?, stability = ?, difficulty = ?,
            due_at = ?, last_reviewed_at = ?, reps = ?, lapses = ?
        WHERE student_id = ? AND card_id = ?
        "#,
        new_state,
        next.memory.stability,
        next.memory.difficulty,
        due_at,
        now,
        new_reps,
        new_lapses,
        claims.sub,
        body.card_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    // Record the review.
    sqlx::query!(
        "INSERT INTO reviews (student_id, card_id, rating, reviewed_at, response_time_ms) VALUES (?, ?, ?, ?, ?)",
        claims.sub,
        body.card_id,
        body.rating,
        now,
        body.response_time_ms
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(Json(ReviewResponse {
        card_id: body.card_id,
        state: new_state.to_string(),
        due_at: Some(due_at),
        stability: next.memory.stability as f64,
        difficulty: next.memory.difficulty as f64,
        reps: new_reps,
        lapses: new_lapses,
        interval_days,
    }))
}
