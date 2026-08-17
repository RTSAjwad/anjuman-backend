// Note management (note-centric, matching Anki's model).
//
// Notes are deck-independent content: a note belongs to a note type and holds
// field values. Cards are generated from a note's templates and each card is
// assigned to a deck. The same note can therefore have its cards spread across
// multiple decks.
//
//   POST   /notes          — create a note (its cards go to a chosen deck)
//   GET    /notes          — list notes (optional ?deck_id= filter)
//   GET    /notes/{id}     — fetch a note with its cards + decks
//   PATCH  /notes/{id}     — update fields / note type
//   DELETE /notes/{id}     — delete a note and all its cards
//
// Moving an individual card to another deck is a separate endpoint:
//   PATCH  /cards/{id}/deck

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    auth::AuthUser,
    handlers::{decks, users::UserRole},
    note_types,
    state::AppState,
};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateNote {
    pub note_type_id: i64,
    pub fields: serde_json::Map<String, Value>,
    /// The deck the note's generated cards go into (the "selected deck").
    pub deck_id: i64,
}

#[derive(Deserialize)]
pub struct UpdateNote {
    pub note_type_id: Option<i64>,
    pub fields: Option<serde_json::Map<String, Value>>,
}

#[derive(Deserialize)]
pub struct ListNotesQuery {
    /// Optional deck filter: only notes with at least one card in this deck.
    pub deck_id: Option<i64>,
}

#[derive(Serialize)]
pub struct NoteResponse {
    pub id: i64,
    pub note_type_id: i64,
    pub note_type_name: String,
    pub fields: serde_json::Map<String, Value>,
    pub cards: Vec<CardSummary>,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct CardSummary {
    pub id: i64,
    pub template_index: i64,
    pub template_name: String,
    pub deck_id: i64,
    pub front: String,
    pub back: String,
}

#[derive(Serialize)]
pub struct MessageResponse {
    pub message: &'static str,
}

#[derive(sqlx::FromRow)]
struct NoteIdRow {
    id: i64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn check_teacher_or_admin(claims: &crate::auth::Claims) -> Result<(), (StatusCode, &'static str)> {
    match claims.role {
        UserRole::Admin | UserRole::Teacher => Ok(()),
        UserRole::Student => Err((
            StatusCode::FORBIDDEN,
            "Only teachers and admins can manage notes",
        )),
    }
}

/// Ensure one card row exists per template for a note in a specific deck.
/// Old extra rows (template indices >= template count) are deleted.
async fn sync_card_rows(
    db: &sqlx::SqlitePool,
    note_id: i64,
    deck_id: i64,
    nt: &note_types::NoteType,
) -> Result<(), StatusCode> {
    let template_count = nt.templates.len() as i64;

    for i in 0..template_count {
        sqlx::query!(
            "INSERT OR IGNORE INTO cards (note_id, deck_id, template_index, created_at) VALUES (?, ?, ?, unixepoch())",
            note_id,
            deck_id,
            i
        )
        .execute(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    sqlx::query!(
        "DELETE FROM cards WHERE note_id = ? AND deck_id = ? AND template_index >= ?",
        note_id,
        deck_id,
        template_count
    )
    .execute(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(())
}

/// Fetch the deck IDs a note's cards are in.
async fn note_deck_ids(db: &sqlx::SqlitePool, note_id: i64) -> Result<Vec<i64>, StatusCode> {
    let rows = sqlx::query!(
        "SELECT DISTINCT deck_id FROM cards WHERE note_id = ?",
        note_id
    )
    .fetch_all(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(rows.into_iter().map(|r| r.deck_id).collect())
}

/// Require the caller to have collaborator access to every deck the note spans.
async fn check_note_authorization(
    db: &sqlx::SqlitePool,
    note_id: i64,
    school_id: i64,
    claims: &crate::auth::Claims,
) -> Result<(), (StatusCode, &'static str)> {
    let deck_ids = note_deck_ids(db, note_id)
        .await
        .map_err(|s| (s, "Database error"))?;
    if deck_ids.is_empty() {
        return Ok(());
    }
    for deck_id in deck_ids {
        decks::check_deck_collaborator(db, deck_id, school_id, claims).await?;
    }
    Ok(())
}

/// Fetch a note and render its cards at display time. Each card includes its
/// deck_id so callers know where the note's cards live.
async fn fetch_note_with_cards(
    db: &sqlx::SqlitePool,
    note_id: i64,
) -> Result<NoteResponse, StatusCode> {
    let note = sqlx::query!(
        "SELECT id, note_type_id, fields_json, created_at FROM notes WHERE id = ?",
        note_id
    )
    .fetch_optional(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)?;

    let fields: serde_json::Map<String, Value> =
        serde_json::from_str(&note.fields_json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let nt = note_types::get_note_type(db, note.note_type_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let card_rows = sqlx::query!(
        "SELECT id, deck_id, template_index FROM cards WHERE note_id = ? ORDER BY deck_id, template_index",
        note_id
    )
    .fetch_all(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let card_summaries: Vec<CardSummary> = card_rows
        .into_iter()
        .filter_map(|c| {
            note_types::render_card(&nt.templates, c.template_index, &fields).map(|rendered| {
                let template_name = nt
                    .templates
                    .iter()
                    .find(|t| t.index == c.template_index)
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| format!("Card {}", c.template_index + 1));
                CardSummary {
                    id: c.id.expect("card.id is NOT NULL"),
                    template_index: c.template_index,
                    template_name,
                    deck_id: c.deck_id,
                    front: rendered.front,
                    back: rendered.back,
                }
            })
        })
        .collect();

    Ok(NoteResponse {
        id: note.id,
        note_type_id: note.note_type_id,
        note_type_name: nt.name,
        fields,
        cards: card_summaries,
        created_at: note.created_at,
    })
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /notes` — Create a note and place its generated cards in a deck.
pub async fn create_note(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateNote>,
) -> Result<(StatusCode, Json<NoteResponse>), (StatusCode, String)> {
    check_teacher_or_admin(&claims).map_err(|(s, m)| (s, m.to_string()))?;
    decks::check_deck_collaborator(&state.db, body.deck_id, claims.school_id, &claims)
        .await
        .map_err(|(s, m)| (s, m.to_string()))?;

    let nt = note_types::get_note_type(&state.db, body.note_type_id)
        .await
        .map_err(|msg| (StatusCode::BAD_REQUEST, msg))?;

    note_types::validate_fields(&nt.field_names, &body.fields)
        .map_err(|msg| (StatusCode::BAD_REQUEST, msg))?;

    let fields_json = serde_json::to_string(&body.fields)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid fields JSON".to_string()))?;

    let result = sqlx::query!(
        "INSERT INTO notes (note_type_id, fields_json, created_at) VALUES (?, ?, unixepoch())",
        body.note_type_id,
        fields_json
    )
    .execute(&state.db)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database error".to_string(),
        )
    })?;

    let note_id = result.last_insert_rowid();

    sync_card_rows(&state.db, note_id, body.deck_id, &nt)
        .await
        .map_err(|s| (s, "Failed to create cards".to_string()))?;

    let note = fetch_note_with_cards(&state.db, note_id)
        .await
        .map_err(|s| (s, "Failed to fetch created note".to_string()))?;

    Ok((StatusCode::CREATED, Json(note)))
}

/// `GET /notes` — List notes (optionally filtered by deck).
pub async fn list_notes(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Query(params): Query<ListNotesQuery>,
) -> Result<Json<Vec<NoteResponse>>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;

    // Restrict to notes the caller can see. For admins: everything in school.
    // For teachers: notes with at least one card in a deck they own/collaborate on.
    if let Some(deck_id) = params.deck_id {
        decks::check_deck_visible(&state.db, deck_id, claims.school_id, &claims).await?;
    }

    let rows: Vec<NoteIdRow> = if let Some(deck_id) = params.deck_id {
        sqlx::query_as::<_, NoteIdRow>(
            "SELECT DISTINCT n.id FROM notes n JOIN cards c ON c.note_id = n.id WHERE c.deck_id = ? ORDER BY n.id",
        )
        .bind(deck_id)
        .fetch_all(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    } else if claims.role == UserRole::Admin {
        sqlx::query_as::<_, NoteIdRow>(
            "SELECT DISTINCT n.id FROM notes n JOIN cards c ON c.note_id = n.id JOIN decks d ON d.id = c.deck_id WHERE d.school_id = ? ORDER BY n.id",
        )
        .bind(claims.school_id)
        .fetch_all(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    } else {
        sqlx::query_as::<_, NoteIdRow>(
            r#"
            SELECT DISTINCT n.id
            FROM notes n
            JOIN cards c ON c.note_id = n.id
            JOIN decks d ON d.id = c.deck_id
            LEFT JOIN deck_collaborators dc ON dc.deck_id = d.id
            WHERE d.school_id = ?
              AND (d.created_by = ? OR dc.user_id = ?)
            ORDER BY n.id
            "#,
        )
        .bind(claims.school_id)
        .bind(claims.sub)
        .bind(claims.sub)
        .fetch_all(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    };

    let mut result = Vec::new();
    for n in rows {
        let note = fetch_note_with_cards(&state.db, n.id)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;
        result.push(note);
    }

    Ok(Json(result))
}

/// `GET /notes/{id}` — Fetch a single note.
pub async fn get_note(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(note_id): Path<i64>,
) -> Result<Json<NoteResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;

    // Reuse note-level authorization (requires access to all decks it spans).
    check_note_authorization(&state.db, note_id, claims.school_id, &claims).await?;

    let note = fetch_note_with_cards(&state.db, note_id)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Note not found"))?;

    Ok(Json(note))
}

/// `PATCH /notes/{id}` — Update a note's fields or note type.
pub async fn update_note(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(note_id): Path<i64>,
    Json(body): Json<UpdateNote>,
) -> Result<Json<NoteResponse>, (StatusCode, String)> {
    check_teacher_or_admin(&claims).map_err(|(s, m)| (s, m.to_string()))?;
    check_note_authorization(&state.db, note_id, claims.school_id, &claims)
        .await
        .map_err(|(s, m)| (s, m.to_string()))?;

    let existing = sqlx::query!(
        "SELECT note_type_id, fields_json FROM notes WHERE id = ?",
        note_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database error".to_string(),
        )
    })?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "Note not found".to_string()))?;

    let new_note_type_id = body.note_type_id.unwrap_or(existing.note_type_id);
    let new_fields = match body.fields {
        Some(f) => f,
        None => serde_json::from_str(&existing.fields_json).map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Invalid stored JSON".to_string(),
            )
        })?,
    };

    let nt = note_types::get_note_type(&state.db, new_note_type_id)
        .await
        .map_err(|msg| (StatusCode::BAD_REQUEST, msg))?;

    note_types::validate_fields(&nt.field_names, &new_fields)
        .map_err(|msg| (StatusCode::BAD_REQUEST, msg))?;

    let fields_json = serde_json::to_string(&new_fields)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid fields JSON".to_string()))?;

    sqlx::query!(
        "UPDATE notes SET note_type_id = ?, fields_json = ? WHERE id = ?",
        new_note_type_id,
        fields_json,
        note_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database error".to_string(),
        )
    })?;

    // If the note type changed, re-sync card rows in every deck the note spans.
    if new_note_type_id != existing.note_type_id {
        let deck_ids = note_deck_ids(&state.db, note_id)
            .await
            .map_err(|s| (s, "Failed to read note decks".to_string()))?;
        for deck_id in deck_ids {
            sync_card_rows(&state.db, note_id, deck_id, &nt)
                .await
                .map_err(|s| (s, "Failed to update cards".to_string()))?;
        }
    }

    let note = fetch_note_with_cards(&state.db, note_id)
        .await
        .map_err(|s| (s, "Failed to fetch updated note".to_string()))?;

    Ok(Json(note))
}

/// `DELETE /notes/{id}` — Delete a note and all its cards.
pub async fn delete_note(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(note_id): Path<i64>,
) -> Result<Json<MessageResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    check_note_authorization(&state.db, note_id, claims.school_id, &claims).await?;

    let result = sqlx::query!("DELETE FROM notes WHERE id = ?", note_id)
        .execute(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Note not found"));
    }

    Ok(Json(MessageResponse {
        message: "Note deleted",
    }))
}
