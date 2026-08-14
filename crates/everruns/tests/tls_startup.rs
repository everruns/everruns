use std::sync::{Arc, Barrier};

#[test]
fn concurrent_framework_openai_construction_initializes_tls() {
    const THREADS: usize = 24;
    let barrier = Arc::new(Barrier::new(THREADS));
    let mut handles = Vec::with_capacity(THREADS);

    for _ in 0..THREADS {
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let _: everruns::Provider = everruns::OpenAI::new("not-a-real-key").into();
        }));
    }

    for handle in handles {
        handle
            .join()
            .expect("Framework provider construction must not panic");
    }
}
