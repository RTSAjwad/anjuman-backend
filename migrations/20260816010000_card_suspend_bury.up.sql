ALTER TABLE student_card_states ADD COLUMN suspended INTEGER NOT NULL DEFAULT 0;
ALTER TABLE student_card_states ADD COLUMN buried_until INTEGER;
