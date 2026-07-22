// Study session handlers.
//
// Two endpoints for retrieving cards to study:
//
//   GET /decks/:id/study  — Cards for a published deck (self-study).
//   GET /study/due        — All due cards across all the student's decks.
//
// Cards are ordered by priority:
//   1. New cards (never seen)
//   2. Overdue reviews (due_at in the past)
//   3. Due-now reviews (due_at <= now)
//
// Each card includes a predicted_interval map showing the estimated
// interval (in days) for each possible rating before the student clicks.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Serialize;

use crate::{auth::AuthUser, handlers::decks, note_types, state::AppState};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct StudyCard {
    pub card_id: i64,
    pub front: String,
    pub back: String,
    pub state: String,
    pub due_at: Option<i64>,
    pub stability: f64,
    pub difficulty: f64,
    pub reps: i64,
    pub lapses: i64,
    /// Which deck this card belongs to, for display context.
    pub deck_title: String,
    /// Predicted interval (in days) until next review for each rating.
    /// Key: "1" (Again), "2" (Hard), "3" (Good), "4" (Easy).
    /// Null for new cards where FSRS can't predict (no prior state).
    pub predicted_interval: Option<HashMap<String, i64>>,
}

#[derive(Serialize)]
pub struct StudySession {
    pub deck_id: Option<i64>,
    pub deck_title: Option<String>,
    pub cards: Vec<StudyCard>,
    pub total_cards: i64,
    pub reviewed_count: i64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn ensure_card_states_for_deck(
    db: &sqlx::SqlitePool,
    student_id: i64,
    deck_id: i64,
) -> Result<(), StatusCode> {
    sqlx::query!(
        r#"
        INSERT OR IGNORE INTO student_card_states
            (student_id, card_id, state, stability, difficulty, reps, lapses)
        SELECT ?, c.id, 'new', 0.0, 0.0, 0, 0
        FROM cards c
        JOIN notes n ON n.id = c.note_id
        WHERE n.deck_id = ?
        "#,
        student_id,
        deck_id
    )
    .execute(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(())
}

/// Convert database rows to StudyCard DTOs, computing predicted intervals
/// and rendering card front/back from note templates.
async fn rows_to_study_cards(
    db: &sqlx::SqlitePool,
    rows: Vec<CardRow>,
) -> Result<Vec<StudyCard>, StatusCode> {
    let fsrs = fsrs::FSRS::default();
    let desired_retention = 0.9;

    let mut cards = Vec::new();
    for c in rows {
        // Render the card front/back from the note type templates.
        let nt = note_types::get_note_type(db, c.note_type_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let fields: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&c.fields_json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let rendered = note_types::render_card(&nt.templates, c.template_index, &fields)
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

        // Compute predicted intervals.
        let predicted_interval = if c.reps > 0 {
            let elapsed = match c.due_at {
                Some(due) => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64;
                    ((now - due).max(0) as f64 / 86400.0) as u32
                }
                None => 0,
            };

            let previous = Some(fsrs::MemoryState {
                stability: c.stability as f32,
                difficulty: c.difficulty as f32,
            });

            fsrs.next_states(previous, desired_retention, elapsed)
                .ok()
                .map(|next| {
                    let mut map = HashMap::new();
                    map.insert("1".to_string(), next.again.interval.round().max(1.0) as i64);
                    map.insert("2".to_string(), next.hard.interval.round().max(1.0) as i64);
                    map.insert("3".to_string(), next.good.interval.round().max(1.0) as i64);
                    map.insert("4".to_string(), next.easy.interval.round().max(1.0) as i64);
                    map
                })
        } else {
            let mut map = HashMap::new();
            map.insert("1".to_string(), 0);
            map.insert("2".to_string(), 0);
            map.insert("3".to_string(), 1);
            map.insert("4".to_string(), 4);
            Some(map)
        };

        cards.push(StudyCard {
            card_id: c.id.expect("card.id is NOT NULL"),
            front: rendered.front,
            back: rendered.back,
            state: c.state,
            due_at: c.due_at,
            stability: c.stability,
            difficulty: c.difficulty,
            reps: c.reps,
            lapses: c.lapses,
            deck_title: c.deck_title,
            predicted_interval,
        });
    }

    Ok(cards)
}

/// A row from the card + state + note query.
struct CardRow {
    id: Option<i64>,
    template_index: i64,
    note_type_id: i64,
    fields_json: String,
    state: String,
    due_at: Option<i64>,
    stability: f64,
    difficulty: f64,
    reps: i64,
    lapses: i64,
    deck_title: String,
}

// ---------------------------------------------------------------------------
// Deck study
// ---------------------------------------------------------------------------

/// `GET /decks/:id/study` — Get cards to study for a deck.
///
/// The caller must have access: owner, admin, collaborator, or be in
/// a class that has the deck. Cards are ordered: new first, then overdue.
pub async fn deck_study(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(deck_id): Path<i64>,
) -> Result<Json<StudySession>, (StatusCode, &'static str)> {
    // Verify access via the same visibility check used for viewing decks.
    decks::check_deck_visible(&state.db, deck_id, claims.school_id, &claims).await?;

    let deck = sqlx::query!(
        "SELECT id, title FROM decks WHERE id = ? AND school_id = ?",
        deck_id,
        claims.school_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "Deck not found"))?;

    // Create state rows for cards the student hasn't seen yet.
    ensure_card_states_for_deck(&state.db, claims.sub, deck_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let rows = sqlx::query_as!(
        CardRow,
        r#"
        SELECT c.id, c.template_index, n.note_type_id, n.fields_json,
               scs.state, scs.due_at, scs.stability as "stability: f64",
               scs.difficulty as "difficulty: f64", scs.reps, scs.lapses,
               d.title as deck_title
        FROM cards c
        JOIN notes n ON n.id = c.note_id
        JOIN decks d ON d.id = n.deck_id
        JOIN student_card_states scs
            ON scs.card_id = c.id AND scs.student_id = ?
        WHERE n.deck_id = ?
        ORDER BY
            CASE WHEN scs.reps = 0 THEN 0 ELSE 1 END,
            CASE WHEN scs.due_at IS NULL THEN 0 ELSE scs.due_at END
        "#,
        claims.sub,
        deck_id
    )
    .fetch_all(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let total_cards = rows.len() as i64;
    let reviewed_count = rows.iter().filter(|r| r.reps > 0).count() as i64;
    let cards = rows_to_study_cards(&state.db, rows)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(Json(StudySession {
        deck_id: Some(deck.id),
        deck_title: Some(deck.title),
        cards,
        total_cards,
        reviewed_count,
    }))
}
