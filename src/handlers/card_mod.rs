// Card modification endpoints: suspend, unsuspend, bury, unbury, reschedule.
//
// These mirror Anki's card-level controls:
//   - Suspend:     permanently exclude a card from study until unsuspended.
//   - Bury:        hide a card until the next day (auto-reappears tomorrow).
//   - Reschedule:  manually override a card's due date.
//
// All operations are scoped to the authenticated student and their own
// per-card scheduling state (student_card_states).

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
pub struct RescheduleBody {
    /// Absolute due timestamp (Unix epoch seconds).
    pub due_at: Option<i64>,
    /// Relative due offset in days from now (alternative to `due_at`).
    pub days: Option<i64>,
}

#[derive(Deserialize)]
pub struct MoveCardBody {
    /// The target deck to move the card into.
    pub deck_id: i64,
}

#[derive(Serialize)]
pub struct MoveCardResponse {
    pub card_id: i64,
    pub deck_id: i64,
}

#[derive(Serialize)]
pub struct CardModResponse {
    pub card_id: i64,
    pub suspended: i64,
    pub buried_until: Option<i64>,
    pub due_at: Option<i64>,
    pub state: String,
}

#[derive(Serialize)]
pub struct NoteModResponse {
    pub note_id: i64,
    pub cards_affected: i64,
    /// IDs of the cards that were affected.
    pub card_ids: Vec<i64>,
    /// 1 if suspended, 0 otherwise (suspend operation).
    pub suspended: i64,
    /// Unix seconds until cards reappear; null if not buried (bury operation).
    pub buried_until: Option<i64>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn start_of_tomorrow() -> i64 {
    let now = now_secs();
    now - (now % 86400) + 86400
}

/// Ensure a `student_card_states` row exists for the given (student, card).
///
/// Creates a `'new'` state row if one doesn't exist yet, so per-student
/// operations (suspend, bury, flag, reschedule) work on cards the student
/// has never studied. Mirrors the study flow's `ensure_card_states_for_deck`.
async fn ensure_card_state(
    db: &sqlx::SqlitePool,
    student_id: i64,
    card_id: i64,
) -> Result<(), (StatusCode, &'static str)> {
    sqlx::query!(
        "INSERT OR IGNORE INTO student_card_states (student_id, card_id, state, stability, difficulty, reps, lapses) VALUES (?, ?, 'new', 0.0, 0.0, 0, 0)",
        student_id,
        card_id
    )
    .execute(db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(())
}

/// Fetch the current card state for the authenticated student.
async fn fetch_state(
    db: &sqlx::SqlitePool,
    student_id: i64,
    card_id: i64,
) -> Result<(String, Option<i64>, i64, Option<i64>), (StatusCode, &'static str)> {
    let row = sqlx::query!(
        "SELECT state, due_at, suspended, buried_until FROM student_card_states WHERE student_id = ? AND card_id = ?",
        student_id,
        card_id
    )
    .fetch_optional(db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "Card state not found"))?;

    Ok((row.state, row.due_at, row.suspended, row.buried_until))
}

/// Fetch the card IDs belonging to a note for the given student.
async fn fetch_note_card_ids(
    db: &sqlx::SqlitePool,
    student_id: i64,
    note_id: i64,
) -> Result<Vec<i64>, (StatusCode, &'static str)> {
    let rows = sqlx::query!(
        "SELECT scs.card_id FROM student_card_states scs JOIN cards c ON c.id = scs.card_id WHERE scs.student_id = ? AND c.note_id = ? ORDER BY scs.card_id",
        student_id,
        note_id
    )
    .fetch_all(db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(rows.into_iter().map(|r| r.card_id).collect())
}

/// Ensure `student_card_states` rows exist for every card of a note.
async fn ensure_note_card_states(
    db: &sqlx::SqlitePool,
    student_id: i64,
    note_id: i64,
) -> Result<(), (StatusCode, &'static str)> {
    sqlx::query!(
        r#"
        INSERT OR IGNORE INTO student_card_states
            (student_id, card_id, state, stability, difficulty, reps, lapses)
        SELECT ?, c.id, 'new', 0.0, 0.0, 0, 0
        FROM cards c
        WHERE c.note_id = ?
        "#,
        student_id,
        note_id
    )
    .execute(db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /cards/:card_id/suspend` — Suspend a card (exclude from study).
pub async fn suspend(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(card_id): Path<i64>,
) -> Result<Json<CardModResponse>, (StatusCode, &'static str)> {
    ensure_card_state(&state.db, claims.sub, card_id).await?;

    let result = sqlx::query!(
        "UPDATE student_card_states SET suspended = 1 WHERE student_id = ? AND card_id = ?",
        claims.sub,
        card_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Card not found"));
    }

    let (state, due_at, suspended, buried_until) =
        fetch_state(&state.db, claims.sub, card_id).await?;

    Ok(Json(CardModResponse {
        card_id,
        suspended,
        buried_until,
        due_at,
        state,
    }))
}

/// `POST /cards/:card_id/unsuspend` — Unsuspend a card.
pub async fn unsuspend(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(card_id): Path<i64>,
) -> Result<Json<CardModResponse>, (StatusCode, &'static str)> {
    ensure_card_state(&state.db, claims.sub, card_id).await?;

    let result = sqlx::query!(
        "UPDATE student_card_states SET suspended = 0 WHERE student_id = ? AND card_id = ?",
        claims.sub,
        card_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Card not found"));
    }

    let (state, due_at, suspended, buried_until) =
        fetch_state(&state.db, claims.sub, card_id).await?;

    Ok(Json(CardModResponse {
        card_id,
        suspended,
        buried_until,
        due_at,
        state,
    }))
}

/// `POST /cards/:card_id/bury` — Bury a card until tomorrow.
pub async fn bury(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(card_id): Path<i64>,
) -> Result<Json<CardModResponse>, (StatusCode, &'static str)> {
    let until = start_of_tomorrow();
    ensure_card_state(&state.db, claims.sub, card_id).await?;

    let result = sqlx::query!(
        "UPDATE student_card_states SET buried_until = ? WHERE student_id = ? AND card_id = ?",
        until,
        claims.sub,
        card_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Card not found"));
    }

    let (state, due_at, suspended, buried_until) =
        fetch_state(&state.db, claims.sub, card_id).await?;

    Ok(Json(CardModResponse {
        card_id,
        suspended,
        buried_until,
        due_at,
        state,
    }))
}

/// `POST /cards/:card_id/unbury` — Unbury a card immediately.
pub async fn unbury(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(card_id): Path<i64>,
) -> Result<Json<CardModResponse>, (StatusCode, &'static str)> {
    ensure_card_state(&state.db, claims.sub, card_id).await?;

    let result = sqlx::query!(
        "UPDATE student_card_states SET buried_until = NULL WHERE student_id = ? AND card_id = ?",
        claims.sub,
        card_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Card not found"));
    }

    let (state, due_at, suspended, buried_until) =
        fetch_state(&state.db, claims.sub, card_id).await?;

    Ok(Json(CardModResponse {
        card_id,
        suspended,
        buried_until,
        due_at,
        state,
    }))
}

/// `PATCH /cards/:card_id/reschedule` — Manually set a card's due date.
///
/// Accepts either an absolute `due_at` (Unix seconds) or a relative `days`
/// offset from now. If both are provided, `due_at` takes precedence.
pub async fn reschedule(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(card_id): Path<i64>,
    Json(body): Json<RescheduleBody>,
) -> Result<Json<CardModResponse>, (StatusCode, &'static str)> {
    let new_due_at = if let Some(due_at) = body.due_at {
        due_at
    } else if let Some(days) = body.days {
        now_secs() + days * 86400
    } else {
        return Err((StatusCode::BAD_REQUEST, "Provide either due_at or days"));
    };

    ensure_card_state(&state.db, claims.sub, card_id).await?;

    let result = sqlx::query!(
        "UPDATE student_card_states SET due_at = ? WHERE student_id = ? AND card_id = ?",
        new_due_at,
        claims.sub,
        card_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Card not found"));
    }

    let (state, due_at, suspended, buried_until) =
        fetch_state(&state.db, claims.sub, card_id).await?;

    Ok(Json(CardModResponse {
        card_id,
        suspended,
        buried_until,
        due_at,
        state,
    }))
}

/// `POST /notes/:note_id/suspend` — Suspend all cards belonging to a note.
///
/// Bulk operation: applies `suspended = 1` to every card of the note for the
/// authenticated student. Affects only the student's own scheduling state.
pub async fn suspend_note(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(note_id): Path<i64>,
) -> Result<Json<NoteModResponse>, (StatusCode, &'static str)> {
    ensure_note_card_states(&state.db, claims.sub, note_id).await?;

    let result = sqlx::query!(
        "UPDATE student_card_states SET suspended = 1 WHERE student_id = ? AND card_id IN (SELECT id FROM cards WHERE note_id = ?)",
        claims.sub,
        note_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let affected = result.rows_affected();
    if affected == 0 {
        return Err((StatusCode::NOT_FOUND, "No cards found for this note"));
    }

    let card_ids = fetch_note_card_ids(&state.db, claims.sub, note_id).await?;

    Ok(Json(NoteModResponse {
        note_id,
        cards_affected: affected as i64,
        card_ids,
        suspended: 1,
        buried_until: None,
    }))
}

/// `POST /notes/:note_id/bury` — Bury all cards belonging to a note until tomorrow.
///
/// Bulk operation: applies `buried_until = start_of_tomorrow` to every card of
/// the note for the authenticated student.
pub async fn bury_note(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(note_id): Path<i64>,
) -> Result<Json<NoteModResponse>, (StatusCode, &'static str)> {
    let until = start_of_tomorrow();
    ensure_note_card_states(&state.db, claims.sub, note_id).await?;

    let result = sqlx::query!(
        "UPDATE student_card_states SET buried_until = ? WHERE student_id = ? AND card_id IN (SELECT id FROM cards WHERE note_id = ?)",
        until,
        claims.sub,
        note_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let affected = result.rows_affected();
    if affected == 0 {
        return Err((StatusCode::NOT_FOUND, "No cards found for this note"));
    }

    let card_ids = fetch_note_card_ids(&state.db, claims.sub, note_id).await?;

    Ok(Json(NoteModResponse {
        note_id,
        cards_affected: affected as i64,
        card_ids,
        suspended: 0,
        buried_until: Some(until),
    }))
}

/// `PATCH /cards/:card_id/deck` — Move a card to another deck.
///
/// This lets a note's cards be split across decks, matching Anki's model.
/// The target deck must be in the same school and visible to the caller.
pub async fn move_card(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(card_id): Path<i64>,
    Json(body): Json<MoveCardBody>,
) -> Result<Json<MoveCardResponse>, (StatusCode, &'static str)> {
    // Validate the target deck exists in the same school and is accessible.
    let deck = sqlx::query!(
        "SELECT id FROM decks WHERE id = ? AND school_id = ?",
        body.deck_id,
        claims.school_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "Deck not found"))?;

    // Authorize: caller must be owner, admin, or collaborator on the target deck.
    crate::handlers::decks::check_deck_collaborator(&state.db, deck.id, claims.school_id, &claims)
        .await?;

    let result = sqlx::query!(
        "UPDATE cards SET deck_id = ? WHERE id = ?",
        body.deck_id,
        card_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Card not found"));
    }

    Ok(Json(MoveCardResponse {
        card_id,
        deck_id: body.deck_id,
    }))
}
