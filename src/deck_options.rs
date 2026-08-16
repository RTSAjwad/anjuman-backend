// Deck options presets and resolution.
//
// Mirrors Anki's deck options ("dconf") model: decks reference a named preset
// via `options_id`. A deck with no preset falls back to built-in defaults.
//
// A preset stores scheduling steps (in seconds, comma-separated), desired
// retention, sibling-bury toggles, and daily limits (the limits are stored
// now but not yet enforced).

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A deck options preset as stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeckOptions {
    pub id: i64,
    pub school_id: i64,
    pub name: String,
    pub learning_steps: Vec<i64>,
    pub relearning_steps: Vec<i64>,
    pub desired_retention: f64,
    pub bury_new: bool,
    pub bury_review: bool,
    pub bury_interday: bool,
    pub new_per_day: i64,
    pub review_per_day: i64,
}

/// Built-in defaults used when a deck has no assigned preset.
pub fn default_options() -> DeckOptions {
    DeckOptions {
        id: 0,
        school_id: 0,
        name: "Default".to_string(),
        learning_steps: vec![60, 600],
        relearning_steps: vec![600],
        desired_retention: 0.9,
        bury_new: false,
        bury_review: false,
        bury_interday: false,
        new_per_day: 20,
        review_per_day: 200,
    }
}

// ---------------------------------------------------------------------------
// Parsing / serialising step lists
// ---------------------------------------------------------------------------

/// Parse a comma-separated seconds list (as stored in the DB).
pub fn parse_steps_csv(csv: &str) -> Vec<i64> {
    csv.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<i64>().ok())
        .collect()
}

/// Serialise a seconds list to a comma-separated string.
pub fn steps_to_csv(steps: &[i64]) -> String {
    steps
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

// ---------------------------------------------------------------------------
// Database access
// ---------------------------------------------------------------------------

/// Fetch a preset by id.
pub async fn get_options(db: &SqlitePool, id: i64) -> Result<DeckOptions, String> {
    let row = sqlx::query!(
        "SELECT id, school_id, name, learning_steps, relearning_steps, desired_retention, bury_new, bury_review, bury_interday, new_per_day, review_per_day FROM deck_options WHERE id = ?",
        id
    )
    .fetch_optional(db)
    .await
    .map_err(|e| format!("Database error: {e}"))?
    .ok_or_else(|| format!("Deck options {id} not found"))?;

    Ok(DeckOptions {
        id: row.id,
        school_id: row.school_id,
        name: row.name,
        learning_steps: parse_steps_csv(&row.learning_steps),
        relearning_steps: parse_steps_csv(&row.relearning_steps),
        desired_retention: row.desired_retention,
        bury_new: row.bury_new != 0,
        bury_review: row.bury_review != 0,
        bury_interday: row.bury_interday != 0,
        new_per_day: row.new_per_day,
        review_per_day: row.review_per_day,
    })
}

/// Resolve the effective options for a deck.
///
/// If the deck has an `options_id`, return that preset; otherwise return the
/// built-in defaults. Returns `None` if the deck doesn't exist.
pub async fn options_for_deck(db: &SqlitePool, deck_id: i64) -> Result<DeckOptions, String> {
    let options_id: Option<Option<i64>> =
        sqlx::query_scalar("SELECT options_id FROM decks WHERE id = ?")
            .bind(deck_id)
            .fetch_optional(db)
            .await
            .map_err(|e| format!("Database error: {e}"))?
            .ok_or_else(|| format!("Deck {deck_id} not found"))?;

    match options_id {
        Some(Some(id)) => get_options(db, id).await,
        _ => Ok(default_options()),
    }
}
