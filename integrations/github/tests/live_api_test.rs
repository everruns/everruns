#![cfg(feature = "github-live-tests")]

use reqwest::StatusCode;

fn github_token() -> String {
    std::env::var("GITHUB_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty())
        .expect("GITHUB_TOKEN must be set when github-live-tests is enabled")
}

#[tokio::test]
async fn github_token_can_search_code_and_read_file() {
    let token = github_token();
    let client = reqwest::Client::new();

    let search = client
        .get("https://api.github.com/search/code?q=fn%20main%20repo%3Arust-lang%2Frust%20path%3Asrc%2Ftools%2Fcargo&per_page=1")
        .bearer_auth(&token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "everruns-github-live-test")
        .send()
        .await
        .expect("GitHub code search request should complete");

    assert_eq!(
        search.status(),
        StatusCode::OK,
        "GitHub code search should succeed: {}",
        search.text().await.unwrap_or_default()
    );

    let file = client
        .get("https://api.github.com/repos/rust-lang/rust/contents/README.md")
        .bearer_auth(&token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "everruns-github-live-test")
        .send()
        .await
        .expect("GitHub file read request should complete");

    assert_eq!(
        file.status(),
        StatusCode::OK,
        "GitHub file read should succeed: {}",
        file.text().await.unwrap_or_default()
    );
}
