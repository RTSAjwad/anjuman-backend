-- Token revocation (blocklist).
--
-- JWTs are stateless, so "logout" requires the server to remember which tokens
-- have been explicitly revoked. This table stores revoked token IDs until they
-- naturally expire, at which point they can be cleaned up.

CREATE TABLE revoked_tokens (
    -- The JWT ID (`jti` claim) that uniquely identifies a token.
    jti TEXT NOT NULL PRIMARY KEY,

    -- The user who revoked this token (i.e. the user who logged out).
    user_id INTEGER NOT NULL,

    -- Unix timestamp (seconds) when this JWT naturally expires.
    -- After this time the entry can be cleaned up since the token
    -- would be rejected for expiration anyway.
    expires_at INTEGER NOT NULL,

    -- When the revocation happened.
    revoked_at INTEGER NOT NULL DEFAULT (unixepoch()),

    FOREIGN KEY (user_id)
        REFERENCES users(id)
        ON DELETE CASCADE
);

-- Helps the periodic cleanup job find expired entries quickly.
CREATE INDEX idx_revoked_tokens_expires
ON revoked_tokens(expires_at);
