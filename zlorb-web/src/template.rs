use askama::Template;

#[derive(Template, Debug)]
#[template(path = "index.html")]
pub(crate) struct Index {
    title: String,
    pub repos: Option<Vec<String>>,
}

impl Index {
    pub(crate) fn new() -> Self {
        Self {
            title: "Zlorb".to_string(),
            repos: None,
        }
    }
}
