//! Local append-only storage for privacy-minimal call records.

use crate::call_ledger::{CallObserver, CallRecord};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// A new, private JSONL file for one process/run. Existing files, including
/// symlinks, are rejected rather than overwritten. The caller owns the parent
/// directory and retention policy. Each record is flushed to disk before return.
///
/// Observer failures do not retry a paid model call. Hosts must inspect
/// [`Self::has_write_error`] and mark the ledger incomplete if it returns true.
/// Unknown or missing records must never be counted as zero usage.
pub struct JsonlCallObserver {
    file: Mutex<File>,
    write_error: AtomicBool,
}

impl JsonlCallObserver {
    pub fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        Ok(Self {
            file: Mutex::new(options.open(path)?),
            write_error: AtomicBool::new(false),
        })
    }

    pub fn has_write_error(&self) -> bool {
        self.write_error.load(Ordering::Acquire)
    }

    fn append(&self, record: &CallRecord) -> io::Result<()> {
        let mut line = serde_json::to_vec(record)?;
        line.push(b'\n');
        let mut file = self
            .file
            .lock()
            .map_err(|_| io::Error::other("call ledger lock poisoned"))?;
        file.write_all(&line)?;
        file.sync_data()
    }
}

impl CallObserver for JsonlCallObserver {
    fn record(&self, record: CallRecord) {
        if self.append(&record).is_err() {
            self.write_error.store(true, Ordering::Release);
            // Do not log a path, record, or I/O error: those may contain private
            // host data. This warning plus the sticky flag signals lost evidence.
            tracing::warn!("Provider call ledger write failed; attribution is incomplete");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_ledger::ObservedChatDriver;
    use crate::driver_registry::*;
    use crate::runtime_provider::ProviderEndpoint;
    use async_trait::async_trait;
    use std::sync::Arc;

    struct Mock;
    #[async_trait]
    impl ChatDriver for Mock {
        async fn chat_completion_stream(
            &self,
            _: &ProviderEndpoint,
            _: Vec<Message>,
            _: &LlmCallConfig,
        ) -> crate::error::Result<LlmResponseStream> {
            let mut metadata = LlmCompletionMetadata::default();
            metadata.prompt_tokens = Some(11);
            metadata.provider_request_id = Some("req_mock".into());
            Ok(Box::pin(futures::stream::iter([Ok(LlmStreamEvent::Done(
                Box::new(metadata),
            ))])))
        }
    }

    #[tokio::test]
    async fn real_call_writes_readable_private_start_and_terminal_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("calls.jsonl");
        let sink = Arc::new(JsonlCallObserver::create(&path).unwrap());
        let driver = ObservedChatDriver::new("openai", Arc::new(Mock), sink.clone());
        let mut config = LlmCallConfig::new("gpt-6-luna");
        config
            .metadata
            .insert("purpose".into(), "completion_review".into());
        driver
            .chat_completion(&ProviderEndpoint::default(), vec![], &config)
            .await
            .unwrap();
        assert!(!sink.has_write_error());
        let lines: Vec<serde_json::Value> = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["phase"], "started");
        assert_eq!(lines[1]["phase"], "completed");
        assert_eq!(lines[0]["call_id"], lines[1]["call_id"]);
        assert_eq!(lines[1]["provider_request_id"], "req_mock");
        assert_eq!(lines[1]["prompt_tokens"], 11);
        assert!(JsonlCallObserver::create(&path).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_destination_without_modifying_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, "untouched").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(JsonlCallObserver::create(&link).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "untouched");
    }

    #[tokio::test]
    async fn storage_failure_is_sticky_without_retrying_the_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("readonly");
        std::fs::write(&path, "unchanged").unwrap();
        let sink = Arc::new(JsonlCallObserver {
            file: Mutex::new(File::open(&path).unwrap()),
            write_error: AtomicBool::new(false),
        });
        let driver = ObservedChatDriver::new("fixture", Arc::new(Mock), sink.clone());
        driver
            .chat_completion(
                &ProviderEndpoint::default(),
                vec![],
                &LlmCallConfig::new("fixture"),
            )
            .await
            .unwrap();
        assert!(sink.has_write_error());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "unchanged");
    }
}
