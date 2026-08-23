use std::sync::{Arc, Barrier};

#[test]
fn concurrent_provider_clients_install_crypto_provider_without_panicking() {
    const THREADS: usize = 24;
    let barrier = Arc::new(Barrier::new(THREADS));
    let mut handles = Vec::with_capacity(THREADS);

    for index in 0..THREADS {
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            if index % 2 == 0 {
                let _ = everruns_provider::driver_helpers::shared_streaming_http_client();
            } else {
                let _ = everruns_provider::driver_helpers::shared_request_http_client();
            }
            everruns_provider::install_default_crypto_provider();
        }));
    }

    for handle in handles {
        handle
            .join()
            .expect("TLS client initialization must not panic");
    }

    // The workspace links exactly one crypto backend (EVE-924). Naming
    // `rustls::crypto::aws_lc_rs` at all is already a compile-time assertion
    // that `aws-lc-rs` is the selected one — `rustls::crypto::ring` does not
    // exist under this feature set. This checks the runtime half: that the
    // provider the racing installers above actually won with is that backend,
    // and not one a re-added `ring` dependency installed first.
    let installed = rustls::crypto::CryptoProvider::get_default()
        .expect("a crypto provider must be installed after client construction");
    let expected = rustls::crypto::aws_lc_rs::default_provider();

    let installed_suites: Vec<_> = installed
        .cipher_suites
        .iter()
        .map(|suite| suite.suite())
        .collect();
    let expected_suites: Vec<_> = expected
        .cipher_suites
        .iter()
        .map(|suite| suite.suite())
        .collect();

    assert_eq!(
        installed_suites, expected_suites,
        "process default crypto provider is not aws-lc-rs; a second rustls backend is linked"
    );
}
