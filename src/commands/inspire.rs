use crate::context::AppContext;
use crate::errors::XmasterError;
use crate::intel::store::IntelStore;
use crate::output::{self, CsvRenderable, OutputFormat, Tableable};
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
struct InspireResults {
    query: String,
    count: usize,
    library_size: i64,
    posts: Vec<InspireRow>,
}

#[derive(Serialize)]
struct InspireRow {
    id: String,
    author: String,
    text: String,
    likes: i64,
    impressions: i64,
    source: String,
}

impl Tableable for InspireResults {
    fn to_table(&self) -> comfy_table::Table {
        let mut table = comfy_table::Table::new();
        table.set_header(vec!["ID", "Author", "Text", "Likes", "Views", "Via"]);
        for p in &self.posts {
            let truncated = if p.text.len() > 120 {
                let boundary = p.text.floor_char_boundary(117);
                format!("{}...", &p.text[..boundary])
            } else {
                p.text.clone()
            };
            table.add_row(vec![
                &p.id, &p.author, &truncated,
                &p.likes.to_string(), &p.impressions.to_string(), &p.source,
            ]);
        }
        table
    }
}

impl CsvRenderable for InspireResults {
    fn csv_headers() -> Vec<&'static str> {
        vec!["id", "author", "text", "likes", "impressions", "source"]
    }
    fn csv_rows(&self) -> Vec<Vec<String>> {
        self.posts.iter().map(|p| vec![
            p.id.clone(), p.author.clone(), p.text.clone(),
            p.likes.to_string(), p.impressions.to_string(), p.source.clone(),
        ]).collect()
    }
}

pub async fn execute(
    _ctx: Arc<AppContext>,
    format: OutputFormat,
    topic: Option<&str>,
    author: Option<&str>,
    min_likes: Option<i64>,
    count: usize,
) -> Result<(), XmasterError> {
    let store = IntelStore::open()
        .map_err(|e| XmasterError::Config(format!("DB error: {e}")))?;

    let library_size = store.discovered_posts_count()
        .map_err(|e| XmasterError::Config(format!("DB error: {e}")))?;

    let rows = store.query_discovered_posts(topic, author, min_likes, count)
        .map_err(|e| XmasterError::Config(format!("Query error: {e}")))?;

    if rows.is_empty() {
        let hint = if library_size == 0 {
            "Library is empty. Run `xmaster search`, `xmaster timeline`, or `xmaster read` to start building it."
        } else {
            "No posts match your filters. Try broader criteria or omit --min-likes."
        };
        return Err(XmasterError::NotFound(hint.into()));
    }

    let display = InspireResults {
        query: topic.unwrap_or("all").to_string(),
        count: rows.len(),
        library_size,
        posts: rows.into_iter().map(|r| InspireRow {
            id: r.tweet_id,
            author: if r.author_username.is_empty() { "?".into() } else { format!("@{}", r.author_username) },
            text: r.text,
            likes: r.like_count,
            impressions: r.impression_count,
            source: r.last_source,
        }).collect(),
    };
    output::render_csv(format, &display, None);
    Ok(())
}
