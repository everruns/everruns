//! Server-owned durable build work. No Session, private identity, or workspace
//! is fabricated to obtain credentials. The named organization account is the
//! only management credential, and is never sent into build compute.
use super::record::*;
use crate::background::cluster_jobs::ClusterJob;
use crate::domains::sandbox_templates::record::SandboxNetworkPolicy;
use crate::storage::{EncryptionService, StorageBackend};
use everruns_contracts::typed_id::SandboxTemplateBuildId;
use everruns_integrations::daytona::{
    api::management::types::CreateSandbox, client::DaytonaClient,
};
use std::{sync::Arc, time::Duration};

pub fn job(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> ClusterJob {
    let Some(encryption) = encryption else {
        return ClusterJob::disabled("sandbox-template-builds", "sandbox_template_builds");
    };
    // A sweep can outlive its cadence. Do not fill the shared maintenance pool
    // with overlapping sweeps while an install is running.
    let sweep_lock = Arc::new(tokio::sync::Mutex::new(()));
    ClusterJob::every(
        "sandbox-template-builds",
        "sandbox_template_builds",
        "Builds reusable Sandbox Template snapshots and reconciles disposable build resources.",
        Duration::from_secs(10),
        move || {
            let db = db.clone();
            let encryption = encryption.clone();
            let sweep_lock = sweep_lock.clone();
            Box::pin(async move {
                let Ok(_sweep) = sweep_lock.try_lock() else {
                    return;
                };
                if let Err(error) = sweep(db, encryption).await {
                    tracing::warn!(%error,"Sandbox Template build sweep failed")
                }
            })
        },
    )
}

async fn sweep(db: Arc<StorageBackend>, encryption: Arc<EncryptionService>) -> anyhow::Result<()> {
    use futures::{StreamExt, stream};
    let pending = db.pending_sandbox_builds().await?;
    stream::iter(pending)
        .for_each_concurrent(4, |(org, id)| {
            let db = db.clone();
            let encryption = encryption.clone();
            async move {
                if let Err(error) = process(db, encryption, org, id).await {
                    tracing::warn!(%error,"Sandbox build processing failed");
                }
            }
        })
        .await;
    Ok(())
}

async fn process(
    db: Arc<StorageBackend>,
    encryption: Arc<EncryptionService>,
    org: i64,
    id: uuid::Uuid,
) -> anyhow::Result<()> {
    let mut lock = everruns_durable::begin_transaction(db.database().pool()).await?;
    let held: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1,5479))")
            .bind(format!("sandbox-build-{id}"))
            .fetch_one(&mut *lock)
            .await?;
    if !held {
        return Ok(());
    }
    let id = SandboxTemplateBuildId::from_uuid(id);
    let Some(mut build) = db.get_sandbox_build(org, id).await? else {
        return Ok(());
    };
    let token = match token(&db, &encryption, org, &build).await {
        Ok(token) => token,
        Err(_) => {
            if matches!(
                build.status,
                SandboxBuildStatus::Queued | SandboxBuildStatus::Running
            ) {
                build.status = SandboxBuildStatus::Failed;
            }
            build.error=Some("The selected provider account is unavailable; restore its access to clean up build resources".into());
            build.cleanup_pending = build.cleanup_pending
                || build.builder_id.is_some()
                || build.verification_sandbox_id.is_some()
                || build.snapshot_id.is_some();
            db.save_sandbox_build(org, &build).await?;
            return Ok(());
        }
    };
    let client = DaytonaClient::new(token.clone()).with_download_limit(64 * 1024);
    if build.status == SandboxBuildStatus::Running {
        // A new owner acquired the lock after the previous process died.
        // Never resume or automatically replay an arbitrary install script.
        build.status = SandboxBuildStatus::Failed;
        build.error = Some("Build interrupted by server restart. Retry from fresh compute.".into());
    } else if build.status == SandboxBuildStatus::Queued {
        if build.cancel_requested {
            build.status = SandboxBuildStatus::Cancelled
        } else {
            build.status = SandboxBuildStatus::Running;
            build.cleanup_pending = true;
            db.save_sandbox_build(org, &build).await?;
            let timeout = Duration::from_secs(build.request.timeout_seconds);
            let result = {
                let execute = execute(&db, org, &client, &token, &mut build);
                tokio::pin!(execute);
                tokio::select! {
                    result=&mut execute=>result,
                    _=tokio::time::sleep(timeout)=>Err(anyhow::anyhow!("Build exceeded its total timeout")),
                    result=watch(&db,org,id,&mut lock)=>result,
                }
            };
            match result {
                Ok(()) => {
                    build.status = SandboxBuildStatus::Ready;
                    build.step = "ready".into()
                }
                Err(error) => {
                    let cancelled = db
                        .get_sandbox_build(org, id)
                        .await?
                        .is_some_and(|b| b.cancel_requested);
                    build.status = if cancelled {
                        SandboxBuildStatus::Cancelled
                    } else {
                        SandboxBuildStatus::Failed
                    };
                    build.error = Some(
                        error
                            .to_string()
                            .replace(&token, "[redacted]")
                            .chars()
                            .take(2048)
                            .collect(),
                    );
                }
            }
        }
    }
    if build.status == SandboxBuildStatus::Ready
        && db
            .get_sandbox_build(org, id)
            .await?
            .is_some_and(|b| b.cancel_requested)
    {
        build.status = SandboxBuildStatus::Cancelled;
    }
    // Cleanup gets a separate bounded window even after total build timeout.
    build.cleanup_pending = !matches!(
        tokio::time::timeout(Duration::from_secs(60), cleanup(&client, &mut build)).await,
        Ok(Ok(()))
    );
    build.updated_at = chrono::Utc::now();
    build.logs = build.logs.replace(&token, "[redacted]");
    db.save_sandbox_build(org, &build).await?;
    everruns_durable::commit_transaction(lock).await?;
    Ok(())
}

async fn watch(
    db: &StorageBackend,
    org: i64,
    id: SandboxTemplateBuildId,
    lock: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> anyhow::Result<()> {
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        sqlx::query("SELECT 1").execute(&mut **lock).await?;
        let build = db
            .get_sandbox_build(org, id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Build was removed"))?;
        if build.cancel_requested {
            anyhow::bail!("Build cancelled")
        }
        let Some(account) = build.source_spec.target.credential.connection_id else {
            anyhow::bail!("Provider account unavailable")
        };
        if db
            .get_organization_connection(org, account)
            .await?
            .is_none()
        {
            anyhow::bail!("Provider account was removed")
        }
    }
}

pub async fn current_token(
    db: &StorageBackend,
    encryption: &EncryptionService,
    org: i64,
    build: &SandboxTemplateBuild,
) -> anyhow::Result<String> {
    let account = build
        .source_spec
        .target
        .credential
        .connection_id
        .ok_or_else(|| anyhow::anyhow!("Missing organization account"))?;
    let account = db
        .get_organization_connection(org, account)
        .await?
        .filter(|a| a.provider == "daytona")
        .ok_or_else(|| anyhow::anyhow!("Organization account unavailable"))?;
    encryption.decrypt_to_string(
        account
            .access_token_encrypted
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Provider credential unavailable"))?,
    )
}

/// Preserve accepted provider authority across connection replacement. A new
/// credential may replace it only after proving access to a known immutable
/// resource id; a 404 from another account is never proof of deletion.
async fn token(
    db: &StorageBackend,
    encryption: &EncryptionService,
    org: i64,
    build: &SandboxTemplateBuild,
) -> anyhow::Result<String> {
    let frozen = db
        .sandbox_build_credential(org, build.id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Build credential binding unavailable"))?;
    let original = encryption.decrypt_to_string(&frozen)?;
    if let Some(account) = build.source_spec.target.credential.connection_id
        && let Some(account) = db.get_organization_connection(org, account).await?
        && let Some(current) = account
            .access_token_encrypted
            .filter(|value| value != &frozen)
        && let Ok(candidate) = encryption.decrypt_to_string(&current)
    {
        let client = DaytonaClient::new(candidate.clone());
        let proof = tokio::time::timeout(Duration::from_secs(10), async {
            if let Some(id) = build.snapshot_id.as_deref()
                && client.get_snapshot(id).await.is_ok()
            {
                return true;
            }
            for id in [
                build.builder_id.as_deref(),
                build.verification_sandbox_id.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                if uuid::Uuid::parse_str(id).is_ok() && client.get_sandbox(id).await.is_ok() {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        if proof {
            db.refresh_sandbox_build_credential(org, build.id, &current)
                .await?;
            return Ok(candidate);
        }
    }
    Ok(original)
}

async fn step(
    db: &StorageBackend,
    org: i64,
    build: &mut SandboxTemplateBuild,
    name: &str,
) -> anyhow::Result<()> {
    build.step = name.into();
    build.updated_at = chrono::Utc::now();
    append_log(&mut build.logs, &format!("\n[{name}]\n"));
    db.save_sandbox_build(org, build).await
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Provider process logs stay bounded too: installation writes to a local file,
/// and only a bounded tail crosses the management transport. This avoids merely
/// truncating after an unbounded provider response has already been allocated.
fn script_command(script: &str) -> String {
    format!(
        "sh -c {}",
        quote(&format!(
            "sh -c {} > /tmp/everruns-build-output 2>&1; __everruns_exit=$?; tail -c 32768 /tmp/everruns-build-output; rm -f /tmp/everruns-build-output; exit \"$__everruns_exit\"",
            quote(script)
        ))
    )
}

async fn run_script(
    client: &DaytonaClient,
    id: &str,
    script: &str,
    cwd: &str,
    logs: &mut String,
) -> anyhow::Result<()> {
    let result = client
        .exec(
            id,
            &script_command(script),
            Some(cwd),
            Some(1_800_000),
            |_| {},
        )
        .await
        .map_err(anyhow::Error::msg)?;
    append_log(logs, &result.stdout);
    append_log(logs, &result.stderr);
    if result.exit_code != 0 {
        anyhow::bail!("Command failed with exit code {}", result.exit_code)
    }
    Ok(())
}

fn desktop(build: &SandboxTemplateBuild) -> bool {
    build
        .source_spec
        .target
        .options
        .get("desktop")
        .is_some_and(|v| v.as_bool() == Some(true) || v.is_object())
}

// THREAT[TM-BASH-034]: Construct fresh compute from explicit build inputs.
// Never forward Session environment, runtime credentials, profiles or volumes.
fn create_body(
    build: &SandboxTemplateBuild,
    name: &str,
    snapshot: Option<&str>,
    network: &SandboxNetworkPolicy,
) -> CreateSandbox {
    let mut body = CreateSandbox {
        name: Some(name.into()),
        snapshot: snapshot.map(str::to_string),
        auto_stop_interval: Some(30),
        auto_delete_interval: Some(60),
        ttl_minutes: Some(60),
        public: Some(false),
        labels: [("everruns.build_id".into(), build.id.to_string())].into(),
        ..Default::default()
    };
    match network {
        SandboxNetworkPolicy::Deny => body.network_block_all = Some(true),
        SandboxNetworkPolicy::Allowlist { allowed_hosts } => {
            body.network_block_all = Some(false);
            body.domain_allow_list = Some(allowed_hosts.join(","))
        }
        SandboxNetworkPolicy::Allow => {}
    }
    if desktop(build) {
        let settings = &build.source_spec.target.options["desktop"];
        let width = settings
            .get("display_width")
            .and_then(|v| v.as_u64())
            .unwrap_or(1280);
        let height = settings
            .get("display_height")
            .and_then(|v| v.as_u64())
            .unwrap_or(800);
        body.env = [
            ("VNC_RESOLUTION".into(), format!("{width}x{height}")),
            ("DISPLAY".into(), ":0".into()),
        ]
        .into();
    }
    body
}

async fn wait_stopped(client: &DaytonaClient, id: &str) -> anyhow::Result<()> {
    for _ in 0..120 {
        let info = client.get_sandbox(id).await.map_err(anyhow::Error::msg)?;
        if info.state == "stopped" {
            return Ok(());
        }
        if info.state == "error" {
            anyhow::bail!("Builder entered an error state while stopping")
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    anyhow::bail!("Provider did not stop the builder before snapshotting")
}

async fn execute(
    db: &StorageBackend,
    org: i64,
    client: &DaytonaClient,
    provider_token: &str,
    build: &mut SandboxTemplateBuild,
) -> anyhow::Result<()> {
    let name = format!("everruns-builder-{}", build.id.uuid().simple());
    // Persist the deterministic name before submission: management APIs accept
    // names for cleanup, including when a create response is lost.
    build.builder_id = Some(name.clone());
    step(db, org, build, "provisioning").await?;
    let options = build.source_spec.target.options.clone();
    let base = options
        .get("snapshot")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            if desktop(build) {
                None
            } else {
                Some(
                    match options
                        .get("size")
                        .and_then(|v| v.as_str())
                        .unwrap_or("small")
                    {
                        "medium" => "daytona-medium",
                        "large" => "daytona-large",
                        _ => "daytona-small",
                    }
                    .into(),
                )
            }
        });
    let runtime_network =
        crate::domains::sandbox_templates::resolution::resolve_spec(&build.source_spec)
            .map_err(anyhow::Error::msg)?
            .containment
            .network;
    let build_network = build
        .request
        .build_network
        .as_ref()
        .unwrap_or(&runtime_network)
        .clone();
    let instance = client
        .create_sandbox(create_body(build, &name, base.as_deref(), &build_network))
        .await
        .map_err(anyhow::Error::msg)?;
    build.builder_id = Some(instance.id.clone());
    db.save_sandbox_build(org, build).await?;
    client
        .wait_for_ready(&instance.id)
        .await
        .map_err(anyhow::Error::msg)?;
    let workspace = options
        .get("workspace_path")
        .and_then(|v| v.as_str())
        .unwrap_or("/home/daytona/workspace")
        .to_string();
    run_script(
        client,
        &instance.id,
        &format!("mkdir -p -- {}", quote(&workspace)),
        "/home/daytona",
        &mut build.logs,
    )
    .await?;
    step(db, org, build, "installing").await?;
    run_script(
        client,
        &instance.id,
        &build.request.install_script,
        &workspace,
        &mut build.logs,
    )
    .await?;
    if let Some(command) = build
        .request
        .verification_command
        .clone()
        .filter(|c| !c.trim().is_empty())
    {
        step(db, org, build, "verifying_installation").await?;
        run_script(client, &instance.id, &command, &workspace, &mut build.logs).await?;
    }
    if let Some(path) = build
        .request
        .metadata_path
        .as_ref()
        .filter(|p| !p.is_empty())
    {
        let result = client
            .exec(
                &instance.id,
                &format!("test -f {p} && head -c 16385 -- {p}", p = quote(path)),
                Some(&workspace),
                Some(15_000),
                |_| {},
            )
            .await
            .map_err(anyhow::Error::msg)?;
        if result.exit_code != 0 || result.stdout.len() > MAX_NOTES_BYTES {
            anyhow::bail!("Metadata file is missing or exceeds 16384 bytes")
        }
        build.software_notes = Some(result.stdout);
    }
    step(db, org, build, "snapshotting").await?;
    client
        .stop_sandbox(&instance.id)
        .await
        .map_err(anyhow::Error::msg)?;
    wait_stopped(client, &instance.id).await?;
    build.snapshot_requested_at = Some(chrono::Utc::now());
    db.save_sandbox_build(org, build).await?;
    client
        .create_sandbox_snapshot(&instance.id, &build.snapshot_name)
        .await
        .map_err(anyhow::Error::msg)?;
    let snapshot = loop {
        if let Some(snapshot) = lookup_snapshot(client, &build.snapshot_name).await? {
            if snapshot.id.is_empty() {
                anyhow::bail!("Provider returned an incomplete snapshot descriptor")
            }
            build.snapshot_id = Some(snapshot.id.clone());
            db.save_sandbox_build(org, build).await?;
            if snapshot.state == "active" {
                break snapshot;
            }
            if matches!(
                snapshot.state.as_str(),
                "error" | "build_failed" | "removing"
            ) {
                anyhow::bail!("Provider snapshot creation failed")
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    step(db, org, build, "verifying_snapshot").await?;
    let verify_name = format!("everruns-verify-{}", build.id.uuid().simple());
    build.verification_sandbox_id = Some(verify_name.clone());
    db.save_sandbox_build(org, build).await?;
    let network = runtime_network;
    let verify = client
        .create_sandbox(create_body(
            build,
            &verify_name,
            Some(&snapshot.id),
            &network,
        ))
        .await
        .map_err(anyhow::Error::msg)?;
    build.verification_sandbox_id = Some(verify.id.clone());
    db.save_sandbox_build(org, build).await?;
    client
        .wait_for_ready(&verify.id)
        .await
        .map_err(anyhow::Error::msg)?;
    if let Some(command) = build
        .request
        .verification_command
        .as_ref()
        .filter(|c| !c.trim().is_empty())
    {
        run_script(client, &verify.id, command, &workspace, &mut build.logs).await?;
    }
    if desktop(build) {
        use everruns_contracts::runtime::computer_use::DisplaySize;
        use everruns_integrations::daytona::computer::DaytonaDesktopSession;
        // The default and custom desktop image must actually start a display.
        let settings = &build.source_spec.target.options["desktop"];
        let display = DisplaySize {
            width: settings
                .get("display_width")
                .and_then(|v| v.as_u64())
                .unwrap_or(1280) as u32,
            height: settings
                .get("display_height")
                .and_then(|v| v.as_u64())
                .unwrap_or(800) as u32,
        };
        // Client contains the same host-owned token, never an in-sandbox value.
        DaytonaDesktopSession::start(
            DaytonaClient::new(provider_token.to_string()).with_download_limit(64 * 1024),
            verify.id,
            display,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        append_log(
            &mut build.logs,
            "\nDesktop startup passed on a fresh snapshot.\n",
        );
    }
    Ok(())
}

fn missing(error: &str) -> bool {
    error.contains("404") || error.contains("not found")
}

/// The provider accepts an immutable id or the unique name persisted before
/// submission. Avoid repeatedly scanning every snapshot in the shared account.
async fn lookup_snapshot(
    client: &DaytonaClient,
    id_or_name: &str,
) -> anyhow::Result<Option<everruns_integrations::daytona::state::SnapshotInfo>> {
    match client.get_snapshot(id_or_name).await {
        Ok(snapshot) => Ok(Some(snapshot)),
        Err(error) if missing(&error) => Ok(None),
        Err(error) => Err(anyhow::Error::msg(error)),
    }
}

async fn cleanup(client: &DaytonaClient, build: &mut SandboxTemplateBuild) -> anyhow::Result<()> {
    for id in [&mut build.builder_id, &mut build.verification_sandbox_id] {
        if let Some(value) = id.as_ref() {
            if let Err(error) = client.delete_sandbox(value).await
                && !missing(&error)
            {
                return Err(anyhow::Error::msg(error));
            }
            match client.get_sandbox(value).await {
                Err(error) if missing(&error) => {}
                Err(error) => return Err(anyhow::Error::msg(error)),
                Ok(_) => anyhow::bail!("Disposable sandbox deletion is still in progress"),
            }
            *id = None;
        }
    }
    if !matches!(
        build.status,
        SandboxBuildStatus::Ready | SandboxBuildStatus::Applied
    ) && build.snapshot_requested_at.is_some()
    {
        // Reconcile an uncertain snapshot submission by its unique, persisted name.
        let snapshot = lookup_snapshot(
            client,
            build.snapshot_id.as_deref().unwrap_or(&build.snapshot_name),
        )
        .await?;
        if let Some(snapshot) = snapshot {
            build.snapshot_id = Some(snapshot.id.clone());
            if let Err(error) = client.delete_snapshot(&snapshot.id).await
                && !missing(&error)
            {
                return Err(anyhow::Error::msg(error));
            }
            anyhow::bail!("Snapshot deletion is still being reconciled");
        } else if build.snapshot_id.is_none()
            && build
                .snapshot_requested_at
                .is_some_and(|at| chrono::Utc::now() - at < chrono::Duration::minutes(10))
        {
            anyhow::bail!("Waiting for uncertain snapshot submission to settle");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_scripts_remain_one_shell_argument() {
        assert_eq!(quote("a'b\n$(echo unsafe)"), "'a'\\''b\n$(echo unsafe)'");
        let command = script_command("exit 17");
        assert!(command.contains("tail -c 32768"));
        let result = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(17));
    }
}
