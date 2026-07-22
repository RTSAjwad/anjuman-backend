// Deck management.
//
// Teachers (and admins) can create, rename, delete, duplicate, share,
// and publish decks. All operations are scoped to the caller's school.
//
// ## Access model
//
// A user can access a deck if any of these are true:
//   1. They created it (owner).
//   2. They are an admin in the same school.
//   3. The deck is published to the school.
//   4. They are listed as a collaborator on the deck.
//
// ## Role permissions
//
// | Action              | Admin | Teacher | Student |
// |---------------------|-------|---------|---------|
// | Create deck         | ✅    | ✅      | ❌      |
// | Rename deck         | ✅    | ✅ (owner) | ❌  |
// | Delete deck         | ✅    | ✅ (owner) | ❌  |
// | Duplicate deck      | ✅    | ✅      | ❌      |
// | Share deck          | ✅    | ✅ (owner) | ❌  |
// | Publish deck        | ✅    | ✅ (owner) | ❌  |
// | View deck           | ✅    | ✅      | ✅      |

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use crate::{auth::AuthUser, handlers::users::UserRole, state::AppState};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateDeck {
    pub title: String,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct RenameDeck {
    pub title: String,
}

#[derive(Deserialize)]
pub struct ShareDeck {
    /// The teacher to share the deck with.
    pub user_id: i64,
}

#[derive(Deserialize)]
pub struct TransferOwner {
    /// The teacher who will become the new owner.
    pub user_id: i64,
}

#[derive(Deserialize)]
pub struct AddDeckToClass {
    pub class_id: i64,
}

#[derive(Serialize)]
pub struct DeckResponse {
    pub id: i64,
    pub school_id: i64,
    pub title: String,
    pub description: Option<String>,
    pub created_by: i64,
    pub owner_email: String,
    pub owner_first_name: String,
    pub owner_last_name: String,
    pub created_at: String,
    /// Card counts for the requesting student. Present only in list_decks.
    /// Not populated in get_deck, create_deck, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learning_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relearning_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_count: Option<i64>,
}

#[derive(Serialize)]
pub struct CollaboratorResponse {
    pub user_id: i64,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub shared_at: i64,
}

#[derive(Serialize)]
pub struct DeckDetailResponse {
    pub deck: DeckResponse,
    pub collaborators: Vec<CollaboratorResponse>,
    pub classes: Vec<ClassInfo>,
}

#[derive(Serialize)]
pub struct ClassInfo {
    pub id: i64,
    pub name: String,
}

#[derive(Serialize)]
pub struct MessageResponse {
    pub message: &'static str,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Require that the authenticated user is a teacher or admin.
pub fn check_teacher_or_admin(
    claims: &crate::auth::Claims,
) -> Result<(), (StatusCode, &'static str)> {
    match claims.role {
        UserRole::Admin | UserRole::Teacher => Ok(()),
        UserRole::Student => Err((
            StatusCode::FORBIDDEN,
            "Only teachers and admins can manage decks",
        )),
    }
}

/// Check whether the caller can manage (edit/delete/share) a deck.
/// Admins can manage any deck in their school. Teachers must be the owner.
pub async fn check_deck_owner(
    db: &sqlx::SqlitePool,
    deck_id: i64,
    school_id: i64,
    claims: &crate::auth::Claims,
) -> Result<(), (StatusCode, &'static str)> {
    let row = sqlx::query!(
        "SELECT id, created_by FROM decks WHERE id = ? AND school_id = ?",
        deck_id,
        school_id
    )
    .fetch_optional(db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "Deck not found"))?;

    if claims.role == UserRole::Admin {
        return Ok(());
    }

    if row.created_by != claims.sub {
        return Err((
            StatusCode::FORBIDDEN,
            "You can only manage decks you created",
        ));
    }

    Ok(())
}

/// Check whether the caller can manage a deck's content (notes).
/// Admins can manage any deck in their school. Teachers must be
/// the owner OR a collaborator on the deck.
pub async fn check_deck_collaborator(
    db: &sqlx::SqlitePool,
    deck_id: i64,
    school_id: i64,
    claims: &crate::auth::Claims,
) -> Result<(), (StatusCode, &'static str)> {
    let row = sqlx::query!(
        "SELECT id, created_by FROM decks WHERE id = ? AND school_id = ?",
        deck_id,
        school_id
    )
    .fetch_optional(db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "Deck not found"))?;

    if claims.role == UserRole::Admin {
        return Ok(());
    }

    // Owner can manage.
    if row.created_by == claims.sub {
        return Ok(());
    }

    // Check if the user is a collaborator.
    let is_collab = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM deck_collaborators WHERE deck_id = ? AND user_id = ?) AS "exists!: i64""#,
        deck_id,
        claims.sub
    )
    .fetch_one(db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if is_collab != 0 {
        return Ok(());
    }

    Err((
        StatusCode::FORBIDDEN,
        "You can only manage decks you own or collaborate on",
    ))
}

/// Check whether the caller can view a deck. The caller can view if they
/// are the owner, an admin, a collaborator, or in a class that has the deck.
pub async fn check_deck_visible(
    db: &sqlx::SqlitePool,
    deck_id: i64,
    school_id: i64,
    claims: &crate::auth::Claims,
) -> Result<(), (StatusCode, &'static str)> {
    let row = sqlx::query!(
        r#"
        SELECT id, created_by
        FROM decks
        WHERE id = ? AND school_id = ?
        "#,
        deck_id,
        school_id
    )
    .fetch_optional(db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "Deck not found"))?;

    // Owner or admin always has access.
    if claims.role == UserRole::Admin || row.created_by == claims.sub {
        return Ok(());
    }

    // Check if the user is a collaborator.
    let is_collab = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM deck_collaborators WHERE deck_id = ? AND user_id = ?) AS "exists!: i64""#,
        deck_id,
        claims.sub
    )
    .fetch_one(db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if is_collab != 0 {
        return Ok(());
    }

    // Check if the user is in a class that has this deck assigned.
    let in_class = sqlx::query_scalar!(
        r#"SELECT EXISTS(
            SELECT 1 FROM deck_classes dcl
            JOIN class_members cm ON cm.class_id = dcl.class_id
            WHERE dcl.deck_id = ? AND cm.user_id = ?
        ) AS "exists!: i64""#,
        deck_id,
        claims.sub
    )
    .fetch_one(db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if in_class != 0 {
        return Ok(());
    }

    Err((StatusCode::FORBIDDEN, "You do not have access to this deck"))
}

/// Fetch a deck row and convert to response DTO.
async fn fetch_deck(
    db: &sqlx::SqlitePool,
    deck_id: i64,
) -> Result<DeckResponse, (StatusCode, &'static str)> {
    let row = sqlx::query!(
        r#"
        SELECT d.id, d.school_id, d.title, d.description,
               d.created_by, d.created_at,
               u.email as owner_email,
               u.first_name as owner_first_name,
               u.last_name as owner_last_name
        FROM decks d
        JOIN users u ON u.id = d.created_by
        WHERE d.id = ?
        "#,
        deck_id
    )
    .fetch_one(db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(DeckResponse {
        id: row.id,
        school_id: row.school_id,
        title: row.title,
        description: row.description,
        created_by: row.created_by,
        owner_email: row.owner_email,
        owner_first_name: row.owner_first_name,
        owner_last_name: row.owner_last_name,
        created_at: row.created_at,
        new_count: None,
        learning_count: None,
        due_count: None,
        relearning_count: None,
        total_count: None,
    })
}

// ---------------------------------------------------------------------------
// Deck CRUD
// ---------------------------------------------------------------------------

/// `POST /decks` — Create a new deck.
pub async fn create_deck(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateDeck>,
) -> Result<(StatusCode, Json<DeckResponse>), (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;

    let result = sqlx::query!(
        r#"
        INSERT INTO decks (school_id, title, description, created_by, created_at)
        VALUES (?, ?, ?, ?, unixepoch())
        "#,
        claims.school_id,
        body.title,
        body.description,
        claims.sub
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let deck = fetch_deck(&state.db, result.last_insert_rowid()).await?;
    Ok((StatusCode::CREATED, Json(deck)))
}

/// `PATCH /decks/:id/rename` — Rename a deck.
pub async fn rename_deck(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(deck_id): Path<i64>,
    Json(body): Json<RenameDeck>,
) -> Result<Json<DeckResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    check_deck_collaborator(&state.db, deck_id, claims.school_id, &claims).await?;

    sqlx::query!(
        "UPDATE decks SET title = ? WHERE id = ?",
        body.title,
        deck_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let deck = fetch_deck(&state.db, deck_id).await?;
    Ok(Json(deck))
}

/// `DELETE /decks/:id` — Delete a deck permanently.
pub async fn delete_deck(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(deck_id): Path<i64>,
) -> Result<Json<MessageResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    check_deck_owner(&state.db, deck_id, claims.school_id, &claims).await?;

    sqlx::query!("DELETE FROM decks WHERE id = ?", deck_id)
        .execute(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(Json(MessageResponse {
        message: "Deck deleted",
    }))
}

/// `POST /decks/:id/duplicate` — Create a copy of an existing deck.
///
/// Copies the deck shell, all notes, and all cards. The new deck belongs
/// to the caller and records the source via `original_deck_id`.
pub async fn duplicate_deck(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(deck_id): Path<i64>,
) -> Result<(StatusCode, Json<DeckResponse>), (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;

    // Fetch the source deck — must be visible to the caller.
    let source = sqlx::query!(
        r#"
        SELECT id, title, description, created_by
        FROM decks
        WHERE id = ? AND school_id = ?
        "#,
        deck_id,
        claims.school_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "Deck not found"))?;

    // Visibility check: must be owner, admin, or collaborator.
    let can_access = claims.role == UserRole::Admin || source.created_by == claims.sub;

    if !can_access {
        let is_collab = sqlx::query_scalar!(
            r#"SELECT EXISTS(SELECT 1 FROM deck_collaborators WHERE deck_id = ? AND user_id = ?) AS "exists!: i64""#,
            deck_id,
            claims.sub
        )
        .fetch_one(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        if is_collab == 0 {
            return Err((StatusCode::FORBIDDEN, "You do not have access to this deck"));
        }
    }

    let new_title = format!("{} (copy)", source.title);

    // Wrap the entire duplication in a transaction — if anything fails,
    // the partial copy is rolled back.
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let result = sqlx::query!(
        r#"
        INSERT INTO decks (school_id, title, description, created_by, created_at)
        VALUES (?, ?, ?, ?, unixepoch())
        "#,
        claims.school_id,
        new_title,
        source.description,
        claims.sub
    )
    .execute(&mut *tx)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let new_deck_id = result.last_insert_rowid();

    // Copy all notes and their cards into the new deck.
    let notes = sqlx::query!(
        "SELECT id, note_type_id, fields_json FROM notes WHERE deck_id = ? ORDER BY id",
        deck_id
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    for note in &notes {
        let note_result = sqlx::query!(
            "INSERT INTO notes (deck_id, note_type_id, fields_json, created_at) VALUES (?, ?, ?, unixepoch())",
            new_deck_id,
            note.note_type_id,
            note.fields_json
        )
        .execute(&mut *tx)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        let new_note_id = note_result.last_insert_rowid();

        let cards = sqlx::query!(
            "SELECT template_index FROM cards WHERE note_id = ?",
            note.id
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        for card in &cards {
            sqlx::query!(
                "INSERT INTO cards (note_id, template_index, created_at) VALUES (?, ?, unixepoch())",
                new_note_id,
                card.template_index
            )
            .execute(&mut *tx)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;
        }
    }

    tx.commit()
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let deck = fetch_deck(&state.db, new_deck_id).await?;
    Ok((StatusCode::CREATED, Json(deck)))
}

// ---------------------------------------------------------------------------
// Sharing & publishing
// ---------------------------------------------------------------------------

/// `POST /decks/:id/share` — Share a deck with another teacher.
///
/// The target user must be a teacher or admin in the same school.
/// If the deck is already shared with them, the request is idempotent.
pub async fn share_deck(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(deck_id): Path<i64>,
    Json(body): Json<ShareDeck>,
) -> Result<Json<CollaboratorResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    check_deck_owner(&state.db, deck_id, claims.school_id, &claims).await?;

    // Verify the target user exists in the same school and is a teacher/admin.
    let target = sqlx::query!(
        "SELECT id, email, first_name, last_name, role FROM users WHERE id = ? AND school_id = ?",
        body.user_id,
        claims.school_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "User not found in your school"))?;

    if target.role == "student" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot share a deck with a student",
        ));
    }

    if target.role == "admin" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Admins already have access to all decks",
        ));
    }

    // Prevent sharing with the deck owner (they already have full access).
    let deck = sqlx::query!("SELECT created_by FROM decks WHERE id = ?", deck_id)
        .fetch_one(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if body.user_id == deck.created_by {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot share a deck with its owner",
        ));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // INSERT OR IGNORE makes this idempotent.
    sqlx::query!(
        "INSERT OR IGNORE INTO deck_collaborators (deck_id, user_id, shared_at) VALUES (?, ?, ?)",
        deck_id,
        body.user_id,
        now
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(Json(CollaboratorResponse {
        user_id: body.user_id,
        email: target.email,
        first_name: target.first_name,
        last_name: target.last_name,
        shared_at: now,
    }))
}

/// `DELETE /decks/:id/share/:user_id` — Remove a collaborator from a deck.
pub async fn unshare_deck(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path((deck_id, user_id)): Path<(i64, i64)>,
) -> Result<Json<MessageResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    check_deck_owner(&state.db, deck_id, claims.school_id, &claims).await?;

    let result = sqlx::query!(
        "DELETE FROM deck_collaborators WHERE deck_id = ? AND user_id = ?",
        deck_id,
        user_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Collaborator not found"));
    }

    Ok(Json(MessageResponse {
        message: "Collaborator removed",
    }))
}

/// `PATCH /decks/:id/owner` — Transfer deck ownership to another teacher.
///
/// The current owner (or an admin) can transfer ownership. The new owner
/// must be a teacher in the same school. The old owner is automatically
/// added as a collaborator so they don't lose access.
pub async fn transfer_owner(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(deck_id): Path<i64>,
    Json(body): Json<TransferOwner>,
) -> Result<Json<DeckResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    check_deck_owner(&state.db, deck_id, claims.school_id, &claims).await?;

    // Fetch current owner for collaborator insertion.
    let current = sqlx::query!("SELECT created_by FROM decks WHERE id = ?", deck_id)
        .fetch_one(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    // Idempotent: transferring to the current owner does nothing.
    if body.user_id == current.created_by {
        let deck = fetch_deck(&state.db, deck_id).await?;
        return Ok(Json(deck));
    }

    // Verify target is a teacher in the same school.
    let target = sqlx::query!(
        "SELECT id, role FROM users WHERE id = ? AND school_id = ?",
        body.user_id,
        claims.school_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "User not found in your school"))?;

    if target.role != "teacher" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Ownership can only be transferred to a teacher",
        ));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // Update the owner.
    sqlx::query!(
        "UPDATE decks SET created_by = ? WHERE id = ?",
        body.user_id,
        deck_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    // Add the old owner as a collaborator.
    sqlx::query!(
        "INSERT OR IGNORE INTO deck_collaborators (deck_id, user_id, shared_at) VALUES (?, ?, ?)",
        deck_id,
        current.created_by,
        now
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let deck = fetch_deck(&state.db, deck_id).await?;
    Ok(Json(deck))
}

/// Add a deck to a class so students in that class can study it.
///
/// Only the deck owner, an admin, or a teacher member of the class can do this.
pub async fn add_deck_to_class(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(deck_id): Path<i64>,
    Json(body): Json<AddDeckToClass>,
) -> Result<Json<MessageResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    check_deck_owner(&state.db, deck_id, claims.school_id, &claims).await?;

    // Verify the class belongs to the same school.
    let _class = sqlx::query!(
        "SELECT id, name FROM classes WHERE id = ? AND school_id = ?",
        body.class_id,
        claims.school_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "Class not found"))?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    sqlx::query!(
        "INSERT OR IGNORE INTO deck_classes (deck_id, class_id, added_at) VALUES (?, ?, ?)",
        deck_id,
        body.class_id,
        now
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(Json(MessageResponse {
        message: "Deck added to class",
    }))
}

/// Remove a deck from a class.
pub async fn remove_deck_from_class(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path((deck_id, class_id)): Path<(i64, i64)>,
) -> Result<Json<MessageResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    check_deck_owner(&state.db, deck_id, claims.school_id, &claims).await?;

    let result = sqlx::query!(
        "DELETE FROM deck_classes WHERE deck_id = ? AND class_id = ?",
        deck_id,
        class_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Deck is not assigned to this class"));
    }

    Ok(Json(MessageResponse {
        message: "Deck removed from class",
    }))
}

/// List classes a deck is assigned to.
pub async fn list_deck_classes(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(deck_id): Path<i64>,
) -> Result<Json<Vec<ClassInfo>>, (StatusCode, &'static str)> {
    check_deck_visible(&state.db, deck_id, claims.school_id, &claims).await?;

    let rows = sqlx::query!(
        r#"
        SELECT c.id, c.name
        FROM deck_classes dc
        JOIN classes c ON c.id = dc.class_id
        WHERE dc.deck_id = ?
        ORDER BY c.name
        "#,
        deck_id
    )
    .fetch_all(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let classes: Vec<ClassInfo> = rows
        .into_iter()
        .map(|r| ClassInfo {
            id: r.id.expect("class.id is NOT NULL"),
            name: r.name,
        })
        .collect();

    Ok(Json(classes))
}

// ---------------------------------------------------------------------------
// Viewing
// ---------------------------------------------------------------------------

/// `GET /decks/:id` — Get a deck with its collaborators list.
///
/// The caller must have access: owner, admin, published deck, or collaborator.
pub async fn get_deck(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(deck_id): Path<i64>,
) -> Result<Json<DeckDetailResponse>, (StatusCode, &'static str)> {
    check_deck_visible(&state.db, deck_id, claims.school_id, &claims).await?;

    let deck = fetch_deck(&state.db, deck_id).await?;

    // Owner, admins, and collaborators can see who else is on the deck.
    let is_collab = if claims.role == UserRole::Admin || deck.created_by == claims.sub {
        true
    } else {
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM deck_collaborators WHERE deck_id = ? AND user_id = ?",
            deck_id,
            claims.sub
        )
        .fetch_one(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;
        count > 0
    };

    let collaborators = if is_collab {
        let rows = sqlx::query!(
            r#"
            SELECT dc.user_id, u.email, u.first_name, u.last_name, dc.shared_at
            FROM deck_collaborators dc
            JOIN users u ON u.id = dc.user_id
            WHERE dc.deck_id = ?
            ORDER BY u.email
            "#,
            deck_id
        )
        .fetch_all(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        rows.into_iter()
            .map(|r| CollaboratorResponse {
                user_id: r.user_id,
                email: r.email,
                first_name: r.first_name,
                last_name: r.last_name,
                shared_at: r.shared_at,
            })
            .collect()
    } else {
        vec![]
    };

    // Fetch classes the deck is assigned to.
    let class_rows = sqlx::query!(
        r#"
        SELECT c.id, c.name
        FROM deck_classes dc
        JOIN classes c ON c.id = dc.class_id
        WHERE dc.deck_id = ?
        ORDER BY c.name
        "#,
        deck_id
    )
    .fetch_all(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let deck_classes: Vec<ClassInfo> = class_rows
        .into_iter()
        .map(|r| ClassInfo {
            id: r.id.expect("class.id is NOT NULL"),
            name: r.name,
        })
        .collect();

    Ok(Json(DeckDetailResponse {
        deck,
        collaborators,
        classes: deck_classes,
    }))
}

/// `GET /decks` — List decks visible to the caller.
///
/// - Teachers see their own decks + shared + published.
/// - Admins see all decks in their school.
/// - Students see published decks + decks assigned to their classes.
pub async fn list_decks(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<DeckResponse>>, (StatusCode, &'static str)> {
    if claims.role == UserRole::Admin {
        let rows = sqlx::query!(
            r#"
            SELECT d.id, d.school_id, d.title, d.description,
                   d.created_by, d.created_at,
                   u.email as owner_email,
                   u.first_name as owner_first_name,
                   u.last_name as owner_last_name,
                   (SELECT COUNT(*) FROM cards c JOIN notes n ON n.id = c.note_id WHERE n.deck_id = d.id) as "card_count!: i64"
            FROM decks d
            JOIN users u ON u.id = d.created_by
            WHERE d.school_id = ?
            ORDER BY d.created_at DESC
            "#,
            claims.school_id
        )
        .fetch_all(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        let decks: Vec<DeckResponse> = rows
            .into_iter()
            .map(|r| DeckResponse {
                id: r.id.expect("deck.id is NOT NULL in schema"),
                school_id: r.school_id,
                title: r.title,
                description: r.description,
                created_by: r.created_by,
                owner_email: r.owner_email,
                owner_first_name: r.owner_first_name,
                owner_last_name: r.owner_last_name,
                created_at: r.created_at,
                new_count: None,
                learning_count: None,
                due_count: None,
                relearning_count: None,
                total_count: Some(r.card_count),
            })
            .collect();
        return Ok(Json(decks));
    }

    if claims.role == UserRole::Teacher {
        let rows = sqlx::query!(
            r#"
            SELECT DISTINCT d.id, d.school_id, d.title, d.description,
                   d.created_by, d.created_at,
                   u.email as owner_email,
                   u.first_name as owner_first_name,
                   u.last_name as owner_last_name,
                   (SELECT COUNT(*) FROM cards c JOIN notes n ON n.id = c.note_id WHERE n.deck_id = d.id) as "card_count!: i64"
            FROM decks d
            JOIN users u ON u.id = d.created_by
            LEFT JOIN deck_collaborators dc ON dc.deck_id = d.id
            WHERE d.school_id = ?
              AND (d.created_by = ? OR dc.user_id = ?)
            ORDER BY d.created_at DESC
            "#,
            claims.school_id,
            claims.sub,
            claims.sub
        )
        .fetch_all(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        let decks: Vec<DeckResponse> = rows
            .into_iter()
            .map(|r| DeckResponse {
                id: r.id.expect("deck.id is NOT NULL in schema"),
                school_id: r.school_id,
                title: r.title,
                description: r.description,
                created_by: r.created_by,
                owner_email: r.owner_email,
                owner_first_name: r.owner_first_name,
                owner_last_name: r.owner_last_name,
                created_at: r.created_at,
                new_count: None,
                learning_count: None,
                due_count: None,
                relearning_count: None,
                total_count: Some(r.card_count),
            })
            .collect();
        return Ok(Json(decks));
    }

    // Students: see decks assigned to their classes, with study counts.
    let rows = sqlx::query!(
        r#"
        SELECT d.id, d.school_id, d.title, d.description,
               d.created_by, d.created_at,
               u.email as owner_email,
               u.first_name as owner_first_name,
               u.last_name as owner_last_name
        FROM decks d
        JOIN users u ON u.id = d.created_by
        JOIN deck_classes dcl ON dcl.deck_id = d.id
        JOIN class_members cm ON cm.class_id = dcl.class_id AND cm.user_id = ?
        WHERE d.school_id = ?
        ORDER BY d.created_at DESC
        "#,
        claims.sub,
        claims.school_id
    )
    .fetch_all(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let mut decks: Vec<DeckResponse> = Vec::new();
    for r in rows {
        let deck_id = r.id.expect("deck.id is NOT NULL in schema");

        // Fetch per-state card counts for this student and deck.
        // LEFT JOIN from cards to student_card_states so we count all cards,
        // treating missing state rows as "new".
        let counts = sqlx::query!(
            r#"
            SELECT
                COUNT(*) as "total!: i64",
                COALESCE(SUM(CASE WHEN scs.student_id IS NULL OR scs.reps = 0 THEN 1 ELSE 0 END), 0) as "new_count!: i64",
                COALESCE(SUM(CASE WHEN scs.state = 'learning' THEN 1 ELSE 0 END), 0) as "learning_count!: i64",
                COALESCE(SUM(CASE WHEN scs.due_at <= unixepoch() AND scs.state IN ('review', 'relearning') THEN 1 ELSE 0 END), 0) as "due_count!: i64",
                COALESCE(SUM(CASE WHEN scs.state = 'relearning' THEN 1 ELSE 0 END), 0) as "relearning_count!: i64"
            FROM cards c
            JOIN notes n ON n.id = c.note_id
            LEFT JOIN student_card_states scs
                ON scs.card_id = c.id AND scs.student_id = ?
            WHERE n.deck_id = ?
            "#,
            claims.sub,
            deck_id
        )
        .fetch_one(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        decks.push(DeckResponse {
            id: deck_id,
            school_id: r.school_id,
            title: r.title,
            description: r.description,
            created_by: r.created_by,
            owner_email: r.owner_email,
            owner_first_name: r.owner_first_name,
            owner_last_name: r.owner_last_name,
            created_at: r.created_at,
            new_count: Some(counts.new_count),
            learning_count: Some(counts.learning_count),
            due_count: Some(counts.due_count),
            relearning_count: Some(counts.relearning_count),
            total_count: Some(counts.total),
        });
    }

    Ok(Json(decks))
}
