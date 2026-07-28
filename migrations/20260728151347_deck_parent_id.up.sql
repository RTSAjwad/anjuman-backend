ALTER TABLE decks ADD COLUMN parent_id INTEGER REFERENCES decks(id) ON DELETE CASCADE;
CREATE INDEX idx_decks_parent ON decks(parent_id);
