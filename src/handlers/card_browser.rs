// Card browser handler.
//
// GET /cards?deck_id=1,2&note_type_id=2,3&state=review,learning&q=DNA&sort=created_at&page=1&per_page=50
//
// Filters support comma-separated lists for multi-value matching:
//   deck_id=1,2      → cards in deck 1 OR 2
//   note_type_id=2,3  → cards of note type 2 OR 3
//   state=new,review  → cards in 'new' OR 'review' state

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use crate::{auth::AuthUser, note_types, state::AppState};

#[derive(Deserialize)]
pub struct CardBrowserQuery {
    /// Comma-separated deck IDs, e.g. "1,2,3"
    #[serde(default)]
    pub deck_id: String,
    /// Comma-separated note type IDs, e.g. "1,2"
    #[serde(default)]
    pub note_type_id: String,
    pub q: Option<String>,
    /// Comma-separated states, e.g. "review,learning"
    #[serde(default)]
    pub state: String,
    /// Comma-separated flag values, e.g. "1,3" for red and green.
    #[serde(default)]
    pub flag: String,
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
    /// Anki-style card flag (0-7). 0 means no flag.
    pub flag: Option<i64>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_card_position: Option<i64>,
}

#[derive(Serialize)]
pub struct CardBrowserPage {
    pub cards: Vec<CardBrowserResponse>,
    pub page: i64,
    pub per_page: i64,
    pub total: i64,
}

async fn rows_to_responses(
    db: &sqlx::SqlitePool,
    rows: Vec<CardBrowserRow>,
    new_card_offset: i64,
) -> Result<Vec<CardBrowserResponse>, StatusCode> {
    let mut cards = Vec::new();
    let mut new_pos = new_card_offset;
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
        let is_new = r.state.as_deref() == Some("new") || r.reps == 0;
        let new_card_position = if is_new {
            new_pos += 1;
            Some(new_pos)
        } else {
            None
        };
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
            flag: Some(r.flag),
            created_at: r.created_at,
            new_card_position,
        });
    }
    Ok(cards)
}

/// Build a comma-separated integer list into an `IN (...)` clause.
/// Returns empty string if the input is empty.
fn in_clause(column: &str, csv: &str) -> String {
    let vals: Vec<&str> = csv
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if vals.is_empty() {
        return String::new();
    }
    // All values come from our own parsing of i64 strings — safe to interpolate.
    format!("AND {} IN ({})", column, vals.join(","))
}

/// Build a WHERE fragment for comma-separated state filters.
/// Supports: new, learning, review, relearning, due.
fn state_where(csv: &str) -> String {
    let states: Vec<&str> = csv
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if states.is_empty() {
        return String::new();
    }

    let mut clauses: Vec<String> = Vec::new();
    for state in states {
        match state {
            "new" => {
                clauses.push("(scs.state = 'new' OR scs.reps = 0 OR scs.state IS NULL)".into())
            }
            "learning" => clauses.push("scs.state = 'learning'".into()),
            "review" => clauses.push("scs.state = 'review'".into()),
            "relearning" => clauses.push("scs.state = 'relearning'".into()),
            "due" => clauses.push(
                "(scs.state IN ('review', 'relearning') AND scs.due_at <= unixepoch())".into(),
            ),
            _ => {}
        }
    }
    if clauses.is_empty() {
        return String::new();
    }
    format!("AND ({})", clauses.join(" OR "))
}

fn sort_clause(sort: &str) -> &'static str {
    match sort {
        "due_at" => "scs.due_at ASC NULLS LAST, c.created_at DESC",
        "deck" => "d.title ASC, c.created_at DESC",
        "question" => "n.fields_json ASC, c.created_at DESC",
        _ => "c.created_at DESC",
    }
}

pub async fn browse_cards(
    AuthUser(claims): AuthUser,
    State(state): State<AppState>,
    Query(params): Query<CardBrowserQuery>,
) -> Result<Json<CardBrowserPage>, (StatusCode, &'static str)> {
    let page = params.page.max(1);
    let per_page = params.per_page.max(1).min(100);
    let offset = (page - 1) * per_page;

    let deck_filter = in_clause("c.deck_id", &params.deck_id);
    let note_type_filter = in_clause("n.note_type_id", &params.note_type_id);
    let q_filter = params
        .q
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .map(|q| format!("AND n.fields_json LIKE '%{}%'", q.trim()))
        .unwrap_or_default();
    let state_filter = state_where(&params.state);
    let flag_filter = in_clause("scs.flag", &params.flag);
    let order = sort_clause(&params.sort);

    let base_from = "FROM cards c JOIN notes n ON n.id = c.note_id JOIN decks d ON d.id = c.deck_id JOIN note_types nt ON nt.id = n.note_type_id LEFT JOIN student_card_states scs ON scs.card_id = c.id AND scs.student_id = $1";
    let base_where = format!(
        "WHERE d.school_id = $2 {} {} {} {} {}",
        deck_filter, note_type_filter, q_filter, state_filter, flag_filter
    );

    // Count
    let count_sql = format!("SELECT COUNT(*) {} {}", base_from, base_where);
    let total: i64 = sqlx::query_scalar(&count_sql)
        .bind(claims.sub)
        .bind(claims.school_id)
        .fetch_one(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    // Fetch
    let fetch_sql = format!(
        "SELECT c.id as card_id, c.note_id, c.deck_id, c.template_index, d.title as deck_title, n.note_type_id, nt.name as note_type_name, n.fields_json, c.created_at, scs.state, scs.due_at, scs.stability, scs.difficulty, scs.reps, scs.lapses, scs.flag {} {} ORDER BY {} LIMIT {} OFFSET {}",
        base_from, base_where, order, per_page, offset
    );

    let rows: Vec<CardBrowserRow> = sqlx::query_as(&fetch_sql)
        .bind(claims.sub)
        .bind(claims.school_id)
        .fetch_all(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    // New card offset for position numbering.
    // We build a separate WHERE that excludes the state filter — we always
    // want to count only new cards regardless of which state the user filtered by.
    let new_card_offset = if let Some(first) = rows.first() {
        let new_where = format!("WHERE d.school_id = $2 {} {}", deck_filter, q_filter);
        let off_sql = format!(
            "SELECT COUNT(*) {} {} AND (scs.state = 'new' OR scs.reps = 0 OR scs.state IS NULL) AND c.created_at > '{}'",
            base_from, new_where, first.created_at
        );
        sqlx::query_scalar(&off_sql)
            .bind(claims.sub)
            .bind(claims.school_id)
            .fetch_one(&state.db)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
    } else {
        0i64
    };

    let cards = rows_to_responses(&state.db, rows, new_card_offset)
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
    flag: i64,
}
