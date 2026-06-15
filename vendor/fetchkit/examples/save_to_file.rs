//! Example: Download files using save_to_file
//!
//! Run with: cargo run -p fetchkit --example save_to_file
//!
//! Demonstrates the FileSaver trait and save_to_file feature by downloading
//! real web content to a temporary directory.

use fetchkit::{FetchRequest, LocalFileSaver, Tool};

#[tokio::main]
async fn main() {
    println!("FetchKit save_to_file Example");
    println!("==============================\n");

    let dir = tempfile::tempdir().expect("Failed to create temp dir");
    let saver = LocalFileSaver::new(Some(dir.path().to_path_buf()));
    let tool = Tool::builder()
        .enable_markdown(true)
        .enable_save_to_file(true)
        .build();

    let mut passed = 0;
    let mut failed = 0;

    // 1. Save HTML page to file
    println!("1. Save HTML page to file");
    let req = FetchRequest::new("https://example.com").save_to_file("downloads/page.html");
    match tool.execute_with_saver(req, Some(&saver)).await {
        Ok(resp) => {
            println!("   Status: {}", resp.status_code);
            println!("   Saved to: {:?}", resp.saved_path);
            println!("   Bytes written: {:?}", resp.bytes_written);
            if resp.status_code == 200 && resp.saved_path.is_some() && resp.content.is_none() {
                let on_disk = std::fs::read_to_string(dir.path().join("downloads/page.html"))
                    .expect("File should exist on disk");
                assert!(!on_disk.is_empty());
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

    // 2. Save binary file (SVG/image)
    println!("2. Save binary file (SVG)");
    let req = FetchRequest::new("https://docs.everruns.com/favicon.svg")
        .save_to_file("downloads/favicon.svg");
    match tool.execute_with_saver(req, Some(&saver)).await {
        Ok(resp) => {
            println!("   Status: {}", resp.status_code);
            println!("   Saved to: {:?}", resp.saved_path);
            println!("   Bytes written: {:?}", resp.bytes_written);
            if resp.status_code == 200 && resp.saved_path.is_some() {
                let bytes = std::fs::read(dir.path().join("downloads/favicon.svg"))
                    .expect("File should exist");
                assert!(!bytes.is_empty());
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

    // 3. Path traversal rejection (safety check)
    println!("3. Path traversal rejection (safety check)");
    let req = FetchRequest::new("https://example.com").save_to_file("../../etc/passwd");
    match tool.execute_with_saver(req, Some(&saver)).await {
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

    // 4. No saver provided
    println!("4. No saver provided (safety check)");
    let req = FetchRequest::new("https://example.com").save_to_file("file.txt");
    match tool.execute_with_saver(req, None).await {
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

    // 5. Feature disabled (safety check)
    println!("5. Feature disabled (safety check)");
    let disabled_tool = Tool::builder().build();
    let req = FetchRequest::new("https://example.com").save_to_file("file.txt");
    match disabled_tool.execute_with_saver(req, Some(&saver)).await {
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

    // 6. Normal fetch still works via execute_with_saver
    println!("6. Normal fetch (no save_to_file) still works");
    let req = FetchRequest::new("https://example.com");
    match tool.execute_with_saver(req, Some(&saver)).await {
        Ok(resp) => {
            println!("   Status: {}", resp.status_code);
            if resp.content.is_some() && resp.saved_path.is_none() {
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

    println!("==============================");
    println!("Results: {} passed, {} failed", passed, failed);

    if failed > 0 {
        std::process::exit(1);
    }
}
