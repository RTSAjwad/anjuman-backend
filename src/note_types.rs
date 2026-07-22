// Note type definitions and card rendering.
//
// Note types are stored in the database (not hardcoded). Each note type
// has a name, a set of field names, and one or more card templates.
//
// Templates define front and back patterns using `{{FieldName}}` placeholders.
// Cards are rendered at display time from the template patterns + note fields.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A note type definition as stored in the database.
#[derive(Debug, Clone, Serialize)]
pub struct NoteType {
    pub id: i64,
    pub name: String,
    pub field_names: Vec<String>,
    pub templates: Vec<Template>,
}

/// A card template within a note type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    pub index: i64,
    pub name: String,
    pub front_pattern: String,
    pub back_pattern: String,
}

/// The fields of a note, as deserialised from JSON.
pub type NoteFields = serde_json::Map<String, serde_json::Value>;

/// The rendered output for a single card.
#[derive(Debug, Clone, Serialize)]
pub struct RenderedCard {
    /// The 0-based index of the template that generated this card.
    pub template_index: i64,
    /// The rendered front (question) text.
    pub front: String,
    /// The rendered back (answer) text.
    pub back: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Look up a note type by ID, including its templates.
pub async fn get_note_type(db: &SqlitePool, id: i64) -> Result<NoteType, String> {
    let row = sqlx::query!(
        "SELECT id, name, field_names, school_id FROM note_types WHERE id = ?",
        id
    )
    .fetch_optional(db)
    .await
    .map_err(|e| format!("Database error: {e}"))?
    .ok_or_else(|| format!("Note type {id} not found"))?;

    let field_names: Vec<String> = serde_json::from_str(&row.field_names)
        .map_err(|e| format!("Invalid field_names JSON: {e}"))?;

    let templates = sqlx::query!(
        "SELECT template_index, name, front_pattern, back_pattern FROM note_type_templates WHERE note_type_id = ? ORDER BY template_index",
        id
    )
    .fetch_all(db)
    .await
    .map_err(|e| format!("Database error: {e}"))?;

    let templates: Vec<Template> = templates
        .into_iter()
        .map(|t| Template {
            index: t.template_index,
            name: t.name,
            front_pattern: t.front_pattern,
            back_pattern: t.back_pattern,
        })
        .collect();

    Ok(NoteType {
        id: row.id,
        name: row.name,
        field_names,
        templates,
    })
}

/// Look up a note type by name (for legacy compatibility).
pub async fn get_note_type_by_name(db: &SqlitePool, name: &str) -> Result<NoteType, String> {
    let row = sqlx::query!("SELECT id FROM note_types WHERE name = ?", name)
        .fetch_optional(db)
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or_else(|| format!("Note type '{name}' not found"))?;

    get_note_type(db, row.id.expect("note_type.id is NOT NULL")).await
}

/// List all note types in a school.
pub async fn list_note_types(db: &SqlitePool, school_id: i64) -> Result<Vec<NoteType>, String> {
    let rows = sqlx::query!(
        "SELECT id, name, field_names FROM note_types WHERE school_id = ? ORDER BY name",
        school_id
    )
    .fetch_all(db)
    .await
    .map_err(|e| format!("Database error: {e}"))?;

    let mut result = Vec::new();
    for row in rows {
        let nt = get_note_type(db, row.id.expect("id is NOT NULL")).await?;
        result.push(nt);
    }
    Ok(result)
}

/// Validate that the given fields match the note type's requirements.
pub fn validate_fields(field_names: &[String], fields: &NoteFields) -> Result<(), String> {
    let missing: Vec<&String> = field_names
        .iter()
        .filter(|name| !fields.contains_key(name.as_str()))
        .collect();

    if missing.is_empty() {
        Ok(())
    } else {
        let names: Vec<String> = missing.iter().map(|n| n.to_string()).collect();
        Err(format!("Missing required fields: {}", names.join(", ")))
    }
}

/// Render all cards for a note by applying each template to the fields.
pub fn render_cards(templates: &[Template], fields: &NoteFields) -> Vec<RenderedCard> {
    templates
        .iter()
        .map(|template| {
            let front = render_template(&template.front_pattern, fields);
            let back = render_template(&template.back_pattern, fields);
            RenderedCard {
                template_index: template.index,
                front,
                back,
            }
        })
        .collect()
}

/// Render a single card given a note type and note fields.
pub fn render_card(
    templates: &[Template],
    template_index: i64,
    fields: &NoteFields,
) -> Option<RenderedCard> {
    templates
        .iter()
        .find(|t| t.index == template_index)
        .map(|template| RenderedCard {
            template_index,
            front: render_template(&template.front_pattern, fields),
            back: render_template(&template.back_pattern, fields),
        })
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Replace `{{FieldName}}` placeholders in a template string.
fn render_template(template: &str, fields: &NoteFields) -> String {
    let mut result = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' && chars.peek() == Some(&'{') {
            chars.next();
            let mut placeholder = String::new();
            loop {
                match chars.next() {
                    Some('}') if chars.peek() == Some(&'}') => {
                        chars.next();
                        break;
                    }
                    Some(c) => placeholder.push(c),
                    None => {
                        result.push_str("{{");
                        result.push_str(&placeholder);
                        break;
                    }
                }
            }
            let value = fields
                .get(placeholder.trim())
                .and_then(|v| v.as_str())
                .unwrap_or(&placeholder);
            result.push_str(value);
        } else {
            result.push(ch);
        }
    }

    result
}
