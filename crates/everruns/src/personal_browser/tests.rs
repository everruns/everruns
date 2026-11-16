use super::*;
use crate::hosting::{AgentBuild, MemoryHosting, SessionHosting};
use crate::{Agent, LlmSimConfig, Model};
const SERVER: &str = r#"
import json,os,sys,time
for line in sys.stdin:
 r=json.loads(line); method=r.get('method'); ident=r.get('id')
 if ident is None: continue
 if method=='initialize': result={'protocolVersion':'2025-11-25'}
 elif method=='tools/list': result={'tools':[]}
 else:
  name=r['params']['name']; args=r['params']['arguments']
  if name=='status': value={'connected':True,'secret':'never-forward','nextStep':'token=secret'}
  elif name=='tabs': value={'tabs':[{'tab':os.getpid(),'title':'Fixture','url':'https://example.test','secret':'never-forward'}]}
  elif name in ('navigate','snapshot','click','fill','press','scroll'):
   allowed={'navigate':{'tab','url'},'snapshot':{'tab'},'click':{'tab','ref'},'fill':{'tab','ref','value'},'press':{'tab','ref','key'},'scroll':{'tab','deltaX','deltaY'}}
   assert set(args)<=allowed[name]
   value={'action':name,'args':args}
  elif name=='screenshot':
   assert set(args)=={'tab'}
   data='aGVsbG8=' if args['tab']!=999 else 'a'*1200000
   result={'content':[{'type':'image','mimeType':'image/png','data':data}],'isError':False}
   print(json.dumps({'jsonrpc':'2.0','id':ident,'result':result}),flush=True);continue
  elif name=='handoff':
   if args['message']=='wait': time.sleep(20)
   value={'tab':args['tab'],'outcome':args['message'] if args['message'] in ('continued','cancelled','timeout','closed') else 'continued'}
  else: sys.exit(3)
  result={'content':[{'type':'text','text':json.dumps(value)}],'isError':value.get('outcome','continued')!='continued'}
 print(json.dumps({'jsonrpc':'2.0','id':ident,'result':result}),flush=True)
"#;
fn config() -> Option<BrowseProcessConfig> {
    let mut candidates = Vec::new();
    if let Some(value) = std::env::var_os("EVERRUNS_BROWSER_TEST_PYTHON") {
        candidates.push(std::path::PathBuf::from(value));
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
                .args(["-c", "import sys;sys.exit(sys.version_info.major!=3)"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
    });
    let Some(python) = python else {
        assert!(
            std::env::var_os("EVERRUNS_BROWSER_ACCEPTANCE_REQUIRED").is_none(),
            "Python3 required"
        );
        eprintln!("SKIP personal browser coordinator subprocess tests: Python3 unavailable");
        return None;
    };
    let mut cfg = BrowseProcessConfig::new(python);
    cfg.args = vec!["-u".into(), "-c".into(), SERVER.into()];
    Some(cfg)
}
fn build() -> AgentBuild {
    AgentBuild::new("browser-test", "v1")
}
fn agent(call: ToolCall) -> Agent {
    Agent::builder()
        .instructions("Test personal browser host tools.")
        .model(Model::simulated_with_config(
            LlmSimConfig::fixed("Done").with_tool_call_sequence(vec![vec![call], vec![]]),
        ))
        .build()
        .unwrap()
}
fn engine(profile: impl SessionHosting, agent: Agent) -> Engine {
    Engine::builder()
        .hosting(profile)
        .register(build(), agent)
        .build()
}
async fn wait_pending(engine: &Engine, id: SessionId) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = engine.session_status(id).await.unwrap();
            if !status.waiting.is_empty() && !status.executing {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn durable_completion(profile: impl SessionHosting + Clone) {
    let Some(cfg) = config() else {
        return;
    };
    let call = ToolCall {
        id: "browser-call".into(),
        name: STATUS_TOOL.into(),
        arguments: json!({}),
    };
    let engine = engine(profile.clone(), agent(call));
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    let worker = PersonalBrowserCoordinator::new(move |_| cfg.clone(), 2).unwrap();
    worker.enable_session(&engine, id).await.unwrap();
    let child = SessionId::new();
    let child_session = engine.create_registered(child, &build()).await.unwrap();
    assert!(
        !child_session
            .inspect()
            .await
            .unwrap()
            .tools
            .iter()
            .any(|tool| owns(&tool.name)),
        "session tools leaked into child Agent binding"
    );
    let admission = session.send("check browser").await.unwrap();
    wait_pending(&engine, id).await;
    let turn = engine.session_status(id).await.unwrap().open_turn.unwrap();
    assert!(
        !engine.session_status(id).await.unwrap().executing,
        "agent worker retained while client tool parked"
    );
    let report = worker.run_pending(&engine, id).await.unwrap();
    assert_eq!(report.completed, 1);
    tokio::time::timeout(Duration::from_secs(5), admission.wait())
        .await
        .unwrap()
        .unwrap();
    let value = profile
        .command(
            id,
            SessionCommand::InspectClientDispatch {
                turn,
                call_id: "browser-call".into(),
            },
        )
        .await
        .unwrap();
    let record: ClientDispatchRecord = serde_json::from_value(value).unwrap();
    assert_eq!(record.state, ClientDispatchState::Completed);
    assert!(
        !serde_json::to_string(&record.result)
            .unwrap()
            .contains("never-forward")
    );
    assert_eq!(worker.run_pending(&engine, id).await.unwrap().completed, 0);
    worker.close_session(&engine, id).await.unwrap();
}
#[tokio::test]
async fn personal_browser_durable_memory_completion_and_session_only_tools() {
    durable_completion(MemoryHosting::default()).await;
}
#[cfg(feature = "local")]
#[tokio::test]
async fn personal_browser_durable_sqlite_completion() {
    let dir = tempfile::tempdir().unwrap();
    durable_completion(
        crate::hosting::SqliteHosting::open(dir.path().join("browser.sqlite")).unwrap(),
    )
    .await;
}
#[tokio::test]
async fn personal_browser_close_interrupts_handoff_without_waiting_for_gate() {
    let Some(cfg) = config() else {
        return;
    };
    let profile = MemoryHosting::default();
    let call = ToolCall {
        id: "handoff".into(),
        name: HANDOFF_TOOL.into(),
        arguments: json!({"tab":42,"message":"wait","timeoutSeconds":600}),
    };
    let engine = engine(profile.clone(), agent(call));
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    let worker = PersonalBrowserCoordinator::new(move |_| cfg.clone(), 1).unwrap();
    worker.enable_session(&engine, id).await.unwrap();
    let _admission = session.send("handoff").await.unwrap();
    wait_pending(&engine, id).await;
    let turn = engine.session_status(id).await.unwrap().open_turn.unwrap();
    let work = {
        let worker = worker.clone();
        let engine = engine.clone();
        tokio::spawn(async move { worker.run_pending(&engine, id).await })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let value = profile
                .command(
                    id,
                    SessionCommand::InspectClientDispatch {
                        turn,
                        call_id: "handoff".into(),
                    },
                )
                .await
                .unwrap();
            if !value.is_null() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), worker.close_session(&engine, id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(work.await.unwrap().unwrap().interrupted, 1);
    let record: ClientDispatchRecord = serde_json::from_value(
        profile
            .command(
                id,
                SessionCommand::InspectClientDispatch {
                    turn,
                    call_id: "handoff".into(),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(record.state, ClientDispatchState::Interrupted);
}
#[tokio::test]
async fn personal_browser_recovery_does_not_steal_live_claim_or_replay() {
    let Some(cfg) = config() else {
        return;
    };
    let profile = MemoryHosting::default();
    let call = ToolCall {
        id: "claimed".into(),
        name: STATUS_TOOL.into(),
        arguments: json!({}),
    };
    let engine = engine(profile.clone(), agent(call.clone()));
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    let worker = PersonalBrowserCoordinator::new(move |_| cfg.clone(), 1).unwrap();
    worker.enable_session(&engine, id).await.unwrap();
    let _admission = session.send("status").await.unwrap();
    wait_pending(&engine, id).await;
    let status = engine.session_status(id).await.unwrap();
    let turn = status.open_turn.unwrap();
    let owner = Uuid::new_v4();
    let resolution_id = Uuid::new_v4();
    let fence = Uuid::new_v4();
    let deadline = status.now_ms + 1000;
    profile
        .command(
            id,
            SessionCommand::BeginClientDispatch {
                id: fence.to_string(),
                turn,
                call,
                owner,
                resolution_id,
                deadline_ms: deadline,
            },
        )
        .await
        .unwrap();
    let report = worker.run_pending(&engine, id).await.unwrap();
    assert_eq!(report.completed, 0);
    assert_eq!(report.interrupted, 0);
    assert_eq!(report.next_check_at_ms, Some(deadline));
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let report = worker.run_pending(&engine, id).await.unwrap();
    assert_eq!(report.completed, 0);
    assert_eq!(report.interrupted, 1);
    worker.close_session(&engine, id).await.unwrap();
}
#[tokio::test]
async fn personal_browser_conflicting_declaration_is_not_overwritten() {
    let profile = MemoryHosting::default();
    let engine = engine(
        profile.clone(),
        Agent::builder()
            .instructions("Test personal browser host tools.")
            .model(Model::simulated("done"))
            .client_tool(STATUS_TOOL, "Unrelated host tool", json!({"type":"object"}))
            .build()
            .unwrap(),
    );
    let id = SessionId::new();
    engine.create_registered(id, &build()).await.unwrap();
    assert_eq!(
        configure(&engine, id, true).await.unwrap_err(),
        CoordinatorError::InvalidConfiguration
    );
    assert_eq!(
        configure(&engine, id, false).await.unwrap_err(),
        CoordinatorError::InvalidConfiguration
    );
}

#[path = "tests/actions.rs"]
mod action_tests;
mod lifecycle;
