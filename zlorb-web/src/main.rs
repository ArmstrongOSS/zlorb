use serde_json::json;

#[tokio::main]
async fn main() {
    let webhook_routes = axum::routing::Router::new().route("/webhook", axum::routing::post(handle_webhook));
    let frontend_service = axum::routing::get_service(tower_http::services::ServeDir::new("zlorb-web/dist"))
        .handle_error(|_| async {
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Unhandled internal error")
        });

    let app = axum::routing::Router::new()
        .nest("/api", webhook_routes)
        .route("/api/repositories", axum::routing::get(get_tracked_repositories))
        .fallback_service(frontend_service);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    println!("Listening on http://127.0.0.1:3000/");
    axum::serve(listener, app).await.unwrap();
}

async fn get_tracked_repositories() -> impl axum::response::IntoResponse {
    // NOTE: hard coded example
    axum::Json(json!([
        { "name": "doom", "path": "/Users/joban/Workspaces/Professional/zlorb/res/doom", "remote": "origin", "branch": "master", "build_command": "" },
        { "name": "quake", "path": "/Users/joban/Workspaces/Professional/zlorb/res/quake", "remote": "origin", "branch": "master", "build_command": "" }
    ]))
}

async fn handle_webhook(axum::extract::Json(payload): axum::extract::Json<serde_json::Value>) {
    println!("Received webhook: {:#?}", payload);
}
