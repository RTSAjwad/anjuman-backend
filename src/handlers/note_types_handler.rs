// Note type management.
//
// Teachers and admins can create, view, update, and delete note types.
// Each note type defines field names and card templates.
// Note types are scoped to a school.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use crate::{
    auth::AuthUser,
    handlers::users::UserRole,
    note_types::{self, Template},
    state::AppState,
};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateNoteType {
    pub name: String,
    pub field_names: Vec<String>,
    pub templates: Vec<CreateTemplate>,
}

#[derive(Deserialize)]
pub struct CreateTemplate {
    pub name: String,
    pub front_pattern: String,
    pub back_pattern: String,
}

#[derive(Deserialize)]
pub struct UpdateNoteType {
    pub name: Option<String>,
    pub field_names: Option<Vec<String>>,
    pub templates: Option<Vec<CreateTemplate>>,
}

#[derive(Serialize)]
pub struct NoteTypeResponse {
    pub id: i64,
    pub name: String,
    pub field_names: Vec<String>,
    pub templates: Vec<Template>,
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
            "Only teachers and admins can manage note types",
        )),
    }
}

/// Sync templates for a note type: deletes old ones, inserts new ones.
async fn sync_templates(
    db: &sqlx::SqlitePool,
    note_type_id: i64,
    templates: &[CreateTemplate],
) -> Result<(), StatusCode> {
    // Delete existing templates.
    sqlx::query!(
        "DELETE FROM note_type_templates WHERE note_type_id = ?",
        note_type_id
    )
    .execute(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Insert new ones.
    for (i, t) in templates.iter().enumerate() {
        let idx = i as i64;
        sqlx::query!(
            "INSERT INTO note_type_templates (note_type_id, template_index, name, front_pattern, back_pattern) VALUES (?, ?, ?, ?, ?)",
            note_type_id,
            idx,
            t.name,
            t.front_pattern,
            t.back_pattern
        )
        .execute(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /note-types` — List all note types in the school.
pub async fn list_note_types(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<NoteTypeResponse>>, (StatusCode, &'static str)> {
    let types = note_types::list_note_types(&state.db, claims.school_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let response: Vec<NoteTypeResponse> = types
        .into_iter()
        .map(|nt| NoteTypeResponse {
            id: nt.id,
            name: nt.name,
            field_names: nt.field_names,
            templates: nt.templates,
        })
        .collect();

    Ok(Json(response))
}

/// `GET /note-types/{id}` — Get a single note type.
pub async fn get_note_type(
    AuthUser(_claims): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<NoteTypeResponse>, (StatusCode, &'static str)> {
    let nt = note_types::get_note_type(&state.db, id)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Note type not found"))?;

    Ok(Json(NoteTypeResponse {
        id: nt.id,
        name: nt.name,
        field_names: nt.field_names,
        templates: nt.templates,
    }))
}

/// `POST /note-types` — Create a new note type.
pub async fn create_note_type(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateNoteType>,
) -> Result<(StatusCode, Json<NoteTypeResponse>), (StatusCode, String)> {
    check_teacher_or_admin(&claims).map_err(|(s, m)| (s, m.to_string()))?;

    if body.name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Name is required".to_string()));
    }
    if body.field_names.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "At least one field is required".to_string(),
        ));
    }
    if body.templates.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "At least one template is required".to_string(),
        ));
    }

    let field_names_json = serde_json::to_string(&body.field_names)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid field names".to_string()))?;

    let result = sqlx::query!(
        "INSERT INTO note_types (school_id, name, field_names, created_by, created_at) VALUES (?, ?, ?, ?, unixepoch())",
        claims.school_id,
        body.name,
        field_names_json,
        claims.sub
    )
    .execute(&state.db)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            (
                StatusCode::CONFLICT,
                "A note type with that name already exists in your school".to_string(),
            )
        } else {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        }
    })?;

    let note_type_id = result.last_insert_rowid();

    sync_templates(&state.db, note_type_id, &body.templates)
        .await
        .map_err(|s| (s, "Failed to create templates".to_string()))?;

    let nt = note_types::get_note_type(&state.db, note_type_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to fetch created note type".to_string(),
            )
        })?;

    Ok((
        StatusCode::CREATED,
        Json(NoteTypeResponse {
            id: nt.id,
            name: nt.name,
            field_names: nt.field_names,
            templates: nt.templates,
        }),
    ))
}

/// `PATCH /note-types/{id}` — Update a note type's name, fields, or templates.
///
/// Changing templates does not update existing cards — cards are rendered
/// at display time, so any notes using this note type will automatically
/// reflect the new templates on the next fetch.
pub async fn update_note_type(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateNoteType>,
) -> Result<Json<NoteTypeResponse>, (StatusCode, String)> {
    check_teacher_or_admin(&claims).map_err(|(s, m)| (s, m.to_string()))?;

    // Verify the note type exists and belongs to this school.
    let _existing = sqlx::query!(
        "SELECT id FROM note_types WHERE id = ? AND school_id = ?",
        id,
        claims.school_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database error".to_string(),
        )
    })?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "Note type not found".to_string()))?;

    if let Some(name) = &body.name {
        if name.is_empty() {
            return Err((StatusCode::BAD_REQUEST, "Name cannot be empty".to_string()));
        }
        sqlx::query!("UPDATE note_types SET name = ? WHERE id = ?", name, id)
            .execute(&state.db)
            .await
            .map_err(|e| {
                if e.to_string().contains("UNIQUE") {
                    (
                        StatusCode::CONFLICT,
                        "A note type with that name already exists".to_string(),
                    )
                } else {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Database error".to_string(),
                    )
                }
            })?;
    }

    if let Some(field_names) = &body.field_names {
        if field_names.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "At least one field is required".to_string(),
            ));
        }
        let json = serde_json::to_string(field_names)
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid field names".to_string()))?;
        sqlx::query!(
            "UPDATE note_types SET field_names = ? WHERE id = ?",
            json,
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

    if let Some(templates) = &body.templates {
        if templates.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "At least one template is required".to_string(),
            ));
        }
        sync_templates(&state.db, id, templates)
            .await
            .map_err(|s| (s, "Failed to update templates".to_string()))?;
    }

    let nt = note_types::get_note_type(&state.db, id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to fetch updated note type".to_string(),
            )
        })?;

    Ok(Json(NoteTypeResponse {
        id: nt.id,
        name: nt.name,
        field_names: nt.field_names,
        templates: nt.templates,
    }))
}

/// `DELETE /note-types/{id}` — Delete a note type.
///
/// Fails if any notes are using this note type.
pub async fn delete_note_type(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<MessageResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;

    let count: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) as \"count!: i64\" FROM notes WHERE note_type_id = ?",
        id
    )
    .fetch_one(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if count > 0 {
        return Err((
            StatusCode::CONFLICT,
            "Cannot delete a note type that is in use by notes",
        ));
    }

    let result = sqlx::query!(
        "DELETE FROM note_types WHERE id = ? AND school_id = ?",
        id,
        claims.school_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Note type not found"));
    }

    Ok(Json(MessageResponse {
        message: "Note type deleted",
    }))
}
