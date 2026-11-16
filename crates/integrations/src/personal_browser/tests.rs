use super::*;
const SERVER: &str = r#"
import json,os,sys,time
count=0
initialized=False
for line in sys.stdin:
 r=json.loads(line); method=r.get('method'); ident=r.get('id')
 if method=='notifications/initialized': initialized=True; continue
 if method=='initialize': result={'protocolVersion':'2025-11-25'}
 elif not initialized: sys.exit(2)
 else:
  count+=1
  name=r.get('params',{}).get('name')
  if name=='hang': time.sleep(20)
  if name=='exit': sys.exit(0)
  if name=='malformed': print('invalid',flush=True); continue
  if name=='oversize': print('x'*4200000,flush=True); continue
  if name=='remote_error':
   print(json.dumps({'jsonrpc':'2.0','id':ident,'error':{'code':-1,'message':'secret'}}),flush=True);continue
  if name=='handoff':
   args=r['params']['arguments']; outcome=args['message']
   result={'content':[{'type':'text','text':json.dumps({'outcome':outcome,'tab':args['tab']})}],'isError':outcome!='continued'}
  else: result={'pid':os.getpid(),'count':count,'meta':r.get('params',{}).get('_meta')}
 print(json.dumps({'jsonrpc':'2.0','id':ident,'result':result}),flush=True)
"#;
fn config() -> Option<BrowseProcessConfig> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("EVERRUNS_BROWSER_TEST_PYTHON") {
        candidates.push(PathBuf::from(path));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for name in ["python3", "python", "python3.exe", "python.exe"] {
                candidates.push(dir.join(name));
            }
        }
    }
    let python = candidates.into_iter().find(|path| {
        path.is_absolute()
            && path.is_file()
            && std::process::Command::new(path)
                .args(["-c", "import sys; sys.exit(sys.version_info.major != 3)"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
    });
    let Some(python) = python else {
        assert!(
            std::env::var_os("EVERRUNS_BROWSER_ACCEPTANCE_REQUIRED").is_none(),
            "Python3 required for personal browser subprocess tests"
        );
        eprintln!(
            "SKIP personal browser subprocess tests: Python3 unavailable; set EVERRUNS_BROWSER_TEST_PYTHON (required mode: EVERRUNS_BROWSER_ACCEPTANCE_REQUIRED=1)"
        );
        return None;
    };
    let mut config = BrowseProcessConfig::new(python);
    config.args = vec!["-u".into(), "-c".into(), SERVER.into()];
    config.request_timeout = Duration::from_secs(2);
    Some(config)
}
#[tokio::test]
async fn personal_browser_persistent_owner_and_distinct_sessions() {
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let Some(cfg) = config() else {
        return;
    };
    let one = BrowseSupervisor::start(a, cfg.clone()).await.unwrap();
    let two = BrowseSupervisor::start(b, cfg.clone()).await.unwrap();
    let first = one.list_tools(a).await.unwrap();
    let second = one.call_tool(a, "status", json!({})).await.unwrap();
    let other = two.list_tools(b).await.unwrap();
    assert_eq!(first["pid"], second["pid"]);
    assert_ne!(first["pid"], other["pid"]);
    assert_eq!(second["count"], 2);
    assert_eq!(second["meta"], Value::Null);
    assert_eq!(
        one.list_tools(b).await.unwrap_err(),
        BrowseError::WrongOwner
    );
    #[cfg(unix)]
    let pid = first["pid"].as_u64().unwrap();
    one.close().await;
    // close() awaits reap, not just a cancellation signal.
    #[cfg(unix)]
    let absent = Command::new(cfg.executable)
        .args(["-c", "import os,sys;\ntry: os.kill(int(sys.argv[1]),0)\nexcept ProcessLookupError: sys.exit(0)\nelse: sys.exit(1)", &pid.to_string()])
        .status().await.unwrap();
    #[cfg(unix)]
    assert!(absent.success(), "closed owner child still exists");
    assert!(two.list_tools(b).await.is_ok());
    two.close().await;
}
#[tokio::test]
async fn personal_browser_handoff_outcomes_are_not_approvals() {
    let owner = Uuid::new_v4();
    let Some(cfg) = config() else {
        return;
    };
    let session = BrowseSupervisor::start(owner, cfg).await.unwrap();
    for (message, expected) in [
        ("continued", HandoffOutcome::Continued),
        ("cancelled", HandoffOutcome::Cancelled),
        ("timeout", HandoffOutcome::TimedOut),
        ("closed", HandoffOutcome::TabClosed),
    ] {
        assert_eq!(
            session
                .handoff(owner, 42, message, Duration::from_secs(1))
                .await
                .unwrap(),
            expected
        );
    }
    assert_eq!(
        session
            .handoff(owner, 42, "invented", Duration::from_secs(1))
            .await
            .unwrap_err(),
        BrowseError::ProtocolError
    );
    session.close().await;
}
#[tokio::test]
async fn personal_browser_remote_errors_do_not_leak_or_restart() {
    let owner = Uuid::new_v4();
    let Some(cfg) = config() else {
        return;
    };
    let session = BrowseSupervisor::start(owner, cfg).await.unwrap();
    let first = session.list_tools(owner).await.unwrap();
    assert_eq!(
        session
            .call_tool(owner, "remote_error", json!({}))
            .await
            .unwrap_err(),
        BrowseError::RemoteError
    );
    assert_eq!(
        session.list_tools(owner).await.unwrap()["pid"],
        first["pid"]
    );
    session.close().await;
}
#[tokio::test]
async fn personal_browser_failure_is_terminal_no_replay() {
    for name in ["exit", "malformed", "oversize", "hang"] {
        let owner = Uuid::new_v4();
        let Some(mut cfg) = config() else {
            return;
        };
        cfg.request_timeout = Duration::from_secs(2);
        let session = BrowseSupervisor::start(owner, cfg).await.unwrap();
        assert!(session.call_tool(owner, name, json!({})).await.is_err());
        let mut state = session.watch_state();
        tokio::time::timeout(Duration::from_secs(3), async {
            while *state.borrow_and_update() == BrowseState::Ready {
                state.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(session.state(), BrowseState::Interrupted);
        assert_eq!(
            session.list_tools(owner).await.unwrap_err(),
            BrowseError::Interrupted
        );
        session.close().await;
    }
}
#[tokio::test]
async fn personal_browser_cancelled_inflight_request_interrupts_session() {
    let owner = Uuid::new_v4();
    let Some(cfg) = config() else {
        return;
    };
    let session = BrowseSupervisor::start(owner, cfg).await.unwrap();
    // Timeout drops the request future; its oneshot receiver closes.
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            session.call_tool(owner, "hang", json!({}))
        )
        .await
        .is_err()
    );
    let mut state = session.watch_state();
    tokio::time::timeout(Duration::from_secs(3), async {
        while *state.borrow_and_update() == BrowseState::Ready {
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(session.state(), BrowseState::Interrupted);
    session.close().await;
}
#[test]
fn personal_browser_config_redacts_secrets() {
    let Some(mut cfg) = config() else {
        return;
    };
    cfg.environment.insert("TOKEN".into(), "supersecret".into());
    assert!(!format!("{cfg:?}").contains("supersecret"));
    assert!(!format!("{cfg:?}").contains(SERVER));
}

/// Optional compatibility smoke against the host's verified pinned Browse
/// executable. This never installs/pairs an extension or calls a browser tool.
#[tokio::test]
async fn personal_browser_real_upstream_discovery() {
    let Some(path) = std::env::var_os("EVERRUNS_BROWSE_EXECUTABLE") else {
        assert!(
            std::env::var_os("EVERRUNS_BROWSE_SMOKE_REQUIRED").is_none(),
            "EVERRUNS_BROWSE_EXECUTABLE required for upstream compatibility smoke"
        );
        eprintln!(
            "SKIP real Browse discovery: set EVERRUNS_BROWSE_EXECUTABLE to host-verified pinned absolute path"
        );
        return;
    };
    let owner = Uuid::new_v4();
    let profile = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut cfg = BrowseProcessConfig::new(path);
    for name in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_RUNTIME_DIR",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
    ] {
        cfg.environment
            .insert(name.into(), profile.path().to_string_lossy().into_owned());
    }
    cfg.environment
        .insert("OPENCODE_BROWSER_AUTOSTART".into(), "0".into());
    cfg.environment
        .insert("OPENCODE_BROWSER_PORT".into(), port.to_string());
    let session = BrowseSupervisor::start(owner, cfg).await.unwrap();
    let discovered = session.list_tools(owner).await.unwrap();
    let tools = discovered["tools"]
        .as_array()
        .expect("upstream tools array");
    let handoff = tools
        .iter()
        .find(|tool| tool["name"] == "handoff")
        .expect("upstream handoff");
    for field in ["tab", "message", "timeoutSeconds"] {
        assert!(
            handoff["inputSchema"]["properties"].get(field).is_some(),
            "missing handoff field {field}"
        );
    }
    assert!(tools.iter().any(|tool| tool["name"] == "status"));
    session.close().await;
}
