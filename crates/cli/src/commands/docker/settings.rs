//! The settings folder of one local instance: its `.env`, its compose file and
//! its lock.
//!
//! Decision: everything `up` decides lives in the instance's `.env`, the file
//! Compose already reads, so there is one record of the setup and a person can
//! read or edit it. Values are written once and never regenerated: the
//! encryption key in particular protects stored secrets, and replacing it would
//! make them unreadable.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use rand::RngExt;

/// The compose file this CLI release runs: a copy of
/// [the documented example](https://github.com/everruns/everruns/blob/main/examples/docker-compose-full.yaml),
/// kept identical by a test so the guide
/// and the CLI start the same, tested stack.
pub const COMPOSE_FILE: &str = include_str!("../../../assets/docker-compose.yaml");

pub const AUTH_MODE: &str = "AUTH_MODE";
pub const ADMIN_EMAIL: &str = "AUTH_ADMIN_EMAIL";
pub const ADMIN_PASSWORD: &str = "AUTH_ADMIN_PASSWORD";
pub const JWT_SECRET: &str = "AUTH_JWT_SECRET";
pub const ENCRYPTION_KEY: &str = "SECRETS_ENCRYPTION_KEY";
pub const WORKER_TOKEN: &str = "WORKER_GRPC_AUTH_TOKEN";
pub const PORT: &str = "EXAMPLE_PROXY_PORT";
pub const BIND: &str = "EXAMPLE_PROXY_BIND";
pub const IMAGE_TAG: &str = "EVERRUNS_TAG";
pub const DEPLOYMENT_GRADE: &str = "DEPLOYMENT_GRADE";
pub const SEED_PROVIDER_KEYS: &str = "SEED_DEFAULT_PROVIDER_KEYS_FROM_ENV";
/// Which provider question was answered, so "skip" is not asked again.
pub const LLM_PROVIDER: &str = "EVERRUNS_LLM_PROVIDER";

/// Every variable the compose file interpolates that `up` manages. A value of
/// the same name in the caller's shell would override `.env` inside Compose,
/// so commands run with these removed from their environment.
pub const MANAGED_KEYS: &[&str] = &[
    AUTH_MODE,
    ADMIN_EMAIL,
    ADMIN_PASSWORD,
    JWT_SECRET,
    ENCRYPTION_KEY,
    WORKER_TOKEN,
    PORT,
    BIND,
    IMAGE_TAG,
    DEPLOYMENT_GRADE,
    SEED_PROVIDER_KEYS,
    "DEFAULT_OPENAI_API_KEY",
    "DEFAULT_ANTHROPIC_API_KEY",
    "DEFAULT_GEMINI_API_KEY",
    "COMPOSE_PROFILES",
    "VICTORIAMETRICS_PORT",
];

/// One local instance, named by `--name`.
pub struct Instance {
    pub name: String,
    pub dir: PathBuf,
}

impl Instance {
    pub fn new(name: &str) -> Result<Self> {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            bail!("instance name must be lowercase letters, digits and '-', got '{name}'");
        }
        let base = crate::user_dirs::data_dir()
            .ok_or_else(|| anyhow!("Could not determine the data directory"))?;
        Ok(Self {
            name: name.to_string(),
            dir: base.join("everruns").join("docker").join(name),
        })
    }

    /// Compose project name, which also names the containers and volumes.
    pub fn project(&self) -> String {
        if self.name == "default" {
            "everruns".to_string()
        } else {
            format!("everruns-{}", self.name)
        }
    }

    /// CLI profile that points at this instance.
    pub fn profile(&self) -> String {
        if self.name == "default" {
            "local".to_string()
        } else {
            format!("local-{}", self.name)
        }
    }

    pub fn env_path(&self) -> PathBuf {
        self.dir.join(".env")
    }

    pub fn compose_path(&self) -> PathBuf {
        self.dir.join("compose.yaml")
    }

    pub fn exists(&self) -> bool {
        self.env_path().exists()
    }

    /// Create the folder, private to the user.
    pub fn ensure_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.dir)
            .with_context(|| format!("Failed to create {}", self.dir.display()))?;
        set_mode(&self.dir, 0o700)
    }

    /// Hold the instance's lock until the returned file drops, so two `up`
    /// runs at the same time take turns instead of racing.
    pub fn lock(&self) -> Result<File> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join(".lock"))
            .context("Failed to open the instance lock")?;
        if file.try_lock().is_err() {
            eprintln!("Waiting for another `everruns docker` command on this instance...");
            file.lock().context("Failed to lock the instance")?;
        }
        Ok(file)
    }

    /// Write the embedded compose file when it differs from what is on disk.
    /// Returns whether it changed.
    pub fn write_compose(&self) -> Result<bool> {
        let path = self.compose_path();
        if fs::read_to_string(&path).ok().as_deref() == Some(COMPOSE_FILE) {
            return Ok(false);
        }
        fs::write(&path, COMPOSE_FILE)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(true)
    }
}

/// The instance's `.env`, in file order so a rewrite keeps a person's edits
/// and comments where they put them.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Settings {
    lines: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq)]
enum Line {
    Pair(String, String),
    Other(String),
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("Failed to read {}", path.display())),
        }
    }

    pub fn parse(text: &str) -> Self {
        let lines = text
            .lines()
            .map(|line| {
                let trimmed = line.trim_start();
                match trimmed.split_once('=') {
                    Some((key, value)) if !trimmed.starts_with('#') && !key.trim().is_empty() => {
                        Line::Pair(key.trim().to_string(), value.to_string())
                    }
                    _ => Line::Other(line.to_string()),
                }
            })
            .collect();
        Self { lines }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                Line::Pair(k, v) => out.push_str(&format!("{k}={v}\n")),
                Line::Other(text) => {
                    out.push_str(text);
                    out.push('\n');
                }
            }
        }
        out
    }

    /// Write with mode 0600: the file holds the encryption key and password.
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        set_mode(path, 0o600)?;
        file.write_all(self.render().as_bytes())?;
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        !self.lines.iter().any(|l| matches!(l, Line::Pair(..)))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.lines.iter().rev().find_map(|l| match l {
            Line::Pair(k, v) if k == key => Some(v.as_str()),
            _ => None,
        })
    }

    /// A saved, non-empty value.
    pub fn value(&self, key: &str) -> Option<&str> {
        self.get(key).filter(|v| !v.is_empty())
    }

    /// Set a value. Returns whether it changed.
    pub fn set(&mut self, key: &str, value: &str) -> bool {
        if self.get(key) == Some(value) {
            return false;
        }
        for line in &mut self.lines {
            if let Line::Pair(k, v) = line
                && k == key
            {
                *v = value.to_string();
                return true;
            }
        }
        self.lines
            .push(Line::Pair(key.to_string(), value.to_string()));
        true
    }

    /// Set a value only when none is saved. Returns whether it was set.
    pub fn set_missing(&mut self, key: &str, make: impl FnOnce() -> String) -> bool {
        if self.value(key).is_some() {
            return false;
        }
        self.set(key, &make())
    }

    /// Generate the secrets that are missing, leaving existing ones alone.
    pub fn fill_secrets(&mut self) {
        self.set_missing(ENCRYPTION_KEY, encryption_key);
        self.set_missing(JWT_SECRET, || random_hex(32));
        self.set_missing(WORKER_TOKEN, || random_hex(32));
    }
}

/// The server's key format: `kek-v1:` and 32 random bytes in base64.
pub fn encryption_key() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    format!(
        "kek-v1:{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

pub fn random_hex(bytes: usize) -> String {
    let mut rng = rand::rng();
    (0..bytes)
        .map(|_| format!("{:02x}", rng.random::<u8>()))
        .collect()
}

/// 24 letters and digits with at least one digit, which satisfies the
/// server's password rules for registered accounts too.
pub fn password() -> String {
    const ALPHABET: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::rng();
    loop {
        let candidate: String = (0..24)
            .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
            .collect();
        if candidate.chars().any(|c| c.is_ascii_digit()) {
            return candidate;
        }
    }
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .with_context(|| format!("Failed to set permissions on {}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_compose_file_matches_the_documented_one() {
        // Only checkable inside the repository; a packaged crate has no examples/.
        let documented = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/docker-compose-full.yaml"
        );
        let Ok(text) = fs::read_to_string(documented) else {
            return;
        };
        assert_eq!(
            text, COMPOSE_FILE,
            "crates/cli/assets/docker-compose.yaml must stay a copy of \
             examples/docker-compose-full.yaml: cp examples/docker-compose-full.yaml \
             crates/cli/assets/docker-compose.yaml"
        );
    }

    #[test]
    fn compose_file_interpolates_every_managed_key_it_needs() {
        for key in [
            AUTH_MODE,
            ADMIN_EMAIL,
            ADMIN_PASSWORD,
            JWT_SECRET,
            ENCRYPTION_KEY,
            WORKER_TOKEN,
            PORT,
            BIND,
            IMAGE_TAG,
        ] {
            assert!(
                COMPOSE_FILE.contains(&format!("${{{key}")),
                "compose file no longer reads {key}"
            );
        }
    }

    #[test]
    fn settings_round_trip_keeps_comments_and_order() {
        let text = "# mine\nB=2\nA=1\n\n";
        let mut s = Settings::parse(text);
        assert_eq!(s.render(), "# mine\nB=2\nA=1\n\n");
        assert!(s.set("A", "3"));
        assert!(!s.set("A", "3"));
        assert!(s.set("C", "x=y"));
        assert_eq!(s.render(), "# mine\nB=2\nA=3\n\nC=x=y\n");
        assert_eq!(Settings::parse(&s.render()).get("C"), Some("x=y"));
    }

    #[test]
    fn secrets_are_generated_once() {
        let mut s = Settings::default();
        assert!(s.is_empty());
        s.fill_secrets();
        let first = s.render();
        assert!(s.value(ENCRYPTION_KEY).unwrap().starts_with("kek-v1:"));
        assert_eq!(s.value(JWT_SECRET).unwrap().len(), 64);
        s.fill_secrets();
        assert_eq!(s.render(), first, "a second fill must not replace anything");
    }

    #[test]
    fn empty_value_counts_as_missing() {
        let mut s = Settings::parse("SECRETS_ENCRYPTION_KEY=\n");
        assert!(s.value(ENCRYPTION_KEY).is_none());
        s.fill_secrets();
        assert!(s.value(ENCRYPTION_KEY).is_some());
    }

    #[test]
    fn encryption_key_decodes_to_32_bytes() {
        let key = encryption_key();
        let raw = base64::engine::general_purpose::STANDARD
            .decode(key.strip_prefix("kek-v1:").unwrap())
            .unwrap();
        assert_eq!(raw.len(), 32);
    }

    #[test]
    fn password_meets_the_server_rules() {
        for _ in 0..50 {
            let p = password();
            assert!(
                p.len() >= 12 && p.chars().any(|c| c.is_ascii_digit()),
                "{p}"
            );
        }
    }

    #[test]
    fn saved_file_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let mut s = Settings::default();
        s.set("A", "1");
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), s);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn instance_names_are_restricted() {
        assert!(Instance::new("default").is_ok());
        assert!(Instance::new("team-2").is_ok());
        for bad in ["", "Bad", "a/b", "../x", "a b"] {
            assert!(Instance::new(bad).is_err(), "{bad}");
        }
        let i = Instance::new("default").unwrap();
        assert_eq!(
            (i.project().as_str(), i.profile().as_str()),
            ("everruns", "local")
        );
        let j = Instance::new("demo").unwrap();
        assert_eq!(
            (j.project().as_str(), j.profile().as_str()),
            ("everruns-demo", "local-demo")
        );
    }
}
