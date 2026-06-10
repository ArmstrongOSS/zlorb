#[tokio::main]
async fn main() {
    let webhook_routes = axum::routing::Router::new().route("/webhook", axum::routing::post(handle_webhook));
    let frontend_service = axum::routing::get_service(tower_http::services::ServeDir::new("dist"))
        .handle_error(|_| async {
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Unhandled internal error")
        });

    let app = axum::routing::Router::new()
        .nest("/api", webhook_routes)
        .route("/health", axum::routing::get(check_health))
        .fallback_service(frontend_service);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    println!("Listening on http://127.0.0.1:3000/");
    axum::serve(listener, app).await.unwrap();
}

async fn check_health() -> String {
    "ONLINE".to_string()
}

async fn handle_webhook(axum::extract::Json(payload): axum::extract::Json<serde_json::Value>) {
    println!("Received webhook: {:#?}", payload);
}
