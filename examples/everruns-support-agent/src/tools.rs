const PAGES: &[(&str, &str)] = &[
    ("persistence", include_str!("../docs/persistence.md")),
    (
        "session-history",
        include_str!("../docs/session-history.md"),
    ),
    (
        "models-and-providers",
        include_str!("../docs/models-and-providers.md"),
    ),
    (
        "tools-and-macros",
        include_str!("../docs/tools-and-macros.md"),
    ),
    (
        "custom-providers",
        include_str!("../docs/custom-providers.md"),
    ),
];

fn search(query: &str) -> Result<String, String> {
    let words: Vec<_> = query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|word| word.len() > 2)
        .collect();
    if words.is_empty() {
        return Err("Use a specific topic, such as persistence or provider.".into());
    }
    let mut hits: Vec<_> = PAGES
        .iter()
        .filter_map(|(id, body)| {
            let lower = body.to_lowercase();
            let score: usize = words.iter().map(|word| lower.matches(word).count()).sum();
            (score > 0).then_some((score, *id, *body))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    if hits.is_empty() {
        return Ok("No matches in this five-page snapshot. Say the corpus is limited; do not invent an answer.".into());
    }
    Ok(hits
        .iter()
        .take(3)
        .map(|(_, id, body)| {
            let excerpt = body
                .lines()
                .find(|line| words.iter().any(|word| line.to_lowercase().contains(word)))
                .unwrap_or("");
            format!(
                "Page: {id}\nSource: https://docs.everruns.com/framework/{id}/\nMatch: {excerpt}"
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n"))
}

fn read(id: &str) -> Result<String, String> {
    let (_, body) = PAGES
        .iter()
        .find(|(key, _)| *key == id)
        .ok_or_else(|| "Unknown page ID; use an ID returned by search_docs.".to_string())?;
    Ok(format!(
        "Official docs snapshot, 2026-09-08\nSource: https://docs.everruns.com/framework/{id}/\n{body}"
    ))
}

#[everruns::tool]
/// Search the text of five bundled official documentation pages. Returns ranked page IDs and excerpts.
pub async fn search_docs(query: String) -> Result<String, String> {
    search(&query)
}

#[everruns::tool]
/// Read a complete bundled documentation page by the ID returned from search_docs.
pub async fn read_doc(page_id: String) -> Result<String, String> {
    read(&page_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn searches_content_not_a_hardcoded_topic_branch() {
        let hits = search("resume persistence").unwrap();
        assert!(hits.contains("Page: persistence") || hits.contains("Page: session-history"));
        assert!(hits.contains("Match:"));
        assert!(search("zzzznonexistent").unwrap().starts_with("No matches"));
        assert!(search(" ").is_err());
    }
    #[test]
    fn reads_citable_documents_and_rejects_arbitrary_paths() {
        let page = read("persistence").unwrap();
        assert!(page.contains("https://docs.everruns.com/framework/persistence/"));
        assert!(page.contains("Engine"));
        assert!(read("../../Cargo.toml").is_err());
    }
}
