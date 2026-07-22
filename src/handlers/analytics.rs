// Analytics & progress tracking.
//
// Student-facing:
//   GET /analytics/me              — Personal stats summary
//   GET /analytics/me/daily        — Daily review breakdown (30 days)
//
// Teacher-facing:
//   GET /analytics/classes/:id     — Class overview with per-student table
//   GET /analytics/classes/:id/students/:student_id  — Drill into one student

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::TimeZone;
use serde::Serialize;
use sqlx::FromRow;

use crate::{
    auth::AuthUser,
    handlers::{classes, users::UserRole},
    note_types,
    state::AppState,
};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct StudentStats {
    pub student_id: i64,
    pub total_reviews: i64,
    pub reviews_today: i64,
    pub retention_rate: f64,
    pub average_rating: f64,
    pub cards_total: i64,
    pub cards_mastered: i64,
    pub cards_learning: i64,
    pub cards_struggling: i64,
    pub study_streak_days: i64,
    pub time_spent_today_seconds: i64,
    pub sessions_this_week: i64,
}

#[derive(Serialize)]
pub struct StudentStatsWithEmail {
    pub student_id: i64,
    pub email: String,
    pub total_reviews: i64,
    pub retention_rate: f64,
    pub cards_mastered: i64,
    pub last_active: Option<String>,
}

#[derive(Serialize)]
pub struct DailyPoint {
    pub date: String,
    pub reviews: i64,
    pub avg_rating: f64,
    pub avg_time_ms: f64,
}

#[derive(Serialize)]
pub struct DifficultCard {
    pub card_id: i64,
    pub front: String,
    pub avg_rating: f64,
    pub total_reviews: i64,
}

#[derive(Serialize)]
pub struct ClassAnalytics {
    pub class_name: String,
    pub students_enrolled: i64,
    pub average_retention: f64,
    pub most_difficult_cards: Vec<DifficultCard>,
    pub students: Vec<StudentStatsWithEmail>,
}

#[derive(Serialize)]
pub struct StudentDetail {
    pub student_id: i64,
    pub email: String,
    pub total_reviews: i64,
    pub reviews_today: i64,
    pub retention_rate: f64,
    pub average_rating: f64,
    pub cards_total: i64,
    pub cards_mastered: i64,
    pub cards_learning: i64,
    pub cards_struggling: i64,
    pub study_streak_days: i64,
    pub time_spent_today_seconds: i64,
    pub sessions_this_week: i64,
    pub daily: Vec<DailyPoint>,
}

// ---------------------------------------------------------------------------
// Row structs for sqlx::query_as
// ---------------------------------------------------------------------------

#[derive(FromRow)]
struct DailyRow {
    day_start: i64,
    review_count: i64,
    avg_rating: f64,
    avg_time: f64,
}

#[derive(FromRow)]
struct RetentionRow {
    total: i64,
    good: i64,
}

#[derive(FromRow)]
struct DifficultCardRow {
    card_id: i64,
    template_index: i64,
    note_type_id: i64,
    fields_json: String,
    avg_rating: f64,
    total_reviews: i64,
}

#[derive(FromRow)]
struct StudentRow {
    student_id: i64,
    email: String,
}

#[derive(FromRow)]
struct ReviewDay {
    reviewed_at: i64,
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

fn start_of_today() -> i64 {
    let now = now_secs();
    now - (now % 86400)
}

fn start_of_week() -> i64 {
    let now = now_secs();
    let days_since_epoch = now / 86400;
    let days_since_monday = (days_since_epoch + 3) % 7;
    (days_since_epoch - days_since_monday) * 86400
}

fn days_since_epoch(ts: i64) -> i64 {
    ts / 86400
}

fn format_date(ts: i64) -> String {
    chrono::Utc
        .timestamp_opt(ts, 0)
        .unwrap()
        .format("%Y-%m-%d")
        .to_string()
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

// ---------------------------------------------------------------------------
// Core metrics for a single student
// ---------------------------------------------------------------------------

struct CoreMetrics {
    total_reviews: i64,
    reviews_today: i64,
    retention_rate: f64,
    average_rating: f64,
    cards_total: i64,
    cards_mastered: i64,
    cards_learning: i64,
    cards_struggling: i64,
    study_streak_days: i64,
    time_spent_today_seconds: i64,
    sessions_this_week: i64,
}

async fn compute_core_metrics(
    db: &sqlx::SqlitePool,
    student_id: i64,
) -> Result<CoreMetrics, StatusCode> {
    let today = start_of_today();
    let week = start_of_week();

    let total_reviews: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM reviews WHERE student_id = ?")
            .bind(student_id)
            .fetch_one(db)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let reviews_today: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM reviews WHERE student_id = ? AND reviewed_at >= ?",
    )
    .bind(student_id)
    .bind(today)
    .fetch_one(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let retention_rate = if total_reviews > 0 {
        let good_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM reviews WHERE student_id = ? AND rating >= 3")
                .bind(student_id)
                .fetch_one(db)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        round2(good_count as f64 / total_reviews as f64)
    } else {
        0.0
    };

    let average_rating = if total_reviews > 0 {
        let avg: f64 = sqlx::query_scalar(
            "SELECT AVG(CAST(rating AS REAL)) FROM reviews WHERE student_id = ?",
        )
        .bind(student_id)
        .fetch_one(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        round2(avg)
    } else {
        0.0
    };

    let cards_total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM student_card_states WHERE student_id = ?")
            .bind(student_id)
            .fetch_one(db)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let cards_mastered: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM student_card_states WHERE student_id = ? AND state = 'review' AND stability > 10.0"
    )
    .bind(student_id)
    .fetch_one(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let cards_learning: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM student_card_states WHERE student_id = ? AND state IN ('new', 'learning')"
    )
    .bind(student_id)
    .fetch_one(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let cards_struggling: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM student_card_states WHERE student_id = ? AND lapses > 2",
    )
    .bind(student_id)
    .fetch_one(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Study streak.
    let days = sqlx::query_as::<_, ReviewDay>(
        "SELECT DISTINCT reviewed_at FROM reviews WHERE student_id = ? ORDER BY reviewed_at DESC LIMIT 365"
    )
    .bind(student_id)
    .fetch_all(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let today_day = days_since_epoch(today);
    let mut review_days: std::collections::HashSet<i64> = days
        .iter()
        .map(|r| days_since_epoch(r.reviewed_at))
        .collect();

    if reviews_today > 0 {
        review_days.insert(days_since_epoch(now_secs()));
    }

    let mut check_day = if reviews_today > 0 {
        today_day
    } else {
        today_day - 1
    };
    let mut streak = 0i64;
    while review_days.contains(&check_day) {
        streak += 1;
        check_day -= 1;
    }

    let time_spent_today_seconds: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(response_time_ms), 0) / 1000 FROM reviews WHERE student_id = ? AND reviewed_at >= ?"
    )
    .bind(student_id)
    .bind(today)
    .fetch_one(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let sessions_this_week: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT reviewed_at / 86400) FROM reviews WHERE student_id = ? AND reviewed_at >= ?",
    )
    .bind(student_id)
    .bind(week)
    .fetch_one(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(CoreMetrics {
        total_reviews,
        reviews_today,
        retention_rate,
        average_rating,
        cards_total,
        cards_mastered,
        cards_learning,
        cards_struggling,
        study_streak_days: streak,
        time_spent_today_seconds,
        sessions_this_week,
    })
}

async fn compute_daily_breakdown(
    db: &sqlx::SqlitePool,
    student_id: i64,
    days: i64,
) -> Result<Vec<DailyPoint>, StatusCode> {
    let since = start_of_today() - (days * 86400);

    let rows = sqlx::query_as::<_, DailyRow>(
        r#"
        SELECT
            (reviewed_at / 86400) * 86400 as day_start,
            COUNT(*) as review_count,
            AVG(CAST(rating AS REAL)) as avg_rating,
            COALESCE(AVG(CAST(response_time_ms AS REAL)), 0) as avg_time
        FROM reviews
        WHERE student_id = ? AND reviewed_at >= ?
        GROUP BY day_start
        ORDER BY day_start ASC
        "#,
    )
    .bind(student_id)
    .bind(since)
    .fetch_all(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(rows
        .into_iter()
        .map(|r| DailyPoint {
            date: format_date(r.day_start),
            reviews: r.review_count,
            avg_rating: round2(r.avg_rating),
            avg_time_ms: round2(r.avg_time),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Student-facing endpoints
// ---------------------------------------------------------------------------

pub async fn my_stats(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<StudentStats>, StatusCode> {
    let m = compute_core_metrics(&state.db, claims.sub).await?;

    Ok(Json(StudentStats {
        student_id: claims.sub,
        total_reviews: m.total_reviews,
        reviews_today: m.reviews_today,
        retention_rate: m.retention_rate,
        average_rating: m.average_rating,
        cards_total: m.cards_total,
        cards_mastered: m.cards_mastered,
        cards_learning: m.cards_learning,
        cards_struggling: m.cards_struggling,
        study_streak_days: m.study_streak_days,
        time_spent_today_seconds: m.time_spent_today_seconds,
        sessions_this_week: m.sessions_this_week,
    }))
}

pub async fn my_daily(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<DailyPoint>>, StatusCode> {
    compute_daily_breakdown(&state.db, claims.sub, 30)
        .await
        .map(Json)
}

// ---------------------------------------------------------------------------
// Teacher-facing endpoints
// ---------------------------------------------------------------------------

async fn check_class_access(
    db: &sqlx::SqlitePool,
    class_id: i64,
    school_id: i64,
    claims: &crate::auth::Claims,
) -> Result<(), (StatusCode, &'static str)> {
    if claims.role == UserRole::Student {
        return Err((
            StatusCode::FORBIDDEN,
            "Only teachers and admins can view class analytics",
        ));
    }
    classes::check_class_member(db, class_id, school_id, claims).await
}

pub async fn class_analytics(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path(class_id): Path<i64>,
) -> Result<Json<ClassAnalytics>, (StatusCode, &'static str)> {
    check_class_access(&state.db, class_id, claims.school_id, &claims).await?;

    let class_name: String = sqlx::query_scalar("SELECT name FROM classes WHERE id = ?")
        .bind(class_id)
        .fetch_one(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let students_enrolled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM class_members WHERE class_id = ? AND role = 'student'",
    )
    .bind(class_id)
    .fetch_one(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    // Average retention: per-student average of their individual retention rates.
    let retention_rows = sqlx::query_as::<_, RetentionRow>(
        r#"
        SELECT COUNT(*) as total, SUM(CASE WHEN r.rating >= 3 THEN 1 ELSE 0 END) as good
        FROM reviews r
        JOIN class_members cm ON cm.user_id = r.student_id AND cm.class_id = ?
        GROUP BY r.student_id
        "#,
    )
    .bind(class_id)
    .fetch_all(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let average_retention = if retention_rows.is_empty() {
        0.0
    } else {
        let sum: f64 = retention_rows
            .iter()
            .map(|r| {
                if r.total > 0 {
                    r.good as f64 / r.total as f64
                } else {
                    0.0
                }
            })
            .sum();
        round2(sum / retention_rows.len() as f64)
    };

    // Most difficult cards.
    let difficult_cards = sqlx::query_as::<_, DifficultCardRow>(
        r#"
        SELECT r.card_id, c.template_index, n.note_type_id, n.fields_json,
               AVG(CAST(r.rating AS REAL)) as avg_rating, COUNT(*) as total_reviews
        FROM reviews r
        JOIN cards c ON c.id = r.card_id
        JOIN notes n ON n.id = c.note_id
        JOIN class_members cm ON cm.user_id = r.student_id AND cm.class_id = ?
        GROUP BY r.card_id
        HAVING COUNT(*) >= 5
        ORDER BY avg_rating ASC
        LIMIT 5
        "#,
    )
    .bind(class_id)
    .fetch_all(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let mut most_difficult_cards: Vec<DifficultCard> = Vec::new();
    for c in difficult_cards {
        let fields: note_types::NoteFields = serde_json::from_str(&c.fields_json)
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Invalid JSON"))?;
        let nt = crate::note_types::get_note_type(&state.db, c.note_type_id)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;
        let rendered = crate::note_types::render_card(&nt.templates, c.template_index, &fields)
            .unwrap_or_else(|| crate::note_types::RenderedCard {
                template_index: c.template_index,
                front: "(unknown)".to_string(),
                back: String::new(),
            });
        most_difficult_cards.push(DifficultCard {
            card_id: c.card_id,
            front: rendered.front,
            avg_rating: round2(c.avg_rating),
            total_reviews: c.total_reviews,
        });
    }

    // Per-student rows.
    let students = sqlx::query_as::<_, StudentRow>(
        "SELECT u.id as student_id, u.email FROM class_members cm JOIN users u ON u.id = cm.user_id WHERE cm.class_id = ? AND cm.role = 'student' ORDER BY u.email"
    )
    .bind(class_id)
    .fetch_all(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let mut student_rows: Vec<StudentStatsWithEmail> = Vec::new();

    for s in students {
        let sid = s.student_id;

        let total_reviews: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM reviews WHERE student_id = ?")
                .bind(sid)
                .fetch_one(&state.db)
                .await
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        let retention_rate = if total_reviews > 0 {
            let good: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM reviews WHERE student_id = ? AND rating >= 3",
            )
            .bind(sid)
            .fetch_one(&state.db)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;
            round2(good as f64 / total_reviews as f64)
        } else {
            0.0
        };

        let cards_mastered: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM student_card_states WHERE student_id = ? AND state = 'review' AND stability > 10.0"
        )
        .bind(sid)
        .fetch_one(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        let last_active: Option<i64> =
            sqlx::query_scalar("SELECT MAX(reviewed_at) FROM reviews WHERE student_id = ?")
                .bind(sid)
                .fetch_one(&state.db)
                .await
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        student_rows.push(StudentStatsWithEmail {
            student_id: sid,
            email: s.email,
            total_reviews,
            retention_rate,
            cards_mastered,
            last_active: last_active.map(format_date),
        });
    }

    Ok(Json(ClassAnalytics {
        class_name,
        students_enrolled,
        average_retention,
        most_difficult_cards,
        students: student_rows,
    }))
}

pub async fn student_detail(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Path((class_id, student_id)): Path<(i64, i64)>,
) -> Result<Json<StudentDetail>, (StatusCode, &'static str)> {
    check_class_access(&state.db, class_id, claims.school_id, &claims).await?;

    let enrolled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM class_members WHERE class_id = ? AND user_id = ? AND role = 'student'"
    )
    .bind(class_id)
    .bind(student_id)
    .fetch_one(&state.db)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    if enrolled == 0 {
        return Err((StatusCode::NOT_FOUND, "Student not found in this class"));
    }

    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id = ?")
        .bind(student_id)
        .fetch_one(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let m = compute_core_metrics(&state.db, student_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let daily = compute_daily_breakdown(&state.db, student_id, 30)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(Json(StudentDetail {
        student_id,
        email,
        total_reviews: m.total_reviews,
        reviews_today: m.reviews_today,
        retention_rate: m.retention_rate,
        average_rating: m.average_rating,
        cards_total: m.cards_total,
        cards_mastered: m.cards_mastered,
        cards_learning: m.cards_learning,
        cards_struggling: m.cards_struggling,
        study_streak_days: m.study_streak_days,
        time_spent_today_seconds: m.time_spent_today_seconds,
        sessions_this_week: m.sessions_this_week,
        daily,
    }))
}
