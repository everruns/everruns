//! Designated test inbox only. No arbitrary recipients; the test sends to a
//! unique +tag of that same inbox and checks provider-origin delivery by API.
//! This proves API delivery, not a provider-origin signed webhook.
use channelkit_agentmail::{AgentMail, EmailSend, recipient_tag};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
#[tokio::test]
#[ignore = "requires AGENTMAIL_API_KEY and AGENTMAIL_TEST_INBOX; sends test mail"]
async fn live_plus_delivery_alternatives_and_idempotency() {
    let key = std::env::var("AGENTMAIL_API_KEY").expect("AGENTMAIL_API_KEY");
    let inbox = std::env::var("AGENTMAIL_TEST_INBOX").expect("designated AGENTMAIL_TEST_INBOX");
    let (local, domain) = inbox.split_once('@').unwrap();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let tag = format!("everruns-e2e-{nonce}");
    let alias = format!("{local}+{tag}@{domain}");
    let client = AgentMail::new(
        key,
        inbox.clone(),
        Arc::new(channelkit::http::ReqwestTransport::new().unwrap()),
    )
    .unwrap();
    client.verify_inbox().await.expect("test inbox accessible");
    client
        .list_received("2026-01-01T00:00:00+00:00", None)
        .await
        .expect("received-message polling accepts a UTC offset and normalizes it for the provider");
    let send = EmailSend {
        to: vec![alias.clone()],
        cc: vec![],
        subject: format!("Everruns inbox E2E {nonce}"),
        text: format!("Test message {nonce}. No action required."),
        html: Some(format!(
            "<p>Test message <strong>{nonce}</strong>. No action required.</p>"
        )),
    };
    let receipt = client.send(&send, None, &tag).await.expect("test send");
    let retry = client
        .send(&send, None, &tag)
        .await
        .expect("idempotent test retry");
    assert_eq!(
        receipt.message_id, retry.message_id,
        "identical retry must not send another email"
    );
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let mail = client
            .get_message(&receipt.message_id)
            .await
            .expect("read sent/received message");
        if mail.labels.iter().any(|v| v == "received") {
            assert!(
                mail.text
                    .as_deref()
                    .unwrap_or_default()
                    .contains(&nonce.to_string())
            );
            assert!(
                mail.html
                    .as_deref()
                    .unwrap_or_default()
                    .contains(&nonce.to_string())
            );
            assert!(
                mail.to.iter().any(|v| v.contains(&alias)),
                "provider preserves delivered +tag"
            );
            assert_eq!(recipient_tag(&inbox, &alias), Some(tag.as_str()));
            // Exercise the provider's reply endpoint as well. Recipients are
            // fixed to the designated test inbox, never copied from headers.
            let reply = EmailSend {
                to: vec![inbox.clone()],
                cc: vec![],
                subject: "Provider retains thread subject".into(),
                text: format!("Reply test {nonce}. No action required."),
                html: Some(format!("<p>Reply test {nonce}. No action required.</p>")),
            };
            let reply_key = format!("{tag}-reply");
            let sent = client
                .send(&reply, Some(&mail.message_id), &reply_key)
                .await
                .expect("live reply");
            let repeated = client
                .send(&reply, Some(&mail.message_id), &reply_key)
                .await
                .expect("idempotent reply retry");
            assert_eq!(sent.message_id, repeated.message_id);
            assert_eq!(sent.thread_id, mail.thread_id);
            let body = client
                .get_message(&sent.message_id)
                .await
                .expect("read reply");
            assert!(
                body.text
                    .as_deref()
                    .unwrap_or_default()
                    .contains(&nonce.to_string())
            );
            assert!(
                body.html
                    .as_deref()
                    .unwrap_or_default()
                    .contains(&nonce.to_string())
            );
            println!(
                "Live AgentMail PASS: +tag delivery, text/HTML, threaded reply and send/reply idempotency; inbox={inbox}"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "provider did not deliver the +tag message within 120 seconds"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
