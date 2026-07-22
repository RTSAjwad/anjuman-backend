-- Add migration script here
PRAGMA foreign_keys = ON;

--------------------------------------------------------------------
-- Schools
--------------------------------------------------------------------

CREATE TABLE schools (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

--------------------------------------------------------------------
-- Users
--------------------------------------------------------------------

CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    school_id INTEGER NOT NULL,

    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,

    role TEXT NOT NULL CHECK (
        role IN ('admin', 'teacher', 'student')
    ),

    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (school_id)
        REFERENCES schools(id)
        ON DELETE CASCADE
);

CREATE INDEX idx_users_school
ON users(school_id);

--------------------------------------------------------------------
-- Classes
--------------------------------------------------------------------

CREATE TABLE classes (
    id INTEGER PRIMARY KEY,
    school_id INTEGER NOT NULL,

    name TEXT NOT NULL,
    description TEXT,

    created_by INTEGER NOT NULL,

    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (school_id)
        REFERENCES schools(id)
        ON DELETE CASCADE,

    FOREIGN KEY (created_by)
        REFERENCES users(id)
);

CREATE INDEX idx_classes_school
ON classes(school_id);

--------------------------------------------------------------------
-- Class membership
--------------------------------------------------------------------

CREATE TABLE class_members (
    class_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,

    role TEXT NOT NULL CHECK (
        role IN ('teacher', 'student')
    ),

    joined_at INTEGER NOT NULL,

    PRIMARY KEY (class_id, user_id),

    FOREIGN KEY (class_id)
        REFERENCES classes(id)
        ON DELETE CASCADE,

    FOREIGN KEY (user_id)
        REFERENCES users(id)
        ON DELETE CASCADE
);

CREATE INDEX idx_class_members_user
ON class_members(user_id);

--------------------------------------------------------------------
-- Decks
--------------------------------------------------------------------

CREATE TABLE decks (
    id INTEGER PRIMARY KEY,

    school_id INTEGER NOT NULL,

    title TEXT NOT NULL,
    description TEXT,

    created_by INTEGER NOT NULL,

    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (school_id)
        REFERENCES schools(id)
        ON DELETE CASCADE,

    FOREIGN KEY (created_by)
        REFERENCES users(id)
);

CREATE INDEX idx_decks_school
ON decks(school_id);

--------------------------------------------------------------------
-- Notes
--------------------------------------------------------------------

CREATE TABLE notes (
    id INTEGER PRIMARY KEY,

    deck_id INTEGER NOT NULL,

    note_type TEXT NOT NULL,

    fields_json TEXT NOT NULL,

    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (deck_id)
        REFERENCES decks(id)
        ON DELETE CASCADE
);

CREATE INDEX idx_notes_deck
ON notes(deck_id);

--------------------------------------------------------------------
-- Cards
--------------------------------------------------------------------

CREATE TABLE cards (
    id INTEGER PRIMARY KEY,

    note_id INTEGER NOT NULL,

    template_index INTEGER NOT NULL,

    front TEXT NOT NULL,
    back TEXT NOT NULL,

    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (note_id)
        REFERENCES notes(id)
        ON DELETE CASCADE
);

CREATE UNIQUE INDEX idx_cards_note_template
ON cards(note_id, template_index);

--------------------------------------------------------------------
-- Student scheduling state (FSRS)
--------------------------------------------------------------------

CREATE TABLE student_card_states (
    student_id INTEGER NOT NULL,
    card_id INTEGER NOT NULL,

    state TEXT NOT NULL CHECK (
        state IN (
            'new',
            'learning',
            'review',
            'relearning'
        )
    ),

    stability REAL NOT NULL,
    difficulty REAL NOT NULL,

    due_at INTEGER,
    last_reviewed_at INTEGER,

    reps INTEGER NOT NULL DEFAULT 0,
    lapses INTEGER NOT NULL DEFAULT 0,

    PRIMARY KEY (student_id, card_id),

    FOREIGN KEY (student_id)
        REFERENCES users(id)
        ON DELETE CASCADE,

    FOREIGN KEY (card_id)
        REFERENCES cards(id)
        ON DELETE CASCADE
);

CREATE INDEX idx_due_cards
ON student_card_states(student_id, due_at);

--------------------------------------------------------------------
-- Review history
--------------------------------------------------------------------

CREATE TABLE reviews (
    id INTEGER PRIMARY KEY,

    student_id INTEGER NOT NULL,
    card_id INTEGER NOT NULL,

    rating INTEGER NOT NULL CHECK (
        rating BETWEEN 1 AND 4
    ),

    reviewed_at INTEGER NOT NULL,

    response_time_ms INTEGER,

    FOREIGN KEY (student_id)
        REFERENCES users(id)
        ON DELETE CASCADE,

    FOREIGN KEY (card_id)
        REFERENCES cards(id)
        ON DELETE CASCADE
);

CREATE INDEX idx_reviews_student
ON reviews(student_id);

CREATE INDEX idx_reviews_card
ON reviews(card_id);

CREATE INDEX idx_reviews_time
ON reviews(reviewed_at);

--------------------------------------------------------------------
-- Assignments
--------------------------------------------------------------------

CREATE TABLE assignments (
    id INTEGER PRIMARY KEY,

    class_id INTEGER NOT NULL,
    deck_id INTEGER NOT NULL,

    title TEXT NOT NULL,

    due_at INTEGER,

    created_by INTEGER NOT NULL,

    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (class_id)
        REFERENCES classes(id)
        ON DELETE CASCADE,

    FOREIGN KEY (deck_id)
        REFERENCES decks(id)
        ON DELETE CASCADE,

    FOREIGN KEY (created_by)
        REFERENCES users(id)
);

CREATE INDEX idx_assignments_class
ON assignments(class_id);

--------------------------------------------------------------------
-- Assignment membership
--------------------------------------------------------------------

CREATE TABLE assignment_members (
    assignment_id INTEGER NOT NULL,
    student_id INTEGER NOT NULL,

    assigned_at INTEGER NOT NULL,

    completed_at INTEGER,

    PRIMARY KEY (assignment_id, student_id),

    FOREIGN KEY (assignment_id)
        REFERENCES assignments(id)
        ON DELETE CASCADE,

    FOREIGN KEY (student_id)
        REFERENCES users(id)
        ON DELETE CASCADE
);

CREATE INDEX idx_assignment_members_student
ON assignment_members(student_id);

--------------------------------------------------------------------
-- Study sessions
--------------------------------------------------------------------

CREATE TABLE study_sessions (
    id INTEGER PRIMARY KEY,

    student_id INTEGER NOT NULL,

    started_at INTEGER NOT NULL,
    ended_at INTEGER,

    cards_reviewed INTEGER NOT NULL DEFAULT 0,

    FOREIGN KEY (student_id)
        REFERENCES users(id)
        ON DELETE CASCADE
);

CREATE INDEX idx_sessions_student
ON study_sessions(student_id);
