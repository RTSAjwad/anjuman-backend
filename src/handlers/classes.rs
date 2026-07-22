// Class management.
//
// Teachers (and admins) can create, rename, archive, and delete classes.
// They can also manage the roster: add/remove students and other teachers.
//
// All operations are scoped to the caller's school — you cannot
// touch classes, users, or memberships from another school.
//
// ## Role permissions
//
// | Action              | Admin | Teacher | Student |
// |---------------------|-------|---------|---------|
// | Create class        | ✅    | ✅      | ❌      |
// | Rename class        | ✅    | ✅      | ❌      |
// | Archive class       | ✅    | ✅      | ❌      |
// | Delete class        | ✅    | ✅      | ❌      |
// | View roster         | ✅    | ✅      | ✅      |
// | Add member          | ✅    | ✅      | ❌      |
// | Remove member       | ✅    | ✅      | ❌      |
//
// Teachers can only manage classes they created (unless they're an admin).

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
pub struct CreateClass {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct RenameClass {
    pub name: String,
}

#[derive(Deserialize)]
pub struct AddMember {
    /// The user to add to the class.
    pub user_id: i64,
}

#[derive(Serialize)]
pub struct ClassResponse {
    pub id: i64,
    pub school_id: i64,
    pub name: String,
    pub description: Option<String>,
    pub archived: bool,
    pub created_by: i64,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct MemberResponse {
    pub user_id: i64,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub role: String,
    pub joined_at: i64,
}

#[derive(Serialize)]
pub struct RosterResponse {
    pub class: ClassResponse,
    pub members: Vec<MemberResponse>,
}

#[derive(Serialize)]
pub struct MessageResponse {
    pub message: &'static str,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Require that the authenticated user is a teacher or admin.
/// Students cannot manage classes.
fn check_teacher_or_admin(claims: &crate::auth::Claims) -> Result<(), (StatusCode, &'static str)> {
    match claims.role {
        UserRole::Admin | UserRole::Teacher => Ok(()),
        UserRole::Student => Err((
            StatusCode::FORBIDDEN,
            "Only teachers and admins can manage classes",
        )),
    }
}

/// Check whether the caller is authorised to manage a specific class.
/// Admins can manage any class in their school. Teachers can only manage
/// classes they created. Returns Ok(()) if authorised, Err otherwise.
pub async fn check_class_owner(
    db: &sqlx::SqlitePool,
    class_id: i64,
    school_id: i64,
    claims: &crate::auth::Claims,
) -> Result<(), (StatusCode, &'static str)> {
    let row = sqlx::query!(
        "SELECT id, created_by FROM classes WHERE id = ? AND school_id = ?",
        class_id,
        school_id
    )
    .fetch_optional(db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "Class not found"))?;

    // Admins can manage any class in their school.
    if claims.role == UserRole::Admin {
        return Ok(());
    }

    // Teachers can only manage classes they created.
    if row.created_by != claims.sub {
        return Err((
            StatusCode::FORBIDDEN,
            "You can only manage classes you created",
        ));
    }

    Ok(())
}

/// Check whether the caller is a member of the class (or owner, or admin).
/// Returns Ok(()) if the caller can view the class.
pub async fn check_class_member(
    db: &sqlx::SqlitePool,
    class_id: i64,
    school_id: i64,
    claims: &crate::auth::Claims,
) -> Result<(), (StatusCode, &'static str)> {
    let row = sqlx::query!(
        "SELECT id, created_by FROM classes WHERE id = ? AND school_id = ?",
        class_id,
        school_id
    )
    .fetch_optional(db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "Class not found"))?;

    // Admin can view any class in their school.
    if claims.role == UserRole::Admin {
        return Ok(());
    }

    // Owner can view their own class.
    if row.created_by == claims.sub {
        return Ok(());
    }

    // Check if the user is a class member.
    let is_member = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM class_members WHERE class_id = ? AND user_id = ?",
        class_id,
        claims.sub
    )
    .fetch_one(db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if is_member > 0 {
        return Ok(());
    }

    Err((StatusCode::FORBIDDEN, "You are not a member of this class"))
}

/// Fetch a class row and convert to response DTO.
async fn fetch_class(
    db: &sqlx::SqlitePool,
    class_id: i64,
) -> Result<ClassResponse, (StatusCode, &'static str)> {
    let row = sqlx::query!(
        r#"
        SELECT id, school_id, name, description, archived, created_by, created_at
        FROM classes WHERE id = ?
        "#,
        class_id
    )
    .fetch_one(db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(ClassResponse {
        id: row.id,
        school_id: row.school_id,
        name: row.name,
        description: row.description,
        archived: row.archived != 0,
        created_by: row.created_by,
        created_at: row.created_at,
    })
}

// ---------------------------------------------------------------------------
// Class CRUD
// ---------------------------------------------------------------------------

/// `POST /classes` — Create a new class.
pub async fn create_class(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateClass>,
) -> Result<(StatusCode, Json<ClassResponse>), (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;

    let result = sqlx::query!(
        r#"
        INSERT INTO classes (school_id, name, description, created_by, created_at)
        VALUES (?, ?, ?, ?, unixepoch())
        "#,
        claims.school_id,
        body.name,
        body.description,
        claims.sub
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let class = fetch_class(&state.db, result.last_insert_rowid()).await?;
    Ok((StatusCode::CREATED, Json(class)))
}

/// `GET /classes` — List classes the caller belongs to.
///
/// Admins see all classes in their school (including archived).
/// Teachers and students see classes they own or are members of.
/// Archived classes are excluded for non-admins by default.
pub async fn list_classes(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<ClassResponse>>, (StatusCode, &'static str)> {
    if claims.role == UserRole::Admin {
        // Admins see all classes in their school.
        let rows = sqlx::query!(
            r#"
            SELECT id, school_id, name, description, archived, created_by, created_at
            FROM classes
            WHERE school_id = ?
            ORDER BY name
            "#,
            claims.school_id
        )
        .fetch_all(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        let classes: Vec<ClassResponse> = rows
            .into_iter()
            .map(|r| ClassResponse {
                id: r.id.expect("id from simple SELECT is never null"),
                school_id: r.school_id,
                name: r.name,
                description: r.description,
                archived: r.archived != 0,
                created_by: r.created_by,
                created_at: r.created_at,
            })
            .collect();
        return Ok(Json(classes));
    }

    // Teachers and students: classes they own or are members of.
    let rows = sqlx::query!(
        r#"
        SELECT DISTINCT c.id, c.school_id, c.name, c.description,
               c.archived, c.created_by, c.created_at
        FROM classes c
        LEFT JOIN class_members cm ON cm.class_id = c.id
        WHERE c.school_id = ?
          AND c.archived = 0
          AND (c.created_by = ? OR cm.user_id = ?)
        ORDER BY c.name
        "#,
        claims.school_id,
        claims.sub,
        claims.sub
    )
    .fetch_all(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let classes: Vec<ClassResponse> = rows
        .into_iter()
        .map(|r| ClassResponse {
            id: r.id.expect("id from SELECT DISTINCT is never null"),
            school_id: r.school_id,
            name: r.name,
            description: r.description,
            archived: r.archived != 0,
            created_by: r.created_by,
            created_at: r.created_at,
        })
        .collect();

    Ok(Json(classes))
}

/// `GET /classes/:id` — Get a single class.
///
/// The caller must be a member, the owner, or an admin.
pub async fn get_class(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(class_id): Path<i64>,
) -> Result<Json<ClassResponse>, (StatusCode, &'static str)> {
    check_class_member(&state.db, class_id, claims.school_id, &claims).await?;
    let class = fetch_class(&state.db, class_id).await?;
    Ok(Json(class))
}

/// `PATCH /classes/:id/rename` — Rename a class.
pub async fn rename_class(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(class_id): Path<i64>,
    Json(body): Json<RenameClass>,
) -> Result<Json<ClassResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    check_class_member(&state.db, class_id, claims.school_id, &claims).await?;

    sqlx::query!(
        "UPDATE classes SET name = ? WHERE id = ?",
        body.name,
        class_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let class = fetch_class(&state.db, class_id).await?;
    Ok(Json(class))
}

/// `POST /classes/:id/archive` — Toggle the archive status of a class.
///
/// If the class is active, it becomes archived. If archived, it becomes
/// active again. Archived classes are hidden from the default class list
/// but retain all their data.
pub async fn archive_class(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(class_id): Path<i64>,
) -> Result<Json<ClassResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    check_class_member(&state.db, class_id, claims.school_id, &claims).await?;

    // Fetch current state, flip it, and update.
    let current = sqlx::query!("SELECT archived FROM classes WHERE id = ?", class_id)
        .fetch_one(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let new_archived = if current.archived != 0 { 0 } else { 1 };

    sqlx::query!(
        "UPDATE classes SET archived = ? WHERE id = ?",
        new_archived,
        class_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let class = fetch_class(&state.db, class_id).await?;
    Ok(Json(class))
}

/// `DELETE /classes/:id` — Delete a class permanently.
///
/// Cascade deletes remove all memberships and related data.
pub async fn delete_class(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(class_id): Path<i64>,
) -> Result<Json<MessageResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    check_class_owner(&state.db, class_id, claims.school_id, &claims).await?;

    sqlx::query!("DELETE FROM classes WHERE id = ?", class_id)
        .execute(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(Json(MessageResponse {
        message: "Class deleted",
    }))
}

// ---------------------------------------------------------------------------
// Roster management
// ---------------------------------------------------------------------------

/// `GET /classes/:id/roster` — View the class details and all members.
///
/// The caller must be a member, the owner, or an admin to view the roster.
pub async fn view_roster(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(class_id): Path<i64>,
) -> Result<Json<RosterResponse>, (StatusCode, &'static str)> {
    check_class_member(&state.db, class_id, claims.school_id, &claims).await?;

    // Fetch the class — scoped to the caller's school.
    let class_row = sqlx::query!(
        r#"
        SELECT id, school_id, name, description, archived, created_by, created_at
        FROM classes
        WHERE id = ? AND school_id = ?
        "#,
        class_id,
        claims.school_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "Class not found"))?;

    // Fetch all members with their user details.
    let members = sqlx::query!(
        r#"
        SELECT cm.user_id, u.email, u.first_name, u.last_name, cm.role, cm.joined_at
        FROM class_members cm
        JOIN users u ON u.id = cm.user_id
        WHERE cm.class_id = ?
        ORDER BY cm.role, u.email
        "#,
        class_id
    )
    .fetch_all(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let member_list: Vec<MemberResponse> = members
        .into_iter()
        .map(|m| MemberResponse {
            user_id: m.user_id,
            email: m.email,
            first_name: m.first_name,
            last_name: m.last_name,
            role: m.role,
            joined_at: m.joined_at,
        })
        .collect();

    Ok(Json(RosterResponse {
        class: ClassResponse {
            id: class_row.id,
            school_id: class_row.school_id,
            name: class_row.name,
            description: class_row.description,
            archived: class_row.archived != 0,
            created_by: class_row.created_by,
            created_at: class_row.created_at,
        },
        members: member_list,
    }))
}

/// `POST /classes/:id/members` — Add a teacher or student to the class.
///
/// The target user must exist in the same school. If the user is already
/// a member, their role is updated (upsert behaviour).
pub async fn add_member(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(class_id): Path<i64>,
    Json(body): Json<AddMember>,
) -> Result<(StatusCode, Json<MemberResponse>), (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    check_class_member(&state.db, class_id, claims.school_id, &claims).await?;

    let target = sqlx::query!(
        "SELECT id, email, first_name, last_name, role FROM users WHERE id = ? AND school_id = ?",
        body.user_id,
        claims.school_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    .ok_or((StatusCode::NOT_FOUND, "User not found in your school"))?;

    if target.role == "admin" {
        return Err((StatusCode::BAD_REQUEST, "Admins cannot be added to classes"));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    sqlx::query!(
        r#"
        INSERT INTO class_members (class_id, user_id, role, joined_at)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(class_id, user_id) DO UPDATE SET role = excluded.role
        "#,
        class_id,
        body.user_id,
        target.role,
        now
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok((
        StatusCode::CREATED,
        Json(MemberResponse {
            user_id: body.user_id,
            email: target.email,
            first_name: target.first_name,
            last_name: target.last_name,
            role: target.role,
            joined_at: now,
        }),
    ))
}

/// `DELETE /classes/:id/members/:user_id` — Remove a member from the class.
pub async fn remove_member(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path((class_id, user_id)): Path<(i64, i64)>,
) -> Result<Json<MessageResponse>, (StatusCode, &'static str)> {
    check_teacher_or_admin(&claims)?;
    check_class_member(&state.db, class_id, claims.school_id, &claims).await?;

    let result = sqlx::query!(
        "DELETE FROM class_members WHERE class_id = ? AND user_id = ?",
        class_id,
        user_id
    )
    .execute(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Member not found in class"));
    }

    Ok(Json(MessageResponse {
        message: "Member removed",
    }))
}
