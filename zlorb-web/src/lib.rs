use axum::{
    Json, Router,
    extract::State,
    response::IntoResponse,
    routing::{get, post},
};
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};
use zlorb_lib::config::RepositoryConfiguration;

const IP: &str = "0.0.0.0";
const PORT: &str = "3000";

#[tokio::main]
pub async fn run() {
    let (config, _) =
        zlorb_lib::create_config_from_toml(false).expect("Failed to load zlorb configuration");

    let webhook_routes = Router::new().route("/webhook", post(handle_webhook));
    let frontend_service = ServeDir::new("zlorb-web/dist")
        .not_found_service(ServeFile::new("zlorb-web/frontend/public/404.html"));

    let app = Router::new()
        .nest("/api", webhook_routes)
        .route("/api/repositories", get(get_tracked_repositories))
        .fallback_service(frontend_service)
        .with_state(config.repositories);

    let binding = format!("{IP}:{PORT}");
    let listener = TcpListener::bind(&binding).await.unwrap();
    println!("Listening on http://{}", binding);

    axum::serve(listener, app).await.unwrap();
}

async fn get_tracked_repositories(
    State(repositories): State<Vec<RepositoryConfiguration>>,
) -> impl IntoResponse {
    Json(repositories)
}

async fn handle_webhook(Json(payload): Json<serde_json::Value>) {
    println!("Received webhook: {:#?}", payload);
}
