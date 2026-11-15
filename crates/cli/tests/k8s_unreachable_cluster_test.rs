#![cfg(target_os = "linux")]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! `everruns k8s destroy` and `status` for an install whose cluster does not answer, run as
//! the built binary with stand-in `kubectl` and `helm` scripts
//! (`EVERRUNS_KUBECTL`, `EVERRUNS_HELM`) and a throwaway home.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

struct Machine {
    home: tempfile::TempDir,
}

impl Machine {
    /// A machine where `everruns k8s up` once installed 'everruns' into
    /// colima-k8s, with one database backup and the CLI profile 'k8s', and
    /// whose cluster now refuses connections, as a stopped Colima does.
    fn with_stopped_cluster() -> Self {
        let home = tempfile::tempdir().unwrap();
        let machine = Self { home };
        let dir = machine.install_dir();
        std::fs::create_dir_all(dir.join("backups")).unwrap();
        std::fs::write(
            dir.join("instance.json"),
            r#"{"context":"colima-k8s","namespace":"everruns","release":"everruns",
                "chart":"oci://ghcr.io/everruns/charts/everruns","port":9300,
                "bundled_postgres":true}"#,
        )
        .unwrap();
        std::fs::write(dir.join("backups").join("everruns-0.52.0.sql.gz"), "x").unwrap();
        let config = machine.home.path().join("config").join("everruns");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("credentials.json"),
            r#"{"profiles":{"k8s":{"api_url":"http://localhost:9300","api_key":"k"}},
                "current_profile":"k8s"}"#,
        )
        .unwrap();
        let refused = "echo 'The connection to the server 127.0.0.1:58531 was refused - did you \
                       specify the right host or port?' >&2; exit 1";
        machine.tool("kubectl", refused);
        machine.tool("helm", "exit 1");
        machine
    }

    /// A stand-in tool that notes each call, then runs `body`.
    fn tool(&self, name: &str, body: &str) {
        let path = self.home.path().join(name);
        let log = self.home.path().join(format!("{name}.calls"));
        std::fs::write(
            &path,
            format!("#!/bin/sh\necho \"$@\" >> '{}'\n{body}\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn calls(&self, name: &str) -> String {
        std::fs::read_to_string(self.home.path().join(format!("{name}.calls"))).unwrap_or_default()
    }

    fn install_dir(&self) -> PathBuf {
        self.home
            .path()
            .join("data")
            .join("everruns")
            .join("k8s")
            .join("everruns")
    }

    fn profiles(&self) -> String {
        std::fs::read_to_string(
            self.home
                .path()
                .join("config")
                .join("everruns")
                .join("credentials.json"),
        )
        .unwrap()
    }

    fn run(&self, args: &[&str]) -> (Output, String) {
        let mut all = vec!["destroy"];
        all.extend_from_slice(args);
        self.k8s(&all)
    }

    fn k8s(&self, args: &[&str]) -> (Output, String) {
        let home: &Path = self.home.path();
        let out = Command::new(env!("CARGO_BIN_EXE_everruns"))
            .arg("k8s")
            .args(args)
            .env("HOME", home)
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("EVERRUNS_KUBECTL", home.join("kubectl"))
            .env("EVERRUNS_HELM", home.join("helm"))
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out, text)
    }
}

#[test]
fn a_cluster_that_does_not_answer_stops_destroy_before_anything_is_deleted() {
    let machine = Machine::with_stopped_cluster();
    for args in [&[][..], &["--yes"][..]] {
        let (out, text) = machine.run(args);
        assert!(!out.status.success(), "{text}");
        assert!(
            text.contains("Cluster colima-k8s does not answer"),
            "{text}"
        );
        assert!(text.contains("was refused"), "{text}");
        assert!(text.contains("everruns k8s destroy --local-only"), "{text}");
        assert!(text.contains("1 database backup"), "{text}");
        // Not asked to type the name, and nothing past the check ran.
        assert!(!text.contains("no terminal to ask on"), "{text}");
        assert_eq!(machine.calls("helm"), "", "{text}");
        assert!(!machine.calls("kubectl").contains("delete"), "{text}");
    }
    assert!(machine.install_dir().join("instance.json").exists());
    assert!(machine.profiles().contains("\"k8s\""));
}

#[test]
fn local_only_forgets_the_install_without_touching_the_cluster() {
    let machine = Machine::with_stopped_cluster();
    let (out, text) = machine.run(&["--local-only"]);
    assert!(!out.status.success(), "asks first: {text}");
    assert!(text.contains("pass --yes"), "{text}");
    assert!(machine.install_dir().exists());

    let (out, text) = machine.run(&["--local-only", "--yes"]);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("Removed CLI profile 'k8s'"), "{text}");
    assert!(!machine.install_dir().exists(), "{text}");
    assert!(!machine.profiles().contains("\"k8s\""));
    assert_eq!(machine.calls("kubectl"), "", "{text}");
    assert_eq!(machine.calls("helm"), "", "{text}");
}

#[test]
fn local_only_and_keep_namespace_do_not_mix() {
    let machine = Machine::with_stopped_cluster();
    let (out, text) = machine.run(&["--local-only", "--keep-namespace", "--yes"]);
    assert!(!out.status.success(), "{text}");
    assert!(machine.install_dir().exists());
}

#[test]
fn status_says_the_cluster_does_not_answer() {
    let machine = Machine::with_stopped_cluster();
    let (out, text) = machine.k8s(&["status"]);
    assert!(!out.status.success(), "{text}");
    assert!(
        text.contains("Cluster colima-k8s does not answer"),
        "{text}"
    );
    assert_eq!(machine.calls("helm"), "", "{text}");
}
