// Route definitions.

use axum::{
    Router,
    routing::{delete, get, patch, post},
};

use crate::{
    handlers::{
        admin_users, analytics, card_browser, card_mod, classes, dashboard, deck_options_handler,
        decks, health, login, logout, me, note_types_handler, notes, reviews, search_users, study,
        users,
    },
    state::AppState,
};

pub fn router(state: AppState) -> Router {
    Router::new()
        // Public
        .route("/health", get(health::health))
        // Auth
        .route("/auth/login", post(login::login))
        .route("/auth/logout", post(logout::logout))
        // Users
        .route("/users/search", get(search_users::search_users))
        .route("/users", get(admin_users::list_users))
        .route("/users", post(users::create_user))
        .route("/users/{id}", get(admin_users::get_user))
        .route("/users/{id}", patch(admin_users::update_user))
        .route("/users/{id}", delete(admin_users::delete_user))
        .route("/me", get(me::me))
        // Classes
        .route("/classes", get(classes::list_classes))
        .route("/classes", post(classes::create_class))
        .route("/classes/{id}", get(classes::get_class))
        .route("/classes/{id}/rename", patch(classes::rename_class))
        .route("/classes/{id}/archive", post(classes::archive_class))
        .route("/classes/{id}", delete(classes::delete_class))
        .route("/classes/{id}/roster", get(classes::view_roster))
        .route("/classes/{id}/members", post(classes::add_member))
        .route(
            "/classes/{id}/members/{user_id}",
            delete(classes::remove_member),
        )
        // Study
        .route("/decks/{id}/study", get(study::deck_study))
        // Card browser
        .route("/cards", get(card_browser::browse_cards))
        // Reviews
        .route("/reviews", post(reviews::submit_review))
        .route("/cards/{card_id}/flag", patch(reviews::set_flag))
        // Card modification (suspend / bury / reschedule)
        .route("/cards/{card_id}/suspend", post(card_mod::suspend))
        .route("/cards/{card_id}/unsuspend", post(card_mod::unsuspend))
        .route("/cards/{card_id}/bury", post(card_mod::bury))
        .route("/cards/{card_id}/unbury", post(card_mod::unbury))
        .route("/cards/{card_id}/reschedule", patch(card_mod::reschedule))
        .route("/notes/{note_id}/bury", post(card_mod::bury_note))
        .route("/notes/{note_id}/suspend", post(card_mod::suspend_note))
        // Move a card to another deck
        .route("/cards/{card_id}/deck", patch(card_mod::move_card))
        // Deck options (presets)
        .route(
            "/deck-options",
            get(deck_options_handler::list_deck_options),
        )
        .route(
            "/deck-options",
            post(deck_options_handler::create_deck_options),
        )
        .route(
            "/deck-options/{id}",
            get(deck_options_handler::get_deck_options),
        )
        .route(
            "/deck-options/{id}",
            patch(deck_options_handler::update_deck_options),
        )
        .route(
            "/deck-options/{id}",
            delete(deck_options_handler::delete_deck_options),
        )
        // Analytics
        // Dashboard
        .route("/dashboard", get(dashboard::dashboard))
        .route("/analytics/me", get(analytics::my_stats))
        .route("/analytics/me/daily", get(analytics::my_daily))
        .route("/analytics/classes/{id}", get(analytics::class_analytics))
        .route(
            "/analytics/classes/{id}/students/{student_id}",
            get(analytics::student_detail),
        )
        // Decks
        .route("/decks", get(decks::list_decks))
        .route("/decks", post(decks::create_deck))
        .route("/decks/{id}", get(decks::get_deck))
        .route("/decks/{id}/rename", patch(decks::rename_deck))
        .route("/decks/{id}", delete(decks::delete_deck))
        .route("/decks/{id}/duplicate", post(decks::duplicate_deck))
        .route("/decks/{id}/share", post(decks::share_deck))
        .route("/decks/{id}/share/{user_id}", delete(decks::unshare_deck))
        .route("/decks/{id}/owner", patch(decks::transfer_owner))
        .route("/decks/{id}/classes", post(decks::add_deck_to_class))
        .route("/decks/{id}/classes", get(decks::list_deck_classes))
        .route(
            "/decks/{id}/classes/{class_id}",
            delete(decks::remove_deck_from_class),
        )
        // Note Types
        .route("/note-types", get(note_types_handler::list_note_types))
        .route("/note-types", post(note_types_handler::create_note_type))
        .route("/note-types/{id}", get(note_types_handler::get_note_type))
        .route(
            "/note-types/{id}",
            patch(note_types_handler::update_note_type),
        )
        .route(
            "/note-types/{id}",
            delete(note_types_handler::delete_note_type),
        )
        // Notes (note-centric)
        .route("/notes", post(notes::create_note))
        .route("/notes", get(notes::list_notes))
        .route("/notes/{note_id}", get(notes::get_note))
        .route("/notes/{note_id}", patch(notes::update_note))
        .route("/notes/{note_id}", delete(notes::delete_note))
        .with_state(state)
}
