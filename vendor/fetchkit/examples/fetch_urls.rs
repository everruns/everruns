//! Example: Fetch various URLs and display results
//!
//! Run with: cargo run -p fetchkit --example fetch_urls
//!
//! Demonstrates the library API by fetching real URLs and showing
//! how FetchKit handles different content types (HTML, JSON, plain text).

use fetchkit::{FetchRequest, Tool};

#[tokio::main]
async fn main() {
    println!("FetchKit URL Examples");
    println!("=====================\n");

    let tool = Tool::builder().enable_markdown(true).build();

    let mut passed = 0;
    let mut failed = 0;

    // 1. Fetch HTML as markdown
    println!("1. Fetch HTML as markdown");
    println!("   URL: https://example.com");
    let req = FetchRequest::new("https://example.com").as_markdown();
    match tool.execute(req).await {
        Ok(resp) => {
            println!("   Status: {}", resp.status_code);
            println!("   Format: {:?}", resp.format);
            if let Some(ref ct) = resp.content_type {
                println!("   Content-Type: {}", ct);
            }
            if let Some(ref content) = resp.content {
                let preview: String = content.chars().take(120).collect();
                println!("   Preview: {}", preview.replace('\n', " "));
            }
            if resp.status_code == 200 && resp.format.as_deref() == Some("markdown") {
                println!("   PASS\n");
                passed += 1;
            } else {
                println!("   FAIL\n");
                failed += 1;
            }
        }
        Err(e) => {
            println!("   Error: {e}\n   FAIL\n");
            failed += 1;
        }
    }

    // 2. Fetch a real site as markdown
    println!("2. Fetch everruns.com as markdown");
    println!("   URL: https://everruns.com");
    let req = FetchRequest::new("https://everruns.com").as_markdown();
    match tool.execute(req).await {
        Ok(resp) => {
            println!("   Status: {}", resp.status_code);
            println!("   Format: {:?}", resp.format);
            if let Some(ref content) = resp.content {
                let preview: String = content.chars().take(120).collect();
                println!("   Preview: {}", preview.replace('\n', " "));
            }
            if resp.status_code == 200 {
                println!("   PASS\n");
                passed += 1;
            } else {
                println!("   FAIL\n");
                failed += 1;
            }
        }
        Err(e) => {
            println!("   Error: {e}\n   FAIL\n");
            failed += 1;
        }
    }

    // 3. Fetch HTML as plain text
    println!("3. Fetch HTML as text");
    println!("   URL: https://example.com");
    let req = FetchRequest::new("https://example.com").as_text();
    match tool.execute(req).await {
        Ok(resp) => {
            println!("   Status: {}", resp.status_code);
            println!("   Format: {:?}", resp.format);
            if let Some(ref content) = resp.content {
                let preview: String = content.chars().take(120).collect();
                println!("   Preview: {}", preview.replace('\n', " "));
            }
            if resp.status_code == 200 && resp.format.as_deref() == Some("text") {
                println!("   PASS\n");
                passed += 1;
            } else {
                println!("   FAIL\n");
                failed += 1;
            }
        }
        Err(e) => {
            println!("   Error: {e}\n   FAIL\n");
            failed += 1;
        }
    }

    // 4. HEAD request (metadata only)
    println!("4. HEAD request (metadata only)");
    println!("   URL: https://example.com");
    let req = FetchRequest::new("https://example.com").method(fetchkit::HttpMethod::Head);
    match tool.execute(req).await {
        Ok(resp) => {
            println!("   Status: {}", resp.status_code);
            println!("   Method: {:?}", resp.method);
            if let Some(ref ct) = resp.content_type {
                println!("   Content-Type: {}", ct);
            }
            if let Some(size) = resp.size {
                println!("   Size: {} bytes", size);
            }
            if resp.status_code == 200 && resp.content.is_none() {
                println!("   PASS\n");
                passed += 1;
            } else {
                println!("   FAIL\n");
                failed += 1;
            }
        }
        Err(e) => {
            println!("   Error: {e}\n   FAIL\n");
            failed += 1;
        }
    }

    // 5. URL validation (invalid scheme rejected)
    println!("5. URL validation (ftp:// rejected)");
    let req = FetchRequest::new("ftp://example.com/file");
    match tool.execute(req).await {
        Err(e) => {
            println!("   Correctly rejected: {e}");
            println!("   PASS\n");
            passed += 1;
        }
        Ok(_) => {
            println!("   Should have been rejected!\n   FAIL\n");
            failed += 1;
        }
    }

    // 6. Private IP blocked by default (SSRF protection)
    println!("6. Private IP blocked by default (SSRF protection)");
    let req = FetchRequest::new("http://127.0.0.1/");
    match tool.execute(req).await {
        Err(e) => {
            println!("   Correctly blocked: {e}");
            println!("   PASS\n");
            passed += 1;
        }
        Ok(_) => {
            println!("   Should have been blocked!\n   FAIL\n");
            failed += 1;
        }
    }

    // 7. Twitter/X regular tweet via syndication API (soft — third-party API)
    println!("7. Fetch Twitter/X tweet (soft, third-party API)");
    println!("   URL: https://x.com/rustlang/status/1821986021505405014");
    let req = FetchRequest::new("https://x.com/rustlang/status/1821986021505405014");
    match tool.execute(req).await {
        Ok(resp) => {
            println!("   Status: {}", resp.status_code);
            println!("   Format: {:?}", resp.format);
            if let Some(ref content) = resp.content {
                let preview: String = content.chars().take(200).collect();
                println!("   Preview: {}", preview.replace('\n', " "));
            }
            if resp.status_code == 200 && resp.format.as_deref() == Some("twitter_tweet") {
                println!("   PASS\n");
            } else {
                println!("   SKIP (third-party API unavailable)\n");
            }
        }
        Err(e) => {
            println!("   SKIP (third-party API unavailable: {e})\n");
        }
    }

    // 8. Twitter/X article tweet (soft — third-party API)
    println!("8. Fetch Twitter/X article tweet (soft, third-party API)");
    println!("   URL: https://x.com/zachlloydtweets/status/2036509756404158559");
    let req = FetchRequest::new("https://x.com/zachlloydtweets/status/2036509756404158559");
    match tool.execute(req).await {
        Ok(resp) => {
            println!("   Status: {}", resp.status_code);
            println!("   Format: {:?}", resp.format);
            if let Some(ref content) = resp.content {
                let preview: String = content.chars().take(300).collect();
                println!("   Preview: {}", preview.replace('\n', " "));
            }
            if resp.status_code == 200 && resp.format.as_deref() == Some("twitter_tweet") {
                println!("   PASS\n");
            } else {
                println!("   SKIP (third-party API unavailable)\n");
            }
        }
        Err(e) => {
            println!("   SKIP (third-party API unavailable: {e})\n");
        }
    }

    println!("=====================");
    println!("Results: {} passed, {} failed", passed, failed);

    if failed > 0 {
        std::process::exit(1);
    }
}
