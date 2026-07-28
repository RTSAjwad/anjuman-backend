-- Deck-to-class visibility: a deck can be made available to one or more classes.
CREATE TABLE IF NOT EXISTS deck_classes (
    deck_id INTEGER NOT NULL,
    class_id INTEGER NOT NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (deck_id, class_id),
    FOREIGN KEY (deck_id) REFERENCES decks(id) ON DELETE CASCADE,
    FOREIGN KEY (class_id) REFERENCES classes(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_deck_classes_class ON deck_classes(class_id);

-- Deck collaborators: share a deck with another teacher.
CREATE TABLE IF NOT EXISTS deck_collaborators (
    deck_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    shared_at INTEGER NOT NULL,
    PRIMARY KEY (deck_id, user_id),
    FOREIGN KEY (deck_id) REFERENCES decks(id) ON DELETE CASCADE,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_deck_collaborators_user ON deck_collaborators(user_id);
