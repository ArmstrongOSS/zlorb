use askama::Template;

#[derive(Template)]
#[template(path = "index.html")]
pub(crate) struct Index {
    title: String,
}

impl Index {
    pub(crate) fn new() -> Self {
        Self {
            title: "HTMX Demo".to_string(),
        }
    }
}
