-- Custom note types and display-time card rendering.
--
-- Already applied to the dev database. For production, this migration:
--   1. Creates note_types + note_type_templates tables
--   2. Seeds default types (Basic, Basic and reversed)
--   3. Adds note_type_id to notes, migrates string -> id
--   4. Rebuilds notes table without note_type column
--   5. Rebuilds cards table without front/back columns
--
-- Since the dev DB is already migrated, this is a no-op placeholder.
-- See migrations/20260625232405_create_schema.sql for the current schema.

SELECT 1;
