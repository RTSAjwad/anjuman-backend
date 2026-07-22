// Note management within a deck.
//
// The deck owner (and admins) can create, read, update, and delete notes.
// Cards are rendered at display time from note type templates + note fields.
//
// When a note is created or its note type changes, card rows are created
// or removed to match the note type's template count. The cards table
// only stores the note-to-template link — front/back text is generated
// on every fetch.

use axum::{
    Json,
    extract::{Path, State},
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
}

#[derive(Deserialize)]
pub struct UpdateNote {
    pub note_type_id: Option<i64>,
    pub fields: Option<serde_json::Map<String, Value>>,
}

#[derive(Serialize)]
pub struct NoteResponse {
    pub id: i64,
    pub deck_ids: Vec<i64>,
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
    pub front: String,
    pub back: String,
}

#[derive(Serialize)]
pub struct MessageResponse {
    pub message: &'static str,
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

/// Synchronise card rows for a note in a deck: ensures one card row exists
/// per template in the note type. Old extra rows are deleted.
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

/// Fetch a note and render its cards at display time.
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
        "SELECT id, deck_id, template_index FROM cards WHERE note_id = ? ORDER BY template_index",
        note_id
    )
    .fetch_all(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let deck_ids: Vec<i64> = card_rows.iter().map(|c| c.deck_id).collect();

    let card_summaries: Vec<CardSummary> = card_rows
        .into_iter()
        .filter_map(|c| {
            note_types::render_card(&nt.templates, c.template_index, &fields).map(|rendered| {
                CardSummary {
                    id: c.id.expect("card.id is NOT NULL"),
                    template_index: c.template_index,
                    front: rendered.front,
                    back: rendered.back,
                }
            })
        })
        .collect();

    Ok(NoteResponse {
        id: note.id,
        deck_ids,
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

pub async fn create_note(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(deck_id): Path<i64>,
    Json(body): Json<CreateNote>,
) -> Result<(StatusCode, Json<NoteResponse>), (StatusCode, String)> {
    check_teacher_or_admin(&claims).map_err(|(s, m)| (s, m.to_string()))?;
    decks::check_deck_collaborator(&state.db, deck_id, claims.school_id, &claims)
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

    sync_card_rows(&state.db, note_id, deck_id, &nt)
        .await
        .map_err(|s| (s, "Failed to create cards".to_string()))?;

    let note = fetch_note_with_cards(&state.db, note_id)
        .await
        .map_err(|s| (s, "Failed to fetch created note".to_string()))?;

    Ok((StatusCode::CREATED, Json(note)))
}

pub async fn get_note(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path((deck_id, note_id)): Path<(i64, i64)>,
) -> Result<Json<NoteResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    decks::check_deck_visible(&state.db, deck_id, claims.school_id, &claims).await?;

    let note = fetch_note_with_cards(&state.db, note_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if !note.deck_ids.contains(&deck_id) {
        return Err((StatusCode::NOT_FOUND, "Note not found in this deck"));
    }

    Ok(Json(note))
}

pub async fn list_notes(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(deck_id): Path<i64>,
) -> Result<Json<Vec<NoteResponse>>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    decks::check_deck_visible(&state.db, deck_id, claims.school_id, &claims).await?;

    let rows = sqlx::query!(
        "SELECT DISTINCT n.id, n.created_at FROM notes n JOIN cards c ON c.note_id = n.id WHERE c.deck_id = ? ORDER BY n.created_at",
        deck_id
    )
    .fetch_all(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let mut result = Vec::new();
    for n in rows {
        let note = fetch_note_with_cards(&state.db, n.id.expect("note.id is NOT NULL"))
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;
        result.push(note);
    }

    Ok(Json(result))
}

pub async fn update_note(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path((deck_id, note_id)): Path<(i64, i64)>,
    Json(body): Json<UpdateNote>,
) -> Result<Json<NoteResponse>, (StatusCode, String)> {
    check_teacher_or_admin(&claims).map_err(|(s, m)| (s, m.to_string()))?;
    decks::check_deck_collaborator(&state.db, deck_id, claims.school_id, &claims)
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

    // If the note type changed, re-sync card rows in all decks the note is in.
    if new_note_type_id != existing.note_type_id {
        let decks = sqlx::query!(
            "SELECT DISTINCT deck_id FROM cards WHERE note_id = ?",
            note_id
        )
        .fetch_all(&state.db)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;
        for d in decks {
            sync_card_rows(&state.db, note_id, d.deck_id, &nt)
                .await
                .map_err(|s| (s, "Failed to update cards".to_string()))?;
        }
    }

    let note = fetch_note_with_cards(&state.db, note_id)
        .await
        .map_err(|s| (s, "Failed to fetch updated note".to_string()))?;

    Ok(Json(note))
}

pub async fn delete_note(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path((deck_id, note_id)): Path<(i64, i64)>,
) -> Result<Json<MessageResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    decks::check_deck_collaborator(&state.db, deck_id, claims.school_id, &claims).await?;

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
