use std::sync::{Arc, Mutex};

use everruns_core::{ContentPart, InputMessage};

use super::AttachError;
use crate::{Agent, InMemoryEngine, Model};

fn capturing_agent(
    capture: Arc<Mutex<Vec<Vec<everruns_contracts::driver_registry::Message>>>>,
) -> Agent {
    Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated_capturing("ok", capture))
        .build()
        .expect("valid agent")
}

#[tokio::test]
async fn an_attached_pdf_reaches_the_model_with_its_path() {
    let capture = Arc::new(Mutex::new(Vec::new()));
    let session = InMemoryEngine::new().create(capturing_agent(capture.clone()));

    let part = session
        .attach(
            "Q3 report.pdf",
            Some("application/pdf"),
            b"%PDF-1.7 body".to_vec(),
        )
        .await
        .expect("kept");
    assert!(matches!(part, ContentPart::File(_)), "{part:?}");
    let mut message = InputMessage::user("Summarize it.");
    message.content.push(part);
    session.run(message).await.expect("turn runs");

    let request = format!("{:?}", capture.lock().unwrap().last().expect("one call"));
    assert!(
        request.contains("Attached file /attachments/file_"),
        "the label names the path: {request}"
    );
    assert!(
        request.contains("Q3_report.pdf (application/pdf)"),
        "{request}"
    );
    assert!(
        request.contains("data:application/pdf;base64,"),
        "the bytes go with it: {request}"
    );
}

#[tokio::test]
async fn a_zip_that_cannot_be_unpacked_is_refused_with_its_reason() {
    let session = InMemoryEngine::new().create(capturing_agent(Arc::default()));

    let error = session
        .attach("data.zip", None, b"PK\x03\x04 not really".to_vec())
        .await
        .unwrap_err();

    let AttachError::Refused(reason) = error else {
        panic!("expected a refusal, got {error:?}")
    };
    assert!(reason.starts_with("the zip cannot be unpacked"), "{reason}");
}
