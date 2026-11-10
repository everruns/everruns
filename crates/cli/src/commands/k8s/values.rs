//! Helm values `up` applies: the release's current values with the person's
//! answers laid over them.
//!
//! Decision: the release is the record of the setup. Helm keeps every value
//! the chart was installed with (the database URL and provider keys included,
//! in its release Secret), so `up` reads them back with `helm get values`
//! instead of keeping a second copy on the laptop, and only asks what the
//! release does not already answer. Values someone set with `--set` stay.

use serde_json::{Map, Value, json};

use crate::commands::docker::AuthChoice;

/// Where the database comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum Database {
    /// A single PostgreSQL pod from the chart, for trying it out.
    Bundled,
    /// The person's own PostgreSQL.
    Url(String),
}

/// What `up` decided this run. `None` keeps what the release has.
#[derive(Debug, Default)]
pub struct Answers {
    pub auth: Option<AuthChoice>,
    pub admin_email: Option<String>,
    pub database: Option<Database>,
    pub public_url: Option<String>,
    pub ingress_class: Option<String>,
    /// Chart key under `providerKeys` (openai, anthropic, gemini, openrouter) and the key.
    pub provider_key: Option<(&'static str, String)>,
    pub image_tag: Option<String>,
}

/// What the release already answers.
pub struct Known<'a>(pub &'a Value);

impl Known<'_> {
    fn get(&self, path: &[&str]) -> Option<&Value> {
        path.iter().try_fold(self.0, |v, key| v.get(key))
    }

    fn str(&self, path: &[&str]) -> Option<&str> {
        self.get(path)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    }

    pub fn auth(&self) -> Option<AuthChoice> {
        self.str(&["auth", "mode"])
            .and_then(AuthChoice::from_server_mode)
    }

    pub fn admin_email(&self) -> Option<&str> {
        self.str(&["auth", "adminEmail"])
    }

    pub fn has_database(&self) -> bool {
        self.get(&["postgresql", "enabled"])
            .and_then(Value::as_bool)
            == Some(true)
            || self.str(&["database", "url"]).is_some()
            || self.str(&["database", "existingSecret"]).is_some()
    }

    pub fn bundled_postgres(&self) -> bool {
        self.get(&["postgresql", "enabled"])
            .and_then(Value::as_bool)
            == Some(true)
    }

    pub fn public_url(&self) -> Option<&str> {
        self.str(&["publicUrl"])
    }

    pub fn autoscaling(&self) -> bool {
        self.get(&["worker", "autoscaling", "enabled"])
            .and_then(Value::as_bool)
            == Some(true)
    }
}

/// `current` with `answers` applied. `current` is what `helm get values`
/// returned (null for a new release).
pub fn build(current: &Value, answers: &Answers) -> Value {
    let mut v = match current {
        Value::Object(map) => Value::Object(map.clone()),
        _ => Value::Object(Map::new()),
    };

    if let Some(auth) = answers.auth {
        set(&mut v, &["auth", "mode"], json!(auth.server_mode()));
        // The server refuses no sign-in outside the dev grade. Going back to
        // sign-in keeps whatever grade the person chose.
        if auth == AuthChoice::None {
            set(&mut v, &["deploymentGrade"], json!("dev"));
        }
        // Shared default keys only when nobody can sign themselves up.
        set(
            &mut v,
            &["seedDefaultProviderKeys"],
            json!(auth.shares_default_keys()),
        );
    }
    if let Some(email) = &answers.admin_email {
        set(&mut v, &["auth", "adminEmail"], json!(email));
    }
    match &answers.database {
        Some(Database::Bundled) => {
            set(&mut v, &["postgresql", "enabled"], json!(true));
            remove(&mut v, &["database", "url"]);
        }
        Some(Database::Url(url)) => {
            set(&mut v, &["postgresql", "enabled"], json!(false));
            set(&mut v, &["database", "url"], json!(url));
        }
        None => {}
    }
    if let Some(url) = &answers.public_url {
        let url = url.trim_end_matches('/');
        set(&mut v, &["publicUrl"], json!(url));
        set(&mut v, &["ingress", "enabled"], json!(true));
        set(&mut v, &["ingress", "host"], json!(host_of(url)));
    }
    if let Some(class) = &answers.ingress_class {
        set(&mut v, &["ingress", "className"], json!(class));
    }
    if let Some((provider, key)) = &answers.provider_key {
        set(&mut v, &["providerKeys", provider], json!(key));
    }
    if let Some(tag) = &answers.image_tag {
        set(&mut v, &["image", "tag"], json!(tag));
    }
    v
}

/// The host of `https://host[:port]/path`.
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    host.rsplit_once(':')
        .filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()))
        .map_or(host, |(h, _)| h)
        .to_string()
}

fn set(v: &mut Value, path: &[&str], value: Value) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut node = v;
    for key in parents {
        if !node.get(key).is_some_and(Value::is_object) {
            node[*key] = Value::Object(Map::new());
        }
        node = &mut node[*key];
    }
    node[*last] = value;
}

fn remove(v: &mut Value, path: &[&str]) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut node = v;
    for key in parents {
        match node.get_mut(key) {
            Some(next) => node = next,
            None => return,
        }
    }
    if let Some(map) = node.as_object_mut() {
        map.remove(*last);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_install_gets_every_answer() {
        let answers = Answers {
            auth: Some(AuthChoice::Single),
            admin_email: Some("me@example.com".into()),
            database: Some(Database::Bundled),
            public_url: Some("https://everruns.example.com/".into()),
            ingress_class: Some("nginx".into()),
            provider_key: Some(("openai", "sk-test".into())),
            image_tag: None,
        };
        let v = build(&Value::Null, &answers);
        assert_eq!(v["auth"]["mode"], "admin");
        assert_eq!(v["auth"]["adminEmail"], "me@example.com");
        assert_eq!(v["seedDefaultProviderKeys"], true);
        assert_eq!(v["postgresql"]["enabled"], true);
        assert_eq!(v["publicUrl"], "https://everruns.example.com");
        assert_eq!(v["ingress"]["host"], "everruns.example.com");
        assert_eq!(v["ingress"]["className"], "nginx");
        assert_eq!(v["providerKeys"]["openai"], "sk-test");
        assert!(v.get("deploymentGrade").is_none());
        assert!(v.get("image").is_none());
    }

    #[test]
    fn a_rerun_keeps_what_the_release_has() {
        let current = json!({
            "auth": { "mode": "full", "adminEmail": "a@b.c" },
            "database": { "url": "postgres://x" },
            "server": { "replicas": 2 },
            "valkey": { "enabled": true },
        });
        let v = build(&current, &Answers::default());
        assert_eq!(v, current);
        let known = Known(&current);
        assert_eq!(known.auth(), Some(AuthChoice::Accounts));
        assert_eq!(known.admin_email(), Some("a@b.c"));
        assert!(known.has_database());
        assert!(!known.bundled_postgres());
    }

    #[test]
    fn accounts_never_share_default_keys() {
        let v = build(
            &Value::Null,
            &Answers {
                auth: Some(AuthChoice::Accounts),
                ..Answers::default()
            },
        );
        assert_eq!(v["auth"]["mode"], "full");
        assert_eq!(v["seedDefaultProviderKeys"], false);
    }

    #[test]
    fn no_sign_in_runs_as_dev() {
        let v = build(
            &Value::Null,
            &Answers {
                auth: Some(AuthChoice::None),
                ..Answers::default()
            },
        );
        assert_eq!(v["deploymentGrade"], "dev");
    }

    #[test]
    fn switching_to_an_external_database_drops_the_bundled_one() {
        let current = json!({ "postgresql": { "enabled": true } });
        let v = build(
            &current,
            &Answers {
                database: Some(Database::Url("postgres://db/e".into())),
                ..Answers::default()
            },
        );
        assert_eq!(v["postgresql"]["enabled"], false);
        assert_eq!(v["database"]["url"], "postgres://db/e");
        let back = build(
            &v,
            &Answers {
                database: Some(Database::Bundled),
                ..Answers::default()
            },
        );
        assert_eq!(back["postgresql"]["enabled"], true);
        assert!(back["database"].get("url").is_none());
    }

    #[test]
    fn host_of_strips_scheme_port_and_path() {
        assert_eq!(host_of("https://a.example.com"), "a.example.com");
        assert_eq!(host_of("https://a.example.com:8443/x"), "a.example.com");
        assert_eq!(host_of("http://localhost:9300"), "localhost");
        assert_eq!(host_of("a.example.com"), "a.example.com");
    }
}
