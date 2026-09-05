//! Private local persistence for custom hosts using native async tools. The
//! journal directory must not be exposed through session tools or shared with
//! another conversation. Distributed hosts need an equivalent fenced store.

use async_trait::async_trait;
use everruns_engine::native_async::NativeAsyncJournal;
use everruns_provider::{
    error::{AgentLoopError, Result},
    native_async::NativeAsyncCheckpoint,
};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub struct FileNativeAsyncJournal {
    directory: PathBuf,
    // Held for the entire coordinator lifetime. OS locks release on process death.
    _owner: File,
    writer: Mutex<()>,
}

impl FileNativeAsyncJournal {
    pub fn open(directory: impl AsRef<Path>) -> Result<Self> {
        let directory = directory.as_ref().to_path_buf();
        fs::create_dir_all(&directory).map_err(store_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(store_error)?;
        }
        let owner = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("owner.lock"))
            .map_err(store_error)?;
        owner.try_lock().map_err(|_| {
            AgentLoopError::store("native async journal already has an execution owner")
        })?;
        Ok(Self {
            directory,
            _owner: owner,
            writer: Mutex::new(()),
        })
    }
}

fn store_error(error: impl std::fmt::Display) -> AgentLoopError {
    AgentLoopError::store(format!("native async journal: {error}"))
}

#[async_trait]
impl NativeAsyncJournal for FileNativeAsyncJournal {
    async fn load(&self) -> Result<NativeAsyncCheckpoint> {
        let _guard = self.writer.lock().map_err(store_error)?;
        match fs::read(self.directory.join("checkpoint.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(store_error),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(NativeAsyncCheckpoint::default())
            }
            Err(error) => Err(store_error(error)),
        }
    }
    async fn save(&self, checkpoint: &NativeAsyncCheckpoint) -> Result<()> {
        let _guard = self.writer.lock().map_err(store_error)?;
        let data = serde_json::to_vec(checkpoint).map_err(store_error)?;
        let temporary = self.directory.join("checkpoint.next");
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(store_error)?;
        file.write_all(&data).map_err(store_error)?;
        file.sync_all().map_err(store_error)?;
        fs::rename(temporary, self.directory.join("checkpoint.json")).map_err(store_error)?;
        File::open(&self.directory)
            .and_then(|directory| directory.sync_all())
            .map_err(store_error)?;
        Ok(())
    }
}

/// Executes native calls through the same Act pipeline as ordinary runtime tools.
/// The template supplies already-resolved org/session scope and tool definitions.
/// This initial adapter excludes client-side and approval-gated calls entirely.
pub struct RuntimeNativeAsyncExecutor<A: crate::RuntimeHostAdapter> {
    adapter: A,
    template: everruns_engine::ActInput,
}

impl<A: crate::RuntimeHostAdapter> RuntimeNativeAsyncExecutor<A> {
    pub fn new(adapter: A, mut template: everruns_engine::ActInput) -> Self {
        template.tool_calls.clear();
        Self { adapter, template }
    }
}

#[async_trait]
impl<A: crate::RuntimeHostAdapter> everruns_engine::native_async::NativeAsyncExecutor
    for RuntimeNativeAsyncExecutor<A>
{
    async fn authorize(
        &self,
        call: &everruns_provider::native_async::NativeToolCall,
    ) -> Result<everruns_engine::native_async::NativeCallPolicy> {
        use everruns_provider::tool_types::{SideEffectClass, ToolPolicy};
        let definition = self
            .template
            .tool_definitions
            .iter()
            .find(|definition| definition.name() == call.name())
            .ok_or_else(|| {
                AgentLoopError::config("native tool is not in the authorized tool set")
            })?;
        if definition.policy() != &ToolPolicy::Auto {
            return Err(AgentLoopError::config(
                "native coordinator cannot execute approval-gated or client-side tools",
            ));
        }
        // web_fetch also supports an optional file-saving mode despite its
        // read-only hint. Keep that mode on the ordinary synchronous path.
        let saves_file = if let everruns_provider::native_async::NativeToolCall::Function {
            name,
            arguments,
            ..
        } = call
        {
            name == "web_fetch"
                && serde_json::from_str::<serde_json::Value>(arguments)
                    .ok()
                    .is_some_and(|args| {
                        args.get("save_to_file")
                            .is_some_and(|value| !value.is_null() && value != false)
                    })
        } else {
            false
        };
        Ok(everruns_engine::native_async::NativeCallPolicy {
            allow_async: definition.hints().readonly == Some(true)
                && definition.hints().concurrency_class.is_none()
                && !saves_file,
            replay_safe: matches!(
                definition.hints().effective_side_effect_class(),
                SideEffectClass::Pure | SideEffectClass::Idempotent
            ),
            concurrency_class: definition.hints().concurrency_class.clone(),
        })
    }
    async fn execute(
        &self,
        call: everruns_provider::native_async::NativeToolCall,
    ) -> Result<String> {
        use everruns_provider::native_async::NativeToolCall;
        let arguments = match &call {
            NativeToolCall::Function { arguments, .. } => serde_json::from_str(arguments)
                .map_err(|_| AgentLoopError::tool("invalid native tool arguments"))?,
            // Custom tools are raw-string tools. The host's implementation owns
            // validation of that string, including its grammar when configured.
            NativeToolCall::Custom { input, .. } => serde_json::Value::String(input.clone()),
        };
        let mut input = self.template.clone();
        input.tool_calls = vec![everruns_provider::tool_types::ToolCall {
            id: call.id().to_owned(),
            name: call.name().to_owned(),
            arguments,
        }];
        let outcome = crate::execute_act_activity(&self.adapter, input)
            .await
            .map_err(|error| AgentLoopError::tool(error.user_facing_message()))?;
        if outcome.blocked || outcome.waiting_for_tool_results {
            return Err(AgentLoopError::tool(
                "native tool paused for required user action; resolve the action before retrying",
            ));
        }
        let result = outcome
            .results
            .into_iter()
            .next()
            .ok_or_else(|| AgentLoopError::tool("native tool execution returned no result"))?
            .result;
        serde_json::to_string(&serde_json::json!({"result":result.result,"error":result.error,"images":result.images})).map_err(store_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn journal_survives_reopen_and_fences_duplicate_owners() {
        let directory =
            std::env::temp_dir().join(format!("everruns-native-async-{}", uuid::Uuid::new_v4()));
        let journal = FileNativeAsyncJournal::open(&directory).unwrap();
        let mut checkpoint = NativeAsyncCheckpoint::default();
        checkpoint
            .response_completed("persisted_response".into())
            .unwrap();
        journal.save(&checkpoint).await.unwrap();
        assert!(FileNativeAsyncJournal::open(&directory).is_err());
        drop(journal);
        let reopened = FileNativeAsyncJournal::open(&directory).unwrap();
        assert_eq!(reopened.load().await.unwrap(), checkpoint);
        drop(reopened);
        fs::remove_dir_all(directory).unwrap();
    }
}
