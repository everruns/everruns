//! Sealing of tenant secrets between the control plane and the data plane
//! (cp-dp-protocol.md, "Secrets"). One implementation for both sides, so
//! what the control plane seals the executors can open.
//!
//! Decisions:
//! - Two layers: the data plane's key (one per data plane) wraps each org's
//!   data key, and the org's data key seals that org's secrets. Rotating
//!   the data plane's key rewraps data keys only; secrets stay as sealed.
//! - AES-256-GCM, 12-byte random nonces. The additional data binds every
//!   ciphertext to where it belongs: a secret to its org, id and revision,
//!   a data key to its org and id. A ciphertext copied to another org,
//!   another secret or an older revision fails to open.
//! - A wrapped data key is base64(nonce ‖ ciphertext); a sealed secret
//!   carries its nonce and ciphertext as separate base64 fields.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use rand::Rng;

use crate::dataplane::SealedSecret;

/// A 32-byte AES-256 key.
pub type Key = [u8; 32];

/// Why something didn't open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealError(pub String);

impl std::fmt::Display for SealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SealError {}

fn err(what: impl Into<String>) -> SealError {
    SealError(what.into())
}

/// The additional data of a sealed secret.
pub fn secret_aad(org: &str, secret_id: &str, revision: u64) -> String {
    format!("everruns/dp/v1/secret/{org}/{secret_id}/{revision}")
}

/// The additional data of a wrapped data key.
pub fn data_key_aad(org: &str, data_key_id: &str) -> String {
    format!("everruns/dp/v1/data-key/{org}/{data_key_id}")
}

/// A key from its base64 form (`CELL_DP_KEY`, a control plane's setting).
pub fn key_from_base64(encoded: &str) -> Result<Key, SealError> {
    let raw = B64
        .decode(encoded.trim())
        .map_err(|e| err(format!("the key is not base64: {e}")))?;
    raw.try_into().map_err(|_| err("the key is not 32 bytes"))
}

/// A fresh random key (an org's data key).
pub fn random_key() -> Key {
    let mut key = [0u8; 32];
    rand::rng().fill_bytes(&mut key);
    key
}

/// A fresh random nonce.
pub fn random_nonce() -> [u8; 12] {
    let mut nonce = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce);
    nonce
}

fn cipher(key: &Key) -> Result<Aes256Gcm, SealError> {
    Aes256Gcm::new_from_slice(key).map_err(|e| err(e.to_string()))
}

fn encrypt(key: &Key, nonce: &[u8; 12], aad: &str, plain: &[u8]) -> Result<Vec<u8>, SealError> {
    let nonce = Nonce::try_from(&nonce[..]).map_err(|e| err(e.to_string()))?;
    cipher(key)?
        .encrypt(
            &nonce,
            Payload {
                msg: plain,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| err("sealing failed"))
}

fn decrypt(key: &Key, nonce: &[u8], aad: &str, sealed: &[u8]) -> Result<Vec<u8>, SealError> {
    let nonce = Nonce::try_from(nonce).map_err(|_| err("the nonce is not 12 bytes"))?;
    cipher(key)?
        .decrypt(
            &nonce,
            Payload {
                msg: sealed,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| err("the ciphertext does not open under this key and place"))
}

/// Wrap `org`'s data key under the data plane's key.
pub fn wrap_data_key(
    dp_key: &Key,
    org: &str,
    data_key_id: &str,
    data_key: &Key,
    nonce: [u8; 12],
) -> Result<String, SealError> {
    let sealed = encrypt(dp_key, &nonce, &data_key_aad(org, data_key_id), data_key)?;
    let mut out = nonce.to_vec();
    out.extend(sealed);
    Ok(B64.encode(out))
}

/// Unwrap `org`'s data key with the data plane's key.
pub fn unwrap_data_key(
    dp_key: &Key,
    org: &str,
    data_key_id: &str,
    wrapped: &str,
) -> Result<Key, SealError> {
    let raw = B64
        .decode(wrapped)
        .map_err(|e| err(format!("the wrapped key is not base64: {e}")))?;
    if raw.len() < 12 {
        return Err(err("the wrapped key is too short"));
    }
    let (nonce, sealed) = raw.split_at(12);
    decrypt(dp_key, nonce, &data_key_aad(org, data_key_id), sealed)?
        .try_into()
        .map_err(|_| err("the data key is not 32 bytes"))
}

/// Seal `plain` as revision `revision` of `org`'s secret `secret_id`.
pub fn seal(
    data_key: &Key,
    data_key_id: &str,
    org: &str,
    secret_id: &str,
    revision: u64,
    plain: &[u8],
    nonce: [u8; 12],
) -> Result<SealedSecret, SealError> {
    let sealed = encrypt(
        data_key,
        &nonce,
        &secret_aad(org, secret_id, revision),
        plain,
    )?;
    Ok(SealedSecret {
        secret_id: secret_id.to_owned(),
        revision,
        ciphertext: B64.encode(sealed),
        nonce: B64.encode(nonce),
        data_key_id: data_key_id.to_owned(),
    })
}

/// Open `org`'s sealed secret with its data key.
pub fn unseal(data_key: &Key, org: &str, secret: &SealedSecret) -> Result<Vec<u8>, SealError> {
    let nonce = B64
        .decode(&secret.nonce)
        .map_err(|e| err(format!("the nonce is not base64: {e}")))?;
    let sealed = B64
        .decode(&secret.ciphertext)
        .map_err(|e| err(format!("the ciphertext is not base64: {e}")))?;
    decrypt(
        data_key,
        &nonce,
        &secret_aad(org, &secret.secret_id, secret.revision),
        &sealed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DP_KEY: Key = [7u8; 32];
    const DATA_KEY: Key = [9u8; 32];

    /// Fixed vectors: a change to the format breaks these, and with them
    /// every data key and secret already sealed.
    #[test]
    fn the_format_matches_its_vectors() {
        let wrapped = wrap_data_key(&DP_KEY, "org-1", "dk-1", &DATA_KEY, [1u8; 12]).unwrap();
        let sealed = seal(&DATA_KEY, "dk-1", "org-1", "s-1", 3, b"hunter2", [2u8; 12]).unwrap();
        assert_eq!(wrapped, WRAPPED_VECTOR);
        assert_eq!(sealed.ciphertext, SEALED_VECTOR);
        assert_eq!(sealed.nonce, "AgICAgICAgICAgIC");
        let key = unwrap_data_key(&DP_KEY, "org-1", "dk-1", WRAPPED_VECTOR).unwrap();
        assert_eq!(key, DATA_KEY);
        assert_eq!(unseal(&key, "org-1", &sealed).unwrap(), b"hunter2");
    }

    const WRAPPED_VECTOR: &str =
        "AQEBAQEBAQEBAQEBf+iAvpm25QfY3dUoeT55OKHyHf0RnjIaUThKa2+9P7LM/2DOuniNe7JR+6A/sqga";
    const SEALED_VECTOR: &str = "kmGIADswov2BWSgZp49ypfnv8VXO8zU=";

    #[test]
    fn a_ciphertext_opens_only_where_it_belongs() {
        let sealed = seal(&DATA_KEY, "dk-1", "org-1", "s-1", 3, b"x", random_nonce()).unwrap();
        assert!(unseal(&DATA_KEY, "org-2", &sealed).is_err(), "another org");
        let mut older = sealed.clone();
        older.revision = 2;
        assert!(
            unseal(&DATA_KEY, "org-1", &older).is_err(),
            "another revision"
        );
        let mut moved = sealed.clone();
        moved.secret_id = "s-2".into();
        assert!(
            unseal(&DATA_KEY, "org-1", &moved).is_err(),
            "another secret"
        );
        assert!(
            unseal(&random_key(), "org-1", &sealed).is_err(),
            "another key"
        );
        let wrapped = wrap_data_key(&DP_KEY, "org-1", "dk-1", &DATA_KEY, random_nonce()).unwrap();
        assert!(unwrap_data_key(&DP_KEY, "org-2", "dk-1", &wrapped).is_err());
        assert!(unwrap_data_key(&DP_KEY, "org-1", "dk-2", &wrapped).is_err());
        assert!(key_from_base64("c2hvcnQ=").is_err());
        assert_eq!(key_from_base64(&B64.encode(DP_KEY)).unwrap(), DP_KEY);
    }
}
