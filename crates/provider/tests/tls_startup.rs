use std::sync::{Arc, Barrier};

#[test]
fn concurrent_provider_clients_install_ring_without_panicking() {
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
            everruns_provider::install_ring_crypto_provider();
        }));
    }

    for handle in handles {
        handle
            .join()
            .expect("TLS client initialization must not panic");
    }
}
