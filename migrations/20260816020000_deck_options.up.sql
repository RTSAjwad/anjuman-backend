CREATE TABLE deck_options (
    id INTEGER PRIMARY KEY,
    school_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    -- Scheduling steps (seconds, comma-separated)
    learning_steps   TEXT NOT NULL DEFAULT '60,600',
    relearning_steps TEXT NOT NULL DEFAULT '600',
    desired_retention REAL NOT NULL DEFAULT 0.9,
    -- Bury sibling toggles (scheduled for later; stored now)
    bury_new        INTEGER NOT NULL DEFAULT 0,
    bury_review     INTEGER NOT NULL DEFAULT 0,
    bury_interday   INTEGER NOT NULL DEFAULT 0,
    -- Daily limits (stored for later enforcement)
    new_per_day     INTEGER NOT NULL DEFAULT 20,
    review_per_day  INTEGER NOT NULL DEFAULT 200,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    FOREIGN KEY (school_id) REFERENCES schools(id) ON DELETE CASCADE,
    UNIQUE(school_id, name)
);

ALTER TABLE decks ADD COLUMN options_id INTEGER REFERENCES deck_options(id) ON DELETE SET NULL;
