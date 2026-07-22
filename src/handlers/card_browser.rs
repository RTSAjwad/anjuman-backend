// Card browser handler.
//
// GET /cards?deck_id=1&q=DNA&sort=created_at&page=1&per_page=50

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use crate::{auth::AuthUser, note_types, state::AppState};

#[derive(Deserialize)]
pub struct CardBrowserQuery {
    pub deck_id: Option<i64>,
    pub q: Option<String>,
    #[serde(default = "default_sort")]
    pub sort: String,
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_per_page")]
    pub per_page: i64,
}

fn default_sort() -> String {
    "created_at".to_string()
}
fn default_page() -> i64 {
    1
}
fn default_per_page() -> i64 {
    50
}

#[derive(Serialize)]
pub struct CardBrowserResponse {
    pub card_id: i64,
    pub note_id: i64,
    pub deck_id: i64,
    pub deck_title: String,
    pub template_index: i64,
    pub front: String,
    pub back: String,
    pub note_type_name: String,
    pub fields: serde_json::Map<String, serde_json::Value>,
    pub state: Option<String>,
    pub due_at: Option<i64>,
    pub stability: Option<f64>,
    pub difficulty: Option<f64>,
    pub reps: Option<i64>,
    pub lapses: Option<i64>,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct CardBrowserPage {
    pub cards: Vec<CardBrowserResponse>,
    pub page: i64,
    pub per_page: i64,
    pub total: i64,
}

// Helper to render cards from rows.
async fn rows_to_responses(
    db: &sqlx::SqlitePool,
    rows: Vec<CardBrowserRow>,
) -> Result<Vec<CardBrowserResponse>, StatusCode> {
    let mut cards = Vec::new();
    for r in rows {
        let fields: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&r.fields_json).unwrap_or_default();

        let nt = note_types::get_note_type(db, r.note_type_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let rendered = note_types::render_card(&nt.templates, r.template_index, &fields)
            .unwrap_or_else(|| note_types::RenderedCard {
                template_index: r.template_index,
                front: "(unknown)".to_string(),
                back: String::new(),
            });

        cards.push(CardBrowserResponse {
            card_id: r.card_id,
            note_id: r.note_id,
            deck_id: r.deck_id,
            deck_title: r.deck_title,
            template_index: r.template_index,
            front: rendered.front,
            back: rendered.back,
            note_type_name: r.note_type_name,
            fields,
            state: r.state,
            due_at: r.due_at,
            stability: Some(r.stability),
            difficulty: Some(r.difficulty),
            reps: Some(r.reps),
            lapses: Some(r.lapses),
            created_at: r.created_at,
        });
    }
    Ok(cards)
}

pub async fn browse_cards(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Query(params): Query<CardBrowserQuery>,
) -> Result<Json<CardBrowserPage>, (StatusCode, &'static str)> {
    let page = params.page.max(1);
    let per_page = params.per_page.max(1).min(100);
    let offset = (page - 1) * per_page;
    let student_id = claims.sub;
    let school_id = claims.school_id;

    let has_deck = params.deck_id.is_some();
    let pattern = params
        .q
        .as_ref()
        .map(|s| format!("%{}%", s.trim()))
        .unwrap_or_default();
    let has_q = !pattern.is_empty();

    // Use separate queries per combination to keep sqlx happy.
    let (total, rows) = if has_deck && has_q {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cards c JOIN notes n ON n.id = c.note_id JOIN decks d ON d.id = c.deck_id WHERE d.school_id = ? AND c.deck_id = ? AND n.fields_json LIKE ?"
        ).bind(school_id).bind(params.deck_id).bind(&pattern).fetch_one(&state.db).await.map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        let rows = sqlx::query_as::<_, CardBrowserRow>(
            "SELECT c.id as card_id, c.note_id, c.deck_id, c.template_index, d.title as deck_title, n.note_type_id, nt.name as note_type_name, n.fields_json, c.created_at, scs.state, scs.due_at, scs.stability, scs.difficulty, scs.reps, scs.lapses FROM cards c JOIN notes n ON n.id = c.note_id JOIN decks d ON d.id = c.deck_id JOIN note_types nt ON nt.id = n.note_type_id LEFT JOIN student_card_states scs ON scs.card_id = c.id AND scs.student_id = ? WHERE d.school_id = ? AND c.deck_id = ? AND n.fields_json LIKE ? ORDER BY c.created_at DESC LIMIT ? OFFSET ?"
        ).bind(student_id).bind(school_id).bind(params.deck_id).bind(&pattern).bind(per_page).bind(offset).fetch_all(&state.db).await.map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        (total, rows)
    } else if has_deck {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cards c JOIN notes n ON n.id = c.note_id JOIN decks d ON d.id = c.deck_id WHERE d.school_id = ? AND c.deck_id = ?"
        ).bind(school_id).bind(params.deck_id).fetch_one(&state.db).await.map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        let rows = sqlx::query_as::<_, CardBrowserRow>(
            "SELECT c.id as card_id, c.note_id, c.deck_id, c.template_index, d.title as deck_title, n.note_type_id, nt.name as note_type_name, n.fields_json, c.created_at, scs.state, scs.due_at, scs.stability, scs.difficulty, scs.reps, scs.lapses FROM cards c JOIN notes n ON n.id = c.note_id JOIN decks d ON d.id = c.deck_id JOIN note_types nt ON nt.id = n.note_type_id LEFT JOIN student_card_states scs ON scs.card_id = c.id AND scs.student_id = ? WHERE d.school_id = ? AND c.deck_id = ? ORDER BY c.created_at DESC LIMIT ? OFFSET ?"
        ).bind(student_id).bind(school_id).bind(params.deck_id).bind(per_page).bind(offset).fetch_all(&state.db).await.map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        (total, rows)
    } else if has_q {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cards c JOIN notes n ON n.id = c.note_id JOIN decks d ON d.id = c.deck_id WHERE d.school_id = ? AND n.fields_json LIKE ?"
        ).bind(school_id).bind(&pattern).fetch_one(&state.db).await.map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        let rows = sqlx::query_as::<_, CardBrowserRow>(
            "SELECT c.id as card_id, c.note_id, c.deck_id, c.template_index, d.title as deck_title, n.note_type_id, nt.name as note_type_name, n.fields_json, c.created_at, scs.state, scs.due_at, scs.stability, scs.difficulty, scs.reps, scs.lapses FROM cards c JOIN notes n ON n.id = c.note_id JOIN decks d ON d.id = c.deck_id JOIN note_types nt ON nt.id = n.note_type_id LEFT JOIN student_card_states scs ON scs.card_id = c.id AND scs.student_id = ? WHERE d.school_id = ? AND n.fields_json LIKE ? ORDER BY c.created_at DESC LIMIT ? OFFSET ?"
        ).bind(student_id).bind(school_id).bind(&pattern).bind(per_page).bind(offset).fetch_all(&state.db).await.map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        (total, rows)
    } else {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cards c JOIN notes n ON n.id = c.note_id JOIN decks d ON d.id = c.deck_id WHERE d.school_id = ?"
        ).bind(school_id).fetch_one(&state.db).await.map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        let rows = sqlx::query_as::<_, CardBrowserRow>(
            "SELECT c.id as card_id, c.note_id, c.deck_id, c.template_index, d.title as deck_title, n.note_type_id, nt.name as note_type_name, n.fields_json, c.created_at, scs.state, scs.due_at, scs.stability, scs.difficulty, scs.reps, scs.lapses FROM cards c JOIN notes n ON n.id = c.note_id JOIN decks d ON d.id = c.deck_id JOIN note_types nt ON nt.id = n.note_type_id LEFT JOIN student_card_states scs ON scs.card_id = c.id AND scs.student_id = ? WHERE d.school_id = ? ORDER BY c.created_at DESC LIMIT ? OFFSET ?"
        ).bind(student_id).bind(school_id).bind(per_page).bind(offset).fetch_all(&state.db).await.map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        (total, rows)
    };

    let cards = rows_to_responses(&state.db, rows)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(Json(CardBrowserPage {
        cards,
        page,
        per_page,
        total,
    }))
}

#[derive(sqlx::FromRow)]
struct CardBrowserRow {
    card_id: i64,
    note_id: i64,
    deck_id: i64,
    template_index: i64,
    deck_title: String,
    note_type_id: i64,
    note_type_name: String,
    fields_json: String,
    created_at: String,
    state: Option<String>,
    due_at: Option<i64>,
    stability: f64,
    difficulty: f64,
    reps: i64,
    lapses: i64,
}
