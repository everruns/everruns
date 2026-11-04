//! Shared, serialized TLS environment fixtures for worker transport tests.
use std::sync::{LazyLock, Mutex, MutexGuard};

static TLS_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

pub(crate) struct TlsEnvGuard {
    _lock: MutexGuard<'static, ()>,
    ca_cert: Option<String>,
    cert: Option<String>,
    key: Option<String>,
    domain: Option<String>,
    token: Option<String>,
}

impl TlsEnvGuard {
    pub(crate) fn new() -> Self {
        let lock = TLS_ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        Self {
            _lock: lock,
            ca_cert: std::env::var("WORKER_GRPC_TLS_CA_CERT").ok(),
            cert: std::env::var("WORKER_GRPC_TLS_CERT").ok(),
            key: std::env::var("WORKER_GRPC_TLS_KEY").ok(),
            domain: std::env::var("WORKER_GRPC_TLS_DOMAIN").ok(),
            token: std::env::var("WORKER_GRPC_AUTH_TOKEN").ok(),
        }
    }

    pub(crate) fn set_var(&self, key: &str, value: &str) {
        unsafe {
            std::env::set_var(key, value);
        }
    }

    pub(crate) fn remove_var(&self, key: &str) {
        unsafe {
            std::env::remove_var(key);
        }
    }
}

impl Drop for TlsEnvGuard {
    fn drop(&mut self) {
        restore_env_var("WORKER_GRPC_TLS_CA_CERT", self.ca_cert.as_deref());
        restore_env_var("WORKER_GRPC_TLS_CERT", self.cert.as_deref());
        restore_env_var("WORKER_GRPC_TLS_KEY", self.key.as_deref());
        restore_env_var("WORKER_GRPC_TLS_DOMAIN", self.domain.as_deref());
        restore_env_var("WORKER_GRPC_AUTH_TOKEN", self.token.as_deref());
    }
}

fn restore_env_var(key: &str, value: Option<&str>) {
    unsafe {
        if let Some(value) = value {
            std::env::set_var(key, value);
        } else {
            std::env::remove_var(key);
        }
    }
}
