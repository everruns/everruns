use super::*;
use clap::Parser;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: K8sCommand,
}

#[test]
fn health_reads_liveness_and_readiness() {
    let live = Ok(serde_json::json!({"status": "ok", "version": "0.52.0"}));
    let ready = Ok(serde_json::json!({"status": "ready"}));
    assert_eq!(
        describe_health(&live, &ready),
        (true, "healthy, server 0.52.0".to_string())
    );
    let (good, line) = describe_health(&live, &Err("ServiceUnavailable".into()));
    assert!(!good && line.contains("not ready"), "{line}");
    let degraded = Ok(
        serde_json::json!({"status": "degraded", "version": "0.52.0",
                                         "event_delivery": "polling"}),
    );
    let (good, line) = describe_health(&degraded, &ready);
    assert!(!good && line.starts_with("degraded"), "{line}");
    let (good, line) = describe_health(&Err("no endpoints available".into()), &ready);
    assert!(
        !good && line.contains("not answering (no endpoints available)"),
        "{line}"
    );
}

#[test]
fn a_reinstall_after_uninstall_keeps_the_data_plane_it_ran() {
    use docker::DataPlaneChoice::{V1, V2};
    let record = |plane: Option<&str>| Record {
        context: "kind-kind".into(),
        namespace: "ns".into(),
        release: "demo".into(),
        chart: CHART_OCI.into(),
        port: DEFAULT_PORT,
        auth: None,
        admin_email: None,
        bundled_postgres: true,
        data_plane: plane.map(str::to_string),
    };
    let v1 = record(Some("v1"));
    // Uninstalled: no release values, the record brings v1 back.
    assert_eq!(data_plane_choice(None, None, Some(&v1)), Some(V1));
    // The flag wins over both.
    assert_eq!(data_plane_choice(Some(V2), Some(V1), Some(&v1)), Some(V2));
    // A live release keeps its own value; nothing to set.
    assert_eq!(data_plane_choice(None, Some(V1), Some(&v1)), None);
    // Nothing remembered: the chart default.
    assert_eq!(data_plane_choice(None, None, Some(&record(None))), None);
    assert_eq!(data_plane_choice(None, None, None), None);
}

#[test]
fn up_parses_every_flag() {
    let cli = Cli::try_parse_from([
        "k8s",
        "up",
        "--name",
        "demo",
        "--context",
        "kind-kind",
        "--namespace",
        "ns",
        "--auth",
        "single",
        "--admin-email",
        "a@b.c",
        "--bundled-postgres",
        "--public-url",
        "https://e.example.com",
        "--chart",
        "charts/everruns",
        "--set",
        "server.replicas=2",
        "--set",
        "valkey.enabled=true",
        "--yes",
    ])
    .unwrap();
    let K8sCommand::Up(args) = cli.command else {
        panic!("not up")
    };
    assert_eq!(args.instance.name, "demo");
    assert!(args.bundled_postgres);
    assert_eq!(args.set.len(), 2);
    assert_eq!(args.timeout, "15m");
}

#[test]
fn a_database_url_and_the_bundled_one_conflict() {
    assert!(
        Cli::try_parse_from([
            "k8s",
            "up",
            "--bundled-postgres",
            "--database-url",
            "postgres://x"
        ])
        .is_err()
    );
}

#[test]
fn install_names_are_release_names() {
    assert!(Instance::new("everruns").is_ok());
    assert!(Instance::new("team-a").is_ok());
    assert!(Instance::new("Team").is_err());
    assert!(Instance::new("-a").is_err());
    assert!(Instance::new("").is_err());
}

#[test]
fn profiles_are_named_after_the_install() {
    assert_eq!(Instance::new("everruns").unwrap().profile(), "k8s");
    assert_eq!(Instance::new("team-a").unwrap().profile(), "k8s-team-a");
}

#[test]
fn the_generated_secret_unless_one_is_managed() {
    let t = Target {
        context: "c".into(),
        namespace: "n".into(),
        release: "everruns".into(),
    };
    assert_eq!(secret_name(&t, &Value::Null), "everruns-generated");
    let v = serde_json::json!({ "secrets": { "existingSecret": "mine" } });
    assert_eq!(secret_name(&t, &v), "mine");
}

/// Every provider `up` offers lands in a chart value the chart turns into
/// the server's `DEFAULT_*_API_KEY`, and the server pod reads that key.
#[test]
fn every_offered_provider_reaches_the_server() {
    let chart = concat!(env!("CARGO_MANIFEST_DIR"), "/../../charts/everruns");
    let read = |f: &str| std::fs::read_to_string(format!("{chart}/{f}")).unwrap();
    let values: serde_yaml::Value = serde_yaml::from_str(&read("values.yaml")).unwrap();
    let secrets = read("templates/secrets.yaml");
    let server = read("templates/server.yaml");
    for provider in LlmProvider::ALL {
        let Some(key) = chart_provider_key(provider) else {
            assert_eq!(provider, LlmProvider::Skip);
            continue;
        };
        let env = provider.server_env().unwrap();
        assert!(
            values["providerKeys"].get(key).is_some(),
            "values.yaml has no providerKeys.{key}"
        );
        assert!(
            secrets.contains(&format!("{env}: {{{{ .{key} | quote }}}}")),
            "secrets.yaml does not map providerKeys.{key} to {env}"
        );
        assert!(
            server.contains(&format!("\"{env}\"")),
            "server.yaml does not read {env}"
        );
    }
}
