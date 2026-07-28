DROP INDEX IF EXISTS idx_decks_parent;
ALTER TABLE decks DROP COLUMN parent_id;
