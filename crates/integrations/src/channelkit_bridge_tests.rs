use crate::channelkit_bridge::EgressTransport;
use async_trait::async_trait;
use channelkit::{
    Error,
    http::{Request, Transport},
};
use everruns_contracts::runtime::egress::*;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

struct Fake {
    seen: Mutex<Vec<EgressRequest>>,
    mode: u8,
}
#[async_trait]
impl EgressService for Fake {
    async fn send(&self, _: EgressRequest) -> EgressResult<EgressResponse> {
        unreachable!()
    }
    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        assert_eq!(request.kind, EgressRequestKind::Integration);
        assert_eq!(request.timeout_ms, Some(30_000));
        assert!(request.dns_pinning_required);
        self.seen.lock().unwrap().push(request);
        if self.mode == 1 {
            return Err(EgressError::Transport(
                "secret private URL diagnostic".into(),
            ));
        }
        Ok(EgressStreamResponse {
            status: 302,
            headers: BTreeMap::from([("location".into(), "https://untrusted.invalid".into())]),
            body: if self.mode == 2 {
                Box::pin(futures::stream::pending())
            } else {
                Box::pin(futures::stream::iter([
                    Ok(b"ab".to_vec()),
                    Ok(b"cd".to_vec()),
                ]))
            },
        })
    }
}
fn request(max_response_bytes: usize) -> Request {
    Request {
        method: "POST".into(),
        url: "https://slack.com/api/fixture".into(),
        headers: BTreeMap::from([("authorization".into(), "Bearer fixture".into())]),
        body: b"payload".to_vec(),
        max_response_bytes,
    }
}
#[tokio::test]
async fn scoped_egress_is_used_once_and_redirects_are_left_to_the_provider() {
    let egress = Arc::new(Fake {
        seen: Mutex::new(vec![]),
        mode: 0,
    });
    let response = EgressTransport::new(egress.clone())
        .execute(request(4))
        .await
        .unwrap();
    assert_eq!(response.status, 302);
    assert_eq!(response.body, b"abcd");
    let seen = egress.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].headers["authorization"], "Bearer fixture");
    assert_eq!(seen[0].body, b"payload");
}
#[tokio::test]
async fn response_bound_and_denial_are_closed_errors_without_retries() {
    for mode in [0, 1] {
        let egress = Arc::new(Fake {
            seen: Mutex::new(vec![]),
            mode,
        });
        let error = EgressTransport::new(egress.clone())
            .execute(request(3))
            .await
            .err()
            .unwrap();
        assert!(if mode == 0 {
            matches!(error, Error::Capacity(_))
        } else {
            matches!(error, Error::Transport)
        });
        assert!(!error.to_string().contains("secret"));
        assert_eq!(egress.seen.lock().unwrap().len(), 1);
    }
}
#[tokio::test(start_paused = true)]
async fn response_body_stalls_have_a_whole_operation_deadline() {
    let egress = Arc::new(Fake {
        seen: Mutex::new(vec![]),
        mode: 2,
    });
    assert!(matches!(
        EgressTransport::new(egress.clone())
            .execute(request(4))
            .await,
        Err(Error::Transport)
    ));
    assert_eq!(egress.seen.lock().unwrap().len(), 1);
}
