mod template;

use askama::Template;
use axum::{
    Router,
    response::{Html, IntoResponse},
    routing::get,
};
use zlorb_lib::create_config_from_toml;

use crate::template::Index;

const IP: &str = "0.0.0.0";
const PORT: &str = "3000";

#[tokio::main]
pub async fn run() {
    let app = Router::new().route("/", get(get_root));

    let binding = format!("{IP}:{PORT}");
    let listener = tokio::net::TcpListener::bind(&binding).await.unwrap();
    println!("Listening on http://{}", binding);

    axum::serve(listener, app).await.unwrap();
}

async fn get_root() -> impl IntoResponse {
    let mut index = Index::new();
    let config = create_config_from_toml(false).unwrap().0;
    let repos = config.repositories;
    let mapped_repos: Vec<String> = repos.iter().map(|x| x.path.clone()).collect();

    index.repos = if mapped_repos.len() < 1 {
        None
    } else {
        Some(mapped_repos)
    };

    let index_render = index.render().unwrap();
    Html(index_render)
}
