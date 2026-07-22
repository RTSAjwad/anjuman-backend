// anki_classroom — main entry point

mod app;
mod auth;
mod db;
mod handlers {
    pub mod admin_users;
    pub mod analytics;
    pub mod card_browser;

    pub mod classes;
    pub mod dashboard;
    pub mod decks;
    pub mod health;
    pub mod login;
    pub mod logout;
    pub mod me;
    pub mod note_types_handler;
    pub mod notes;
    pub mod reviews;
    pub mod search_users;
    pub mod study;
    pub mod users;
}
mod note_types;
mod routes;
mod state;

use std::net::SocketAddr;

use state::AppState;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let db = db::connect().await.expect("database connection failed");

    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .expect("migration failed");

    // -------------------------------------------------------------------
    // Background cleanup task
    // -------------------------------------------------------------------

    let cleanup_db = db.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;

        loop {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            match sqlx::query!("DELETE FROM revoked_tokens WHERE expires_at <= ?", now)
                .execute(&cleanup_db)
                .await
            {
                Ok(result) => {
                    let deleted = result.rows_affected();
                    if deleted > 0 {
                        tracing::info!("Cleaned up {deleted} expired revoked token(s)");
                    }
                }
                Err(e) => {
                    tracing::error!("Token cleanup failed: {e}");
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });

    // -------------------------------------------------------------------
    // Start the server
    // -------------------------------------------------------------------

    let state = AppState { db };
    let app = app::app(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("Listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
