use super::*;

#[tokio::test]
async fn test_session_export_atif_segmented_walk_reconstructs_session() {
    // Tiny cap forces one step per segment; walk the whole chain and confirm
    // the stitched steps reproduce the seeded session in order.
    let server = TestServer::in_memory_with_atif_export_cap(64).await;
    let session_id = seed_session_with_n_messages(&server, 5).await;

    let mut path = format!(
        "/v1/platform/sessions/{}/export?format=atif&segmented=true",
        session_id
    );
    let mut collected: Vec<serde_json::Value> = Vec::new();
    let mut segment_count = 0;
    loop {
        let response = server.get(&path).await.assert_status(StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/json"
        );
        let seg_index_header = response
            .headers()
            .get("x-atif-segment-index")
            .expect("segment index header")
            .to_str()
            .unwrap()
            .to_string();
        let doc = response.json_value();
        assert_eq!(doc["schema_version"], json!("ATIF-v1.7"));
        assert_eq!(doc["session_id"], json!(session_id.to_string()));
        assert_eq!(
            doc["extra"]["segment_index"].as_u64().unwrap().to_string(),
            seg_index_header,
            "extra.segment_index must match the header"
        );
        for step in doc["steps"].as_array().unwrap() {
            collected.push(step.clone());
        }
        segment_count += 1;
        assert!(segment_count < 100, "segmented walk did not terminate");

        match doc.get("continued_trajectory_ref") {
            Some(reference) => {
                let reference = reference.as_str().expect("ref is a string");
                assert!(reference.contains("segmented=true"));
                assert!(reference.contains("cursor="));
                path = reference.to_string();
            }
            None => break,
        }
    }

    assert!(segment_count >= 2, "tiny cap must yield multiple segments");
    // All five user steps reconstructed, in order, with absolute step ids.
    assert_eq!(collected.len(), 5);
    for (i, step) in collected.iter().enumerate() {
        assert_eq!(step["step_id"], json!(i + 1));
        assert_eq!(step["source"], json!("user"));
        assert_eq!(step["message"], json!(format!("segmented message {i}")));
    }
}

#[tokio::test]
async fn test_session_export_atif_segmented_bad_cursor_returns_400() {
    let server = TestServer::in_memory_with_atif_export_cap(64).await;
    let session_id = seed_session_with_n_messages(&server, 3).await;

    // Garbage cursor is rejected without panicking.
    let response = server
        .get(&format!(
            "/v1/platform/sessions/{}/export?format=atif&segmented=true&cursor=not-a-real-cursor",
            session_id
        ))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    let body = response.json_value();
    assert_eq!(body["code"], json!("atif_cursor_invalid"));
}

#[tokio::test]
async fn test_session_export_atif_segmented_single_segment_when_small() {
    // Default (large) cap: a small session is one self-contained segment with
    // no continuation, even though segmented=true was requested.
    let server = TestServer::in_memory().await;
    let session_id = seed_session_with_n_messages(&server, 2).await;

    let response = server
        .get(&format!(
            "/v1/platform/sessions/{}/export?format=atif&segmented=true",
            session_id
        ))
        .await
        .assert_status(StatusCode::OK);
    assert!(response.headers().get("x-atif-next-cursor").is_none());
    let doc = response.json_value();
    assert!(doc.get("continued_trajectory_ref").is_none());
    assert_eq!(doc["steps"].as_array().unwrap().len(), 2);
    assert_eq!(doc["extra"]["segment_index"], json!(0));
}
