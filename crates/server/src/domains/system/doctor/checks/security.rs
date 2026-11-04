use super::{Findings, plural};
use crate::auth::config::AuthMode;
use crate::domains::system::doctor::{CheckArea, DoctorCheck, PROBE_TIMEOUT, SystemDoctor};
use crate::storage::encryption::{ENCRYPTED_COLUMNS, EncryptionService};
use std::collections::BTreeMap;

const AREA: CheckArea = CheckArea::Security;

/// The shortest JWT signing secret accepted without a warning.
const MIN_JWT_SECRET_LEN: usize = 32;

pub(super) async fn run(doctor: &SystemDoctor) -> Findings {
    let mut found = Findings::default();
    found.check(auth_mode_check(doctor));
    if let Some(check) = jwt_check(doctor) {
        found.check(check);
    }
    match doctor.encryption.as_deref() {
        None => found.check(
            DoctorCheck::new(AREA, "security.encryption_key", "Encryption key")
                .problem_if(doctor.env.strict(), "SECRETS_ENCRYPTION_KEY is not set.")
                .why("Provider keys, connections and other secrets cannot be stored.")
                .fix(
                    "Set SECRETS_ENCRYPTION_KEY to `kek-v1:` plus 32 random bytes in base64 \
                     (`openssl rand -base64 32`). Keep it: secrets stored with it are \
                     unreadable without it.",
                ),
        ),
        Some(encryption) => {
            let keys = encryption.available_key_ids().len();
            found.check(
                DoctorCheck::new(AREA, "security.encryption_key", "Encryption key").ok(format!(
                    "Secrets are encrypted with key `{}`{}.",
                    encryption.primary_key_id(),
                    if keys > 1 {
                        format!(
                            "; {} can still be read",
                            plural(keys - 1, "older key", "older keys")
                        )
                    } else {
                        String::new()
                    }
                )),
            );
            let census = tokio::time::timeout(PROBE_TIMEOUT * 2, key_census(doctor)).await;
            found.check(match census {
                Ok(Ok(census)) => stored_secrets_check(
                    encryption,
                    &census,
                    decrypt_sample(doctor, encryption).await,
                ),
                Ok(Err(error)) => {
                    DoctorCheck::new(AREA, "security.stored_secrets", "Stored secrets")
                        .info(format!("Stored secrets cannot be counted: {error}."))
                }
                Err(_) => DoctorCheck::new(AREA, "security.stored_secrets", "Stored secrets")
                    .info("Counting stored secrets took too long; skipped."),
            });
        }
    }
    found
}

fn auth_mode_check(doctor: &SystemDoctor) -> DoctorCheck {
    let env = &doctor.env;
    let check = DoctorCheck::new(AREA, "security.sign_in", "Sign-in");
    match env.auth_mode {
        AuthMode::None if env.strict() => check
            .problem("Sign-in is off: anyone who reaches the app is an administrator.")
            .why("Every agent, secret and conversation is open to whoever finds the address.")
            .fix("Set AUTH_MODE to admin, full or external."),
        AuthMode::None => {
            check.info("Sign-in is off. Fine on your own machine, never on a shared one.")
        }
        AuthMode::Admin => check.ok("A single administrator account signs in."),
        AuthMode::Full => check.ok("People sign in with their own accounts."),
        AuthMode::External => check.ok("Sign-in is handled by an external identity provider."),
    }
}

/// The JWT secret signs every session; only modes that issue tokens need it.
fn jwt_check(doctor: &SystemDoctor) -> Option<DoctorCheck> {
    let env = &doctor.env;
    if matches!(env.auth_mode, AuthMode::None) {
        return None;
    }
    let check = DoctorCheck::new(AREA, "security.session_signing", "Session signing secret");
    Some(if env.jwt_secret_len == 0 {
        check
            .problem("AUTH_JWT_SECRET is not set.")
            .why("Sessions cannot be signed, so nobody can stay signed in.")
            .fix("Set AUTH_JWT_SECRET to at least 32 random characters (`openssl rand -hex 32`).")
    } else if env.jwt_secret_len < MIN_JWT_SECRET_LEN {
        check
            .problem_if(
                env.strict(),
                format!(
                    "The session signing secret is only {} characters long.",
                    env.jwt_secret_len
                ),
            )
            .why("A short secret can be guessed, and then anyone can forge a sign-in.")
            .fix("Set AUTH_JWT_SECRET to at least 32 random characters (`openssl rand -hex 32`).")
    } else {
        check.ok("Sessions are signed with a strong secret.")
    })
}

/// Stored secrets per key id, across every encrypted column.
async fn key_census(doctor: &SystemDoctor) -> Result<BTreeMap<String, i64>, String> {
    let pool = doctor.db.background_pool();
    let mut census = BTreeMap::new();
    for column in ENCRYPTED_COLUMNS {
        // The registry holds only compile-time identifiers.
        let query = format!(
            "SELECT convert_from({col}, 'UTF8')::jsonb ->> 'key_id', count(*) \
             FROM {table} WHERE {col} IS NOT NULL GROUP BY 1",
            col = column.column,
            table = column.table,
        );
        let rows: Vec<(Option<String>, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(query.as_str()))
            .fetch_all(pool)
            .await
            .map_err(|error| format!("{}.{}: {error}", column.table, column.column))?;
        for (key_id, count) in rows {
            *census.entry(key_id.unwrap_or_default()).or_insert(0) += count;
        }
    }
    Ok(census)
}

/// Decrypt one stored secret, so a key whose id is right but whose bytes
/// changed is caught. `None` when nothing is stored yet.
async fn decrypt_sample(doctor: &SystemDoctor, encryption: &EncryptionService) -> Option<bool> {
    let pool = doctor.db.background_pool();
    for column in ENCRYPTED_COLUMNS {
        let query = format!(
            "SELECT {col} FROM {table} WHERE {col} IS NOT NULL LIMIT 1",
            col = column.column,
            table = column.table,
        );
        let row: Option<(Vec<u8>,)> = sqlx::query_as(sqlx::AssertSqlSafe(query.as_str()))
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
        if let Some((data,)) = row {
            return Some(encryption.decrypt(&data).is_ok());
        }
    }
    None
}

pub(crate) fn stored_secrets_check(
    encryption: &EncryptionService,
    census: &BTreeMap<String, i64>,
    sample_decrypts: Option<bool>,
) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "security.stored_secrets", "Stored secrets");
    let primary = encryption.primary_key_id();
    let known = encryption.available_key_ids();
    let total: i64 = census.values().sum();
    let lost: i64 = census
        .iter()
        .filter(|(id, _)| !known.contains(&id.as_str()))
        .map(|(_, count)| count)
        .sum();
    let old: i64 = census
        .iter()
        .filter(|(id, _)| id.as_str() != primary && known.contains(&id.as_str()))
        .map(|(_, count)| count)
        .sum();
    if lost > 0 {
        let ids: Vec<&str> = census
            .keys()
            .map(String::as_str)
            .filter(|id| !known.contains(id))
            .collect();
        return check
            .problem(format!(
                "{} encrypted with a key this server does not have ({}).",
                plural(lost as usize, "stored secret is", "stored secrets are"),
                ids.join(", ")
            ))
            .why("Those provider keys and connections cannot be read; agents that need them fail.")
            .fix(
                "Add the missing key to SECRETS_ENCRYPTION_KEY_PREVIOUS, then run \
                 `everruns-admin reencrypt` to move them to the current key.",
            );
    }
    if sample_decrypts == Some(false) {
        return check
            .problem("A stored secret does not decrypt with the configured key.")
            .why(
                "The key's name matches but its value changed, so stored secrets \
                 cannot be read.",
            )
            .fix("Restore the original SECRETS_ENCRYPTION_KEY value.");
    }
    if old > 0 {
        return check
            .warning(format!(
                "{} still use an older key.",
                plural(old as usize, "stored secret", "stored secrets")
            ))
            .why("Key rotation is unfinished; the older key cannot be retired yet.")
            .fix("Run `everruns-admin reencrypt` to move them to the current key.");
    }
    check.ok(format!(
        "{} readable with the current key.",
        plural(total as usize, "stored secret is", "stored secrets are")
    ))
}
