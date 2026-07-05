mod template;

use askama::Template;
use axum::{Router, response::Html, routing::get};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

const IP: &str = "0.0.0.0";
const PORT: &str = "3000";
const UPTIME_POLLING_INTERVAL_SEC: u64 = 30;

#[derive(Template)]
#[template(path = "index.html")]
struct Index {
    title: String,
}

impl Index {
    fn new() -> Self {
        Self {
            title: "HTMX + Askama".to_string(),
        }
    }
}

static START_TIME_SECS: LazyLock<u64> = LazyLock::new(|| {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64
});

async fn system_uptime() -> impl IntoResponse {
    axum::Json(std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs())
}

#[tokio::main]
pub async fn run() {
    let app = Router::new()
        .route("/", get(get_root))
        .route("/api/system", get(system_uptime));

    let binding = format!("{IP}:{PORT}");
    let listener = tokio::net::TcpListener::bind(&binding).await.unwrap();
    println!("Listening on http://{}", binding);

    axum::serve(listener, app).await.unwrap();
}

fn get_root() -> Html<Index> {
    Index::new().render().unwrap().into()
}
