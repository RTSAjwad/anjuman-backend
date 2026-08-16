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

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
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

#[derive(Deserialize)]
pub struct SetFlag {
    /// Flag value 0-7. 0 clears the flag.
    pub flag: i32,
}

#[derive(Serialize)]
pub struct FlagResponse {
    pub card_id: i64,
    pub flag: i64,
}

// ---------------------------------------------------------------------------
// Learning & relearning step parsing
// ---------------------------------------------------------------------------

/// Parse an Anki-style step string into seconds.
///
/// Supports units:
///   - `s` = seconds, `m` = minutes, `h` = hours, `d` = days
///   - bare numbers default to minutes (Anki convention)
///   - decimals are allowed (e.g. `1.5d`)
///
/// Example: `"1m 1d"` → `[60, 86400]`.
#[allow(dead_code)] // used once a config endpoint is added
pub fn parse_steps(input: &str) -> Result<Vec<i64>, String> {
    let mut steps = Vec::new();
    for token in input.split_whitespace() {
        // Find the split point between the numeric part and the unit suffix.
        let split_at = token
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(token.len());
        let (num_str, unit) = token.split_at(split_at);

        let value: f64 = num_str
            .parse()
            .map_err(|_| format!("Invalid step value: '{token}'"))?;

        let multiplier: f64 = match unit {
            "" | "m" => 60.0,
            "s" => 1.0,
            "h" => 3600.0,
            "d" => 86400.0,
            other => return Err(format!("Unknown step unit: '{other}'")),
        };

        let seconds = (value * multiplier).round() as i64;
        if seconds <= 0 {
            return Err(format!("Step must be positive: '{token}'"));
        }
        steps.push(seconds);
    }

    if steps.is_empty() {
        return Err("At least one step is required".to_string());
    }

    Ok(steps)
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

    // Fetch the current scheduling state (and the card's deck).
    let current = sqlx::query!(
        r#"
        SELECT scs.state, scs.stability, scs.difficulty, scs.last_reviewed_at,
               scs.reps, scs.lapses, scs.step_index, c.deck_id
        FROM student_card_states scs
        JOIN cards c ON c.id = scs.card_id
        WHERE scs.student_id = ? AND scs.card_id = ?
        "#,
        claims.sub,
        body.card_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "Card state not found"))?;

    // Resolve this deck's effective scheduling options (preset or defaults).
    let options = crate::deck_options::options_for_deck(&state.db, current.deck_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load deck options",
            )
        })?;
    let learning_steps = &options.learning_steps;
    let relearning_steps = &options.relearning_steps;
    let desired_retention = options.desired_retention;

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

    let next_states = fsrs
        .next_states(previous_memory, desired_retention as f32, elapsed_days)
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
    let interval_fsrs_secs = interval_days * 86400;

    // Determine the new state, step index, and due timestamp.
    //
    // Learning/relearning cards follow Anki's step model:
    //   - Steps are fixed intervals (in seconds).
    //   - "Good" advances one step; graduating past the last step → review.
    //   - "Easy" graduates immediately.
    //   - "Again" resets to the first step.
    //   - "Hard" advances one step like "Good" (FSRS still records lower
    //     stability, affecting future intervals after graduation).
    // Review cards lapse back to relearning on "Again".
    let mut step_index = current.step_index;
    let new_state: String;
    let due_at: i64;

    match current.state.as_str() {
        "new" => match body.rating {
            4 => {
                // Easy: graduate immediately.
                new_state = "review".to_string();
                step_index = 0;
                due_at = now + interval_fsrs_secs;
            }
            3 => {
                // Good: advance one step; graduate if past last.
                step_index += 1;
                if step_index >= learning_steps.len() as i64 {
                    new_state = "review".to_string();
                    step_index = 0;
                    due_at = now + interval_fsrs_secs;
                } else {
                    new_state = "learning".to_string();
                    due_at = now + learning_steps[step_index as usize];
                }
            }
            _ => {
                // Again or Hard: stay in learning at first step.
                new_state = "learning".to_string();
                step_index = 0;
                due_at = now + learning_steps[0];
            }
        },
        "learning" => match body.rating {
            1 => {
                // Again: reset to first step.
                step_index = 0;
                due_at = now + learning_steps[0];
                new_state = "learning".to_string();
            }
            2 | 3 => {
                // Hard or Good: advance one step; graduate if past last.
                step_index += 1;
                if step_index >= learning_steps.len() as i64 {
                    new_state = "review".to_string();
                    step_index = 0;
                    due_at = now + interval_fsrs_secs;
                } else {
                    new_state = "learning".to_string();
                    due_at = now + learning_steps[step_index as usize];
                }
            }
            _ => {
                // Easy: graduate immediately.
                new_state = "review".to_string();
                step_index = 0;
                due_at = now + interval_fsrs_secs;
            }
        },
        "review" => match body.rating {
            1 => {
                // Again: lapse to relearning.
                new_state = "relearning".to_string();
                step_index = 0;
                due_at = now + relearning_steps[0];
            }
            _ => {
                // Hard/Good/Easy: stay review.
                new_state = "review".to_string();
                due_at = now + interval_fsrs_secs;
            }
        },
        "relearning" => match body.rating {
            1 => {
                // Again: restart relearning steps.
                step_index = 0;
                due_at = now + relearning_steps[0];
                new_state = "relearning".to_string();
            }
            2 | 3 => {
                // Hard or Good: advance one step; graduate if past last.
                step_index += 1;
                if step_index >= relearning_steps.len() as i64 {
                    new_state = "review".to_string();
                    step_index = 0;
                    due_at = now + interval_fsrs_secs;
                } else {
                    new_state = "relearning".to_string();
                    due_at = now + relearning_steps[step_index as usize];
                }
            }
            _ => {
                // Easy: graduate immediately.
                new_state = "review".to_string();
                step_index = 0;
                due_at = now + interval_fsrs_secs;
            }
        },
        other => {
            new_state = other.to_string();
            due_at = now + interval_fsrs_secs;
        }
    }

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
            step_index = ?, due_at = ?, last_reviewed_at = ?, reps = ?, lapses = ?
        WHERE student_id = ? AND card_id = ?
        "#,
        new_state,
        next.memory.stability,
        next.memory.difficulty,
        step_index,
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

/// `PATCH /cards/:card_id/flag` — Set or clear a flag on a card.
///
/// Flags are per-student, per-card markers (Anki-style).
/// Values: 0 (none), 1 (red), 2 (orange), 3 (green), 4 (blue),
/// 5 (pink), 6 (turquoise), 7 (purple).
pub async fn set_flag(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(card_id): Path<i64>,
    Json(body): Json<SetFlag>,
) -> Result<Json<FlagResponse>, (StatusCode, &'static str)> {
    if !(0..=7).contains(&body.flag) {
        return Err((StatusCode::BAD_REQUEST, "Flag must be between 0 and 7"));
    }

    let result = sqlx::query!(
        "UPDATE student_card_states SET flag = ? WHERE student_id = ? AND card_id = ?",
        body.flag,
        claims.sub,
        card_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Card state not found"));
    }

    Ok(Json(FlagResponse {
        card_id,
        flag: body.flag as i64,
    }))
}
