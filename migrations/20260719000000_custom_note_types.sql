-- Custom note types and display-time card rendering.

--------------------------------------------------------------------
-- Note types
--------------------------------------------------------------------

CREATE TABLE note_types (
    id INTEGER PRIMARY KEY,
    school_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    field_names TEXT NOT NULL,
    sort_field TEXT NOT NULL DEFAULT '',
    created_by INTEGER,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (school_id) REFERENCES schools(id) ON DELETE CASCADE,
    FOREIGN KEY (created_by) REFERENCES users(id),
    UNIQUE(school_id, name)
);

CREATE TABLE note_type_templates (
    note_type_id INTEGER NOT NULL,
    template_index INTEGER NOT NULL,
    name TEXT NOT NULL,
    front_pattern TEXT NOT NULL,
    back_pattern TEXT NOT NULL,
    PRIMARY KEY (note_type_id, template_index),
    FOREIGN KEY (note_type_id) REFERENCES note_types(id) ON DELETE CASCADE
);

--------------------------------------------------------------------
-- Migrate notes: note_type TEXT -> note_type_id INTEGER
--------------------------------------------------------------------

ALTER TABLE notes ADD COLUMN note_type_id INTEGER REFERENCES note_types(id);

-- Rebuild cards table without front/back columns, adding deck_id.
-- Cards are now rendered at display time from note type templates.
CREATE TABLE cards_new (
    id INTEGER PRIMARY KEY,
    note_id INTEGER NOT NULL,
    deck_id INTEGER NOT NULL,
    template_index INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE,
    FOREIGN KEY (deck_id) REFERENCES decks(id) ON DELETE CASCADE
);

INSERT INTO cards_new (id, note_id, deck_id, template_index, created_at)
SELECT c.id, c.note_id, n.deck_id, c.template_index, c.created_at
FROM cards c JOIN notes n ON n.id = c.note_id;

DROP TABLE cards;
ALTER TABLE cards_new RENAME TO cards;

CREATE UNIQUE INDEX idx_cards_note_template ON cards(note_id, template_index);
CREATE INDEX idx_cards_deck ON cards(deck_id);

-- Rebuild notes table without deck_id and note_type columns.
CREATE TABLE notes_new (
    id INTEGER PRIMARY KEY,
    note_type_id INTEGER NOT NULL,
    fields_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (note_type_id) REFERENCES note_types(id)
);

INSERT INTO notes_new (id, note_type_id, fields_json, created_at)
SELECT id, note_type_id, fields_json, created_at FROM notes;

DROP TABLE notes;
ALTER TABLE notes_new RENAME TO notes;
