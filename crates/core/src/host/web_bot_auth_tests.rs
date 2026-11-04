use super::*;
use ed25519_dalek::{Signature, Verifier};

/// Ed25519 test key from RFC 9421 appendix B.1.4, used by every Web Bot Auth
/// test vector. Test keys must never be used in production (draft 6.8).
const TEST_SEED: &str = "n4Ni-HpISpVObnQMW0wOhCKROaIKqKtW_2ZYb2p9KcU";
const TEST_KEYID: &str = "poqkLGiymh_W0uP6PZFw-dvez3QJT5SolqXBCW38r0U";

fn signer() -> WebBotAuth {
    WebBotAuth::new(TEST_SEED, "https://signature-agent.test").unwrap()
}

fn header<'a>(headers: &'a SignatureHeaders, name: &str) -> &'a str {
    headers
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.as_str())
        .unwrap_or_else(|| panic!("missing {name}"))
}

fn verify(base: &str, signature_b64: &str) {
    let bytes = STANDARD.decode(signature_b64).expect("standard base64");
    let signature = Signature::from_slice(&bytes).unwrap();
    signing_key_from_seed(TEST_SEED)
        .unwrap()
        .verifying_key()
        .verify(base.as_bytes(), &signature)
        .expect("signature verifies");
}

#[test]
fn keyid_is_the_rfc8037_thumbprint_of_the_test_key() {
    assert_eq!(signer().keyid(), TEST_KEYID);
}

#[test]
fn signs_the_draft_legacy_signature_agent_vector() {
    // draft-ietf-webbotauth-httpsig-protocol-01 appendix E.2.2: the same key
    // and encoding must reproduce the published signature byte for byte.
    let base = "\"@authority\": example.com\n\
        \"signature-agent\": \"https://signature-agent.test\"\n\
        \"@signature-params\": (\"@authority\" \"signature-agent\");created=1735689600;\
        keyid=\"poqkLGiymh_W0uP6PZFw-dvez3QJT5SolqXBCW38r0U\";alg=\"ed25519\";\
        expires=1735693200;nonce=\"e8N7S2MFd/qrd6T2R3tdfAuuANngKI7LFtKYI/vowzk4lAZYadIX6wW25MwG7DCT9RUKAJ0qVkU0mEeLElW1qg==\";\
        tag=\"web-bot-auth\"";
    let signer = signer();
    assert_eq!(
        signer.sign(&signer.signing, base),
        "jdq0SqOwHdyHr9+r5jw3iYZH6aNGKijYp/EstF4RQTQdi5N5YYKrD+mCT1HA1nZDsi6nJKuHxUi/5Syp3rLWBA=="
    );
}

#[test]
fn reproduces_the_draft_signed_directory_vector() {
    // Appendix E.2.3: body bytes, Content-Digest and signature all match.
    let directory = signer().directory_with("signature-agent.test", 1735689600, 4889289600);
    assert_eq!(
        directory.body,
        "{\"keys\":[{\"kty\":\"OKP\",\"crv\":\"Ed25519\",\"kid\":\"poqkLGiymh_W0uP6PZFw-dvez3QJT5SolqXBCW38r0U\",\
         \"x\":\"JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs\",\"use\":\"sig\"}]}"
    );
    assert_eq!(
        header(&directory.headers, "content-digest"),
        "sha-256=:CADMT2aBdV/rqQr/NIru64ERQkCobVvllA4V0fLFDu0=:"
    );
    assert_eq!(
        header(&directory.headers, "signature-input"),
        "binding0=(\"@authority\";req \"content-digest\");created=1735689600;expires=4889289600;\
         keyid=\"poqkLGiymh_W0uP6PZFw-dvez3QJT5SolqXBCW38r0U\";tag=\"http-message-signatures-directory\""
    );
    assert_eq!(
        header(&directory.headers, "signature"),
        "binding0=:l6P8R67tm3kujAxbHWio7ll01qrEZ0dKD/WWlGhNYEmTnFZM8Wt0VQ9zqGfvo7T/UMkBxsigzChM1Gpz7gOVBg==:"
    );
}

#[test]
fn request_signature_covers_method_authority_path_and_agent() {
    let headers = signer()
        .sign_request_with(
            "get",
            "https://Example.COM:8443/docs/page?q=1",
            &RequestParams {
                created: 100,
                expires: 400,
                nonce: "bm9uY2U",
            },
        )
        .unwrap();
    assert_eq!(
        header(&headers, "signature-agent"),
        "\"https://signature-agent.test\""
    );
    let input = header(&headers, "signature-input");
    let params = input.strip_prefix("sig1=").unwrap();
    assert_eq!(
        params,
        "(\"@method\" \"@authority\" \"@path\" \"signature-agent\");created=100;\
         keyid=\"poqkLGiymh_W0uP6PZFw-dvez3QJT5SolqXBCW38r0U\";alg=\"ed25519\";expires=400;\
         nonce=\"bm9uY2U\";tag=\"web-bot-auth\""
    );
    let base = format!(
        "\"@method\": GET\n\"@authority\": example.com:8443\n\"@path\": /docs/page\n\
         \"signature-agent\": \"https://signature-agent.test\"\n\"@signature-params\": {params}"
    );
    let signature = header(&headers, "signature");
    let value = signature
        .strip_prefix("sig1=:")
        .and_then(|rest| rest.strip_suffix(':'))
        .unwrap();
    verify(&base, value);
}

#[test]
fn default_port_is_omitted_and_empty_path_is_slash() {
    let headers = signer()
        .sign_request_with(
            "POST",
            "https://example.com:443",
            &RequestParams {
                created: 1,
                expires: 2,
                nonce: "n",
            },
        )
        .unwrap();
    let params = header(&headers, "signature-input")
        .strip_prefix("sig1=")
        .unwrap();
    let base = format!(
        "\"@method\": POST\n\"@authority\": example.com\n\"@path\": /\n\
         \"signature-agent\": \"https://signature-agent.test\"\n\"@signature-params\": {params}"
    );
    let signature = header(&headers, "signature");
    verify(&base, &signature["sig1=:".len()..signature.len() - 1]);
}

#[test]
fn live_signatures_are_fresh_and_use_standard_base64() {
    let headers = signer()
        .sign_request("GET", "https://example.com/")
        .unwrap();
    let input = header(&headers, "signature-input");
    let created: u64 = input
        .split(";created=")
        .nth(1)
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let expires: u64 = input
        .split(";expires=")
        .nth(1)
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(expires - created, 300);
    let signature = header(&headers, "signature");
    let value = &signature["sig1=:".len()..signature.len() - 1];
    assert_eq!(STANDARD.decode(value).unwrap().len(), 64);
    let other = signer()
        .sign_request("GET", "https://example.com/")
        .unwrap();
    assert_ne!(
        header(&other, "signature-input"),
        input,
        "nonce differs per request"
    );
}

#[test]
fn plain_http_and_invalid_urls_are_not_signed() {
    assert!(matches!(
        signer().sign_request("GET", "http://example.com/"),
        Err(WebBotAuthError::UnsignableUrl(_))
    ));
    assert!(signer().sign_request("GET", "not a url").is_err());
}

#[test]
fn signature_agent_must_be_https_and_is_reduced_to_its_origin() {
    assert_eq!(
        WebBotAuth::new(TEST_SEED, "https://App.Example.com/some/path")
            .unwrap()
            .signature_agent,
        "\"https://app.example.com\""
    );
    for bad in ["http://app.example.com", "app.example.com", ""] {
        assert_eq!(
            WebBotAuth::new(TEST_SEED, bad).unwrap_err(),
            WebBotAuthError::InvalidSignatureAgent,
            "{bad}"
        );
    }
}

#[test]
fn invalid_seeds_are_rejected() {
    for bad in ["", "!!!", "AAAA"] {
        assert_eq!(
            WebBotAuth::new(bad, "https://a.test").unwrap_err(),
            WebBotAuthError::InvalidSeed
        );
    }
}

#[test]
fn published_rotation_keys_are_listed_and_each_signs_the_directory() {
    let next = URL_SAFE_NO_PAD.encode([7u8; 32]);
    let signer = signer()
        .with_published_seeds([next.as_str(), next.as_str(), TEST_SEED])
        .unwrap();
    let directory = signer.directory_with("app.test", 10, 20);
    let parsed: serde_json::Value = serde_json::from_str(&directory.body).unwrap();
    assert_eq!(parsed["keys"].as_array().unwrap().len(), 2, "deduplicated");
    assert_eq!(parsed["keys"][0]["kid"], TEST_KEYID, "signing key first");
    let inputs = header(&directory.headers, "signature-input");
    assert!(inputs.starts_with("binding0=") && inputs.contains(", binding1="));
    assert_eq!(
        header(&directory.headers, "signature")
            .matches("binding")
            .count(),
        2
    );
}

#[test]
fn from_vars_requires_a_seed_and_an_agent_url() {
    let vars = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    };
    assert!(WebBotAuth::from_vars(vars(&[])).is_none());
    assert!(WebBotAuth::from_vars(vars(&[("BOT_AUTH_SIGNING_KEY_SEED", TEST_SEED)])).is_none());
    let from_app_url = WebBotAuth::from_vars(vars(&[
        ("BOT_AUTH_SIGNING_KEY_SEED", TEST_SEED),
        ("PUBLIC_APP_URL", "https://app.example.com"),
    ]))
    .unwrap();
    assert_eq!(from_app_url.signature_agent, "\"https://app.example.com\"");
    let explicit = WebBotAuth::from_vars(vars(&[
        ("BOT_AUTH_SIGNING_KEY_SEED", TEST_SEED),
        ("PUBLIC_APP_URL", "https://app.example.com"),
        ("BOT_AUTH_SIGNATURE_AGENT_URL", "https://agents.example.com"),
        ("BOT_AUTH_VALIDITY_SECS", "60"),
    ]))
    .unwrap();
    assert_eq!(explicit.signature_agent, "\"https://agents.example.com\"");
    assert_eq!(explicit.validity, Duration::from_secs(60));
    assert!(
        WebBotAuth::from_vars(vars(&[
            ("BOT_AUTH_SIGNING_KEY_SEED", "bad"),
            ("PUBLIC_APP_URL", "https://app.example.com"),
        ]))
        .is_none()
    );
}
