# Anki Classroom — Backend Architecture

A Rust/Axum backend for a classroom-focused spaced repetition learning platform. Uses SQLite via sqlx, JWT authentication, Argon2 password hashing, and the FSRS-rs scheduling algorithm.

---

## Project Structure

```
src/
├── main.rs             # Entry point, server startup, background tasks
├── app.rs              # Router construction wrapper
├── routes.rs           # All route definitions (46 endpoints)
├── state.rs            # Shared AppState (DB pool)
├── db.rs               # SQLite connection pool setup
├── note_types.rs       # Note type definitions and card template rendering
├── auth/
│   ├── mod.rs          # Re-exports
│   ├── jwt.rs          # JWT creation, verification, and revocation
│   └── middleware.rs   # AuthUser extractor (protects routes)
└── handlers/
    ├── health.rs       # GET /health — liveness check
    ├── login.rs        # POST /auth/login — JWT issuance
    ├── logout.rs       # POST /auth/logout — token revocation
    ├── me.rs           # GET /me — current user profile
    ├── users.rs        # POST /users — registration, password hashing
    ├── admin_users.rs  # User CRUD (admin-only)
    ├── search_users.rs # GET /users/search — typeahead search
    ├── classes.rs      # Class CRUD, roster management
    ├── decks.rs        # Deck CRUD, sharing, ownership, class assignment
    ├── notes.rs        # Note CRUD, card auto-generation
    ├── study.rs        # GET /decks/{id}/study — Anki-style study queue
    ├── reviews.rs      # POST /reviews — FSRS-based card review submission
    ├── analytics.rs    # Student and class analytics
    └── dashboard.rs    # GET /dashboard — teacher overview

migrations/
├── 20260625232405_create_schema.sql   # All core tables
├── 20260711000000_revoked_tokens.sql  # JWT revocation table
├── 20260713000000_user_names.sql      # first_name, last_name columns
└── 20260714000000_deck_classes.sql    # Deck-to-class visibility

api.yaml              # OpenAPI 3.0 specification
platform.db           # SQLite development database
Cargo.toml            # Rust dependencies
```

---

## Core Infrastructure

### `src/main.rs`

- Loads `.env`, initialises tracing, connects to SQLite
- Runs sqlx migrations at startup
- Spawns a background task that cleans up expired revoked tokens every hour
- Starts the Axum server on `127.0.0.1:3000`

### `src/state.rs`

`AppState` holds the `SqlitePool` (cloneable, backed by `Arc`). Injected into every handler via Axum's state mechanism.

### `src/db.rs`

Creates a SQLite connection pool (5 connections max) using `DATABASE_URL` from the environment.

### `src/app.rs`

A thin wrapper that calls `routes::router(state)` — keeps `main.rs` clean.

### `src/routes.rs`

All 48 endpoints defined as a flat `Router` chain. No nesting — every route is explicit.

---

## Authentication & Authorisation

### `src/auth/jwt.rs`

JWT creation and verification:
- Tokens signed with HMAC-SHA256 using `JWT_SECRET` env var
- 24-hour expiry
- Claims: `jti` (UUID for revocation), `sub` (user ID), `school_id`, `role`, `iat`, `exp`
- `create_token()` — builds and signs a JWT
- `verify_token()` — validates signature, expiry, and checks revocation table
- `revoke_token()` — inserts `jti` into `revoked_tokens` for logout
- `is_revoked()` — checks if a token's `jti` is in the revocation table

### `src/auth/middleware.rs`

`AuthUser` — an Axum extractor implementing `FromRequestParts`:
1. Extracts `Authorization: Bearer <token>` header
2. Verifies JWT signature and expiry
3. Checks revocation table
4. Returns decoded `Claims` to the handler

Any handler can protect itself by adding `AuthUser(claims): AuthUser` as a parameter.

### Password Security

Argon2id hashing in `handlers/users.rs`:
- `hash_password()` — generates random salt, hashes with default params (19 MiB memory, 2 iterations)
- `verify_password()` — constant-time comparison against stored hash

### Role-Based Access

Three roles defined in `UserRole` enum: `Admin`, `Teacher`, `Student`.

Permission helpers scattered across handlers:
- `check_admin()` — admin-only gate
- `check_teacher_or_admin()` — blocks students
- `check_class_owner()` — allows admins (any class in school) + class creator
- `check_class_member()` — allows admins + creator + any class member
- `check_deck_owner()` — allows admins + deck creator
- `check_deck_collaborator()` — allows admins + creator + collaborators
- `check_deck_visible()` — allows admins + creator + collaborators + class members

---

## Database Schema

### `schools`
| Column | Type | Notes |
|--------|------|-------|
| id | INTEGER PK | |
| name | TEXT NOT NULL | |
| created_at | TEXT | DEFAULT CURRENT_TIMESTAMP |

### `users`
| Column | Type | Notes |
|--------|------|-------|
| id | INTEGER PK | |
| school_id | INTEGER FK→schools | School-scoped |
| email | TEXT UNIQUE NOT NULL | Login identifier |
| password_hash | TEXT NOT NULL | Argon2id |
| first_name | TEXT NOT NULL | |
| last_name | TEXT NOT NULL | |
| role | TEXT NOT NULL | CHECK (admin, teacher, student) |
| created_at | TEXT | |

### `classes`
| Column | Type | Notes |
|--------|------|-------|
| id | INTEGER PK | |
| school_id | INTEGER FK | |
| name | TEXT NOT NULL | |
| description | TEXT | |
| archived | INTEGER | 0=active, 1=archived |
| created_by | INTEGER FK→users | |
| created_at | TEXT | |

### `class_members`
| Column | Type | Notes |
|--------|------|-------|
| class_id | INTEGER FK | |
| user_id | INTEGER FK | |
| role | TEXT | CHECK (teacher, student) |
| joined_at | INTEGER | Unix timestamp |
| PRIMARY KEY | (class_id, user_id) | |

### `decks`
| Column | Type | Notes |
|--------|------|-------|
| id | INTEGER PK | |
| school_id | INTEGER FK | |
| title | TEXT NOT NULL | |
| description | TEXT | |
| created_by | INTEGER FK→users | Deck owner |
| created_at | TEXT | |

### `deck_collaborators`
| Column | Type | Notes |
|--------|------|-------|
| deck_id | INTEGER FK | |
| user_id | INTEGER FK | |
| shared_at | INTEGER | |
| PRIMARY KEY | (deck_id, user_id) | |

### `deck_classes`
| Column | Type | Notes |
|--------|------|-------|
| deck_id | INTEGER FK | |
| class_id | INTEGER FK | |
| added_at | INTEGER | |
| PRIMARY KEY | (deck_id, class_id) | |

### `notes`
| Column | Type | Notes |
|--------|------|-------|
| id | INTEGER PK | |
| deck_id | INTEGER FK→decks | CASCADE delete |
| note_type | TEXT NOT NULL | e.g. "Basic" |
| fields_json | TEXT NOT NULL | JSON map of field values |
| created_at | TEXT | |

### `cards`
| Column | Type | Notes |
|--------|------|-------|
| id | INTEGER PK | |
| note_id | INTEGER FK→notes | CASCADE delete |
| template_index | INTEGER | Which template generated this card |
| front | TEXT NOT NULL | Rendered question |
| back | TEXT NOT NULL | Rendered answer |
| created_at | TEXT | |
| UNIQUE | (note_id, template_index) | |

### `student_card_states`
| Column | Type | Notes |
|--------|------|-------|
| student_id | INTEGER FK | |
| card_id | INTEGER FK | CASCADE delete |
| state | TEXT | CHECK (new, learning, review, relearning) |
| stability | REAL | FSRS parameter |
| difficulty | REAL | FSRS parameter |
| due_at | INTEGER | Unix timestamp, NULL for new cards |
| last_reviewed_at | INTEGER | |
| reps | INTEGER | Total review count |
| lapses | INTEGER | "Again" count |
| PRIMARY KEY | (student_id, card_id) | |

### `reviews`
| Column | Type | Notes |
|--------|------|-------|
| id | INTEGER PK | |
| student_id | INTEGER FK | |
| card_id | INTEGER FK | |
| rating | INTEGER | CHECK (1-4) |
| reviewed_at | INTEGER | |
| response_time_ms | INTEGER | Optional |

### `revoked_tokens`
| Column | Type | Notes |
|--------|------|-------|
| jti | TEXT PK | JWT ID |
| user_id | INTEGER | |
| expires_at | INTEGER | When the token naturally expires |

---

## Handler Reference

### Health

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/health` | None | Returns `{"status": "ok"}` |

### Auth

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/auth/login` | None | Email + password → JWT + user info |
| POST | `/auth/logout` | Required | Revokes current JWT |

### Users

| Method | Path | Auth | Role | Description |
|--------|------|------|------|-------------|
| GET | `/users` | Required | Admin | List all users in school |
| POST | `/users` | None | — | Register new user |
| GET | `/users/search?q=` | Required | Teacher/Admin | Search users by name/email (excludes admins, limit 20) |
| GET | `/users/{id}` | Required | Admin | Get user details |
| PATCH | `/users/{id}` | Required | Admin | Update email, password, role, names |
| DELETE | `/users/{id}` | Required | Admin | Delete user (cascade) |
| GET | `/me` | Required | Any | Current user profile |

### Classes

| Method | Path | Auth | Role | Description |
|--------|------|------|------|-------------|
| GET | `/classes` | Required | Any | List classes (admin: all; teacher/student: own + member) |
| POST | `/classes` | Required | Teacher/Admin | Create class |
| GET | `/classes/{id}` | Required | Any | Get class (must be member/owner/admin) |
| PATCH | `/classes/{id}/rename` | Required | Teacher/Admin | Rename class (must be member) |
| POST | `/classes/{id}/archive` | Required | Teacher/Admin | Toggle archive (must be member) |
| DELETE | `/classes/{id}` | Required | Teacher/Admin | Delete class (owner/admin only) |
| GET | `/classes/{id}/roster` | Required | Any | View members (must be member) |
| POST | `/classes/{id}/members` | Required | Teacher/Admin | Add user to class (must be member) |
| DELETE | `/classes/{id}/members/{user_id}` | Required | Teacher/Admin | Remove from class (must be member) |

### Decks

| Method | Path | Auth | Role | Description |
|--------|------|------|------|-------------|
| GET | `/decks` | Required | Any | List decks (role-filtered; students get card counts) |
| POST | `/decks` | Required | Teacher/Admin | Create deck |
| GET | `/decks/{id}` | Required | Any | Get deck + collaborators + classes |
| PATCH | `/decks/{id}/rename` | Required | Teacher/Admin | Rename (owner/collaborator/admin) |
| DELETE | `/decks/{id}` | Required | Teacher/Admin | Delete (owner/admin only) |
| POST | `/decks/{id}/duplicate` | Required | Teacher/Admin | Copy deck, notes, and cards (transactional) |
| POST | `/decks/{id}/share` | Required | Teacher/Admin | Share with teacher (owner/admin only) |
| DELETE | `/decks/{id}/share/{user_id}` | Required | Teacher/Admin | Remove collaborator (owner/admin only) |
| PATCH | `/decks/{id}/owner` | Required | Teacher/Admin | Transfer ownership (owner/admin only; old owner becomes collaborator) |
| POST | `/decks/{id}/classes` | Required | Teacher/Admin | Add deck to class (owner/admin only) |
| GET | `/decks/{id}/classes` | Required | Any | List classes deck is assigned to |
| DELETE | `/decks/{id}/classes/{class_id}` | Required | Teacher/Admin | Remove from class (owner/admin only) |

### Notes

| Method | Path | Auth | Role | Description |
|--------|------|------|------|-------------|
| POST | `/decks/{deck_id}/notes` | Required | Teacher/Admin | Create note (owner/collaborator/admin) |
| GET | `/decks/{deck_id}/notes` | Required | Teacher/Admin | List notes |
| GET | `/decks/{deck_id}/notes/{note_id}` | Required | Teacher/Admin | Get note with cards |
| PATCH | `/decks/{deck_id}/notes/{note_id}` | Required | Teacher/Admin | Update note, re-renders cards |
| DELETE | `/decks/{deck_id}/notes/{note_id}` | Required | Teacher/Admin | Delete note and cards (cascade) |

### Study

| Method | Path | Auth | Role | Description |
|--------|------|------|------|-------------|
| GET | `/decks/{id}/study` | Required | Any | Get due cards for a deck (owner/admin/collaborator/class member) |

Returns cards with FSRS state (`new`, `learning`, `review`, `relearning`) and `predicted_interval` map for each rating. Cards ordered: new first, then overdue. Client should loop: fetch cards → show one → submit review → re-fetch until `total_cards: 0`.

### Reviews

| Method | Path | Auth | Role | Description |
|--------|------|------|------|-------------|
| POST | `/reviews` | Required | Any | Submit rating (1=Again, 2=Hard, 3=Good, 4=Easy) |

Rating 1 sets `due_at = now` (re-queues the card). Ratings 2-4 schedule the card per FSRS. State machine: `new → learning → review`. `relearning` (from rating 1) graduates to `review` on rating 2-4.

### Analytics

| Method | Path | Auth | Role | Description |
|--------|------|------|------|-------------|
| GET | `/analytics/me` | Required | Any | Personal stats (reviews, retention, cards mastered, streak) |
| GET | `/analytics/me/daily` | Required | Any | Daily breakdown (last 30 days) |
| GET | `/analytics/classes/{id}` | Required | Teacher/Admin | Class overview with per-student stats |
| GET | `/analytics/classes/{id}/students/{student_id}` | Required | Teacher/Admin | Student drill-down with daily data |

### Dashboard

| Method | Path | Auth | Role | Description |
|--------|------|------|------|-------------|
| GET | `/dashboard` | Required | Teacher/Admin | Aggregated overview: student count, reviews today, retention, class cards, attention alerts |

---

## Note Types & Card Rendering (`note_types.rs`)

### Supported Note Types

- **Basic**: 1 card (Front → Back)
- **Basic (and reversed)**: 2 cards (Front → Back + Back → Front)

### How It Works

1. A note type defines field names (e.g. `["Front", "Back"]`) and card templates with `{{FieldName}}` placeholders
2. When a note is created or updated, `validate_fields()` checks all required fields are present
3. `render_cards()` replaces placeholders with field values to produce front/back text
4. `sync_cards()` in `notes.rs` upserts the rendered cards into the `cards` table, deleting any excess templates (if note type changed to one with fewer templates)

---

## Deck Visibility Model

A deck is visible to a user if **any** of these are true:
1. User is an admin in the same school
2. User created the deck (owner)
3. User is a collaborator on the deck
4. User is a member of a class that has the deck assigned

### Student Deck List

When a student calls `GET /decks`, each deck includes per-state card counts:

| Field | Meaning |
|-------|---------|
| `new_count` | Cards never reviewed (`reps = 0`) |
| `learning_count` | Cards in learning state |
| `due_count` | Review/relearning cards with `due_at <= now` |
| `relearning_count` | Cards in relearning state |
| `total_count` | All cards in the deck |

These fields are omitted for teachers and admins.

---

## FSRS Scheduling (`reviews.rs`)

Uses the `fsrs-rs` crate (v6.6) to schedule each card review:

1. Fetches current `student_card_states` row
2. Calculates elapsed days since last review
3. Runs `FSRS::next_states()` with previous stability/difficulty (or `None` for new cards)
4. Selects the output state based on rating (1=again, 2=hard, 3=good, 4=easy)
5. Updates `student_card_states` with new stability, difficulty, due_at, state
6. Inserts a `reviews` row for analytics

### Card State Transitions

```
new ──(rating 2-4)──→ learning ──(rating 2-4)──→ review
  │                       │                         │
  └───(rating 1)─────→ relearning ←──(rating 1)─────┘
                            │
                     (rating 2-4)
                            │
                            ↓
                         review
```

### Again (Rating 1) Behaviour

- `due_at` is set to `now` (not the FSRS interval)
- The card re-appears immediately on the next study fetch
- This persists across devices/sessions — the card stays in the queue until a passing rating
- FSRS stability/difficulty are still saved for future scheduling

---

## Study Flow (Frontend Guide)

```
1. Student opens a deck → GET /decks/{id}/study
   → receives cards due now (new + cards where due_at <= now)
   → each card has predicted_interval for each rating

2. For each card:
   a. Show front text
   b. Student reveals back, selects rating (1-4)
   c. POST /reviews { card_id, rating, response_time_ms? }
   d. Move to next card
   e. Re-fetch GET /decks/{id}/study to get re-queued cards

3. Repeat until total_cards: 0 (queue empty)
```

---

## Key Design Decisions

- **No server-side published toggle**: Decks are made visible to students via class assignment (`deck_classes`). No global "publish to school" flag.
- **No assignments**: Students study decks directly. No due dates, no completion tracking. FSRS handles everything.
- **No explicit sessions**: Like Anki, session tracking is implicit. Analytics (time spent, study days, streaks) are derived from review timestamps.
- **Class-scoped membership**: A user's role (teacher/student) is global, not per-class. Adding a teacher to a class gives them management permissions for that class.
- **Deck duplication**: Copies the deck shell, all notes, and all cards in a single transaction. If it fails, nothing is left behind.
- **Deck ownership transfer**: Old owner becomes a collaborator automatically.
- **Collab sharing rules**: Cannot share with students, admins, or the deck owner.
- **Admins cannot be added to classes**: They already have access to everything.
- **Token revocation**: JWT logout inserts `jti` into a blocklist. Background task cleans up expired entries hourly.
- **Error responses are plain text**: Handlers return `(StatusCode, &str)` tuples. Axum serialises as `text/plain`.
