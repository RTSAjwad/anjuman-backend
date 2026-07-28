-- Seed data for development/testing.
-- All passwords are hashed with Argon2id and are VALID.
-- Credentials:
--   teacher@school1.com / teach123
--   admin@school1.com   / admin123
--   student@school1.com / stud123

--------------------------------------------------------------------
-- School
--------------------------------------------------------------------

INSERT OR IGNORE INTO schools (id, name) VALUES (1, 'Springfield High');

--------------------------------------------------------------------
-- Users
--------------------------------------------------------------------

INSERT OR IGNORE INTO users (id, school_id, email, password_hash, role, first_name, last_name, created_at)
VALUES
    (1, 1, 'teacher@school1.com', '$argon2id$v=19$m=19456,t=2,p=1$7ArQkDZAbK6WWtSMDD3swg$KzG8ucoLFw86POIGm9cZotQJiG/vM+R+Drsz19SgAvo', 'teacher', 'Alice', 'Johnson', '1700000000'),
    (2, 1, 'admin@school1.com',   '$argon2id$v=19$m=19456,t=2,p=1$FtKF9MeZcvxkgd1AnsG35Q$5mh2ZTwlqufppZ8XXJwjnMvvMW45/1EOq0l/iy8Kuaw', 'admin',   'Bob',   'Williams', '1700000001'),
    (3, 1, 'student@school1.com', '$argon2id$v=19$m=19456,t=2,p=1$Q2CRjpZ1fVPtBf5SUR8Klg$e5XdI09f5EKMj4GMGCojXrWirexuuA2nwiI1QGPaNUQ', 'student', 'Charlie', 'Smith', '1700000002');

--------------------------------------------------------------------
-- Class
--------------------------------------------------------------------

INSERT OR IGNORE INTO classes (id, school_id, name, description, created_by, created_at)
VALUES (1, 1, 'Biology 101', 'Introduction to biology', 1, '1700000000');

INSERT OR IGNORE INTO class_members (class_id, user_id, role, joined_at)
VALUES (1, 3, 'student', '1700000000');

--------------------------------------------------------------------
-- Note types (Basic)
--------------------------------------------------------------------

INSERT OR IGNORE INTO note_types (id, school_id, name, field_names, sort_field, created_by, created_at)
VALUES (1, 1, 'Basic', '["Front","Back"]', 'Front', 1, '1700000000');

INSERT OR IGNORE INTO note_type_templates (note_type_id, template_index, name, front_pattern, back_pattern)
VALUES (1, 0, 'Card 1', '{{Front}}', '{{Front}}\n\n<hr>\n\n{{Back}}');

--------------------------------------------------------------------
-- Deck
--------------------------------------------------------------------

INSERT OR IGNORE INTO decks (id, school_id, title, description, created_by, parent_id, created_at)
VALUES (1, 1, 'Biology 101', 'Core biology deck', 1, NULL, '1700000000');

INSERT OR IGNORE INTO deck_classes (deck_id, class_id, added_at)
VALUES (1, 1, '1700000000');

--------------------------------------------------------------------
-- Notes & Cards (10 basic biology questions)
--------------------------------------------------------------------

INSERT OR IGNORE INTO notes (id, note_type_id, fields_json, created_at)
VALUES
    (1,  1, '{"Front":"What is the powerhouse of the cell?","Back":"Mitochondria"}',                     '1700000100'),
    (2,  1, '{"Front":"What is the process by which plants make food?","Back":"Photosynthesis"}',         '1700000101'),
    (3,  1, '{"Front":"What gas do plants absorb from the atmosphere?","Back":"Carbon dioxide"}',          '1700000102'),
    (4,  1, '{"Front":"What is the basic unit of life?","Back":"The cell"}',                              '1700000103'),
    (5,  1, '{"Front":"What organelle contains genetic material?","Back":"Nucleus"}',                      '1700000104'),
    (6,  1, '{"Front":"What is the jelly-like substance inside a cell?","Back":"Cytoplasm"}',             '1700000105'),
    (7,  1, '{"Front":"Which organelle produces proteins?","Back":"Ribosomes"}',                           '1700000106'),
    (8,  1, '{"Front":"What is the cell membrane made of?","Back":"Phospholipid bilayer"}',               '1700000107'),
    (9,  1, '{"Front":"What molecule carries genetic information?","Back":"DNA"}',                         '1700000108'),
    (10, 1, '{"Front":"What organelle is responsible for packaging proteins?","Back":"Golgi apparatus"}',  '1700000109');

INSERT OR IGNORE INTO cards (id, note_id, deck_id, template_index, created_at)
VALUES
    (1,  1,  1, 0, '1700000100'),
    (2,  2,  1, 0, '1700000101'),
    (3,  3,  1, 0, '1700000102'),
    (4,  4,  1, 0, '1700000103'),
    (5,  5,  1, 0, '1700000104'),
    (6,  6,  1, 0, '1700000105'),
    (7,  7,  1, 0, '1700000106'),
    (8,  8,  1, 0, '1700000107'),
    (9,  9,  1, 0, '1700000108'),
    (10, 10, 1, 0, '1700000109');
