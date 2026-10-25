//! Encrypted session secrets in the native authority (EVE-1298).
//!
//! Decision: the same envelope scheme and `key_id:base64_key` format the
//! platform uses for `SECRETS_ENCRYPTION_KEY`: a random per-value data key
//! encrypts the value with AES-256-GCM and is wrapped by a versioned key
//! encryption key; older keys stay available for decryption during rotation.
//! The value ciphertext is additionally bound to its session and name, so a
//! row copied to another session or name does not decrypt. Without a key the
//! authority refuses secret operations instead of storing plaintext.
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::SessionId;

const NONCE_SIZE: usize = 12;
const KEY_SIZE: usize = 32;
const PAYLOAD_VERSION: u8 = 1;
const ALGORITHM: &str = "AES-256-GCM";

#[derive(Serialize, Deserialize)]
pub(super) struct SealedSecret {
    version: u8,
    alg: String,
    key_id: String,
    /// Wrap nonce followed by the wrapped data key.
    dek_wrapped: String,
    nonce: String,
    ciphertext: String,
}

/// Versioned key encryption keys; the first one seals new values.
#[derive(Clone)]
pub(super) struct SecretKeys {
    primary: String,
    keys: Arc<HashMap<String, Aes256Gcm>>,
}

fn failure(message: &str) -> AgentLoopError {
    // THREAT[TM-CRYPTO-001]: never echo key material or plaintext in errors.
    AgentLoopError::store(message.to_owned())
}

fn parse(key: &str) -> Result<(String, Aes256Gcm)> {
    let (id, encoded) = key
        .split_once(':')
        .filter(|(id, _)| !id.is_empty())
        .ok_or_else(|| AgentLoopError::config("secret key must be 'key_id:base64_key'"))?;
    let bytes = BASE64
        .decode(encoded)
        .map_err(|_| AgentLoopError::config("secret key is not base64"))?;
    if bytes.len() != KEY_SIZE {
        return Err(AgentLoopError::config("secret key must be 32 bytes"));
    }
    let cipher = Aes256Gcm::new_from_slice(&bytes)
        .map_err(|_| AgentLoopError::config("invalid secret key"))?;
    Ok((id.to_owned(), cipher))
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    rand::rng().fill_bytes(&mut bytes);
    bytes
}

fn aad(session: SessionId, name: &str) -> Vec<u8> {
    format!("everruns.framework.session-secret.v1:{session}:{name}").into_bytes()
}

impl SecretKeys {
    pub(super) fn new(primary: &str, previous: &[&str]) -> Result<Self> {
        let (primary_id, cipher) = parse(primary)?;
        let mut keys = HashMap::from([(primary_id.clone(), cipher)]);
        for key in previous {
            let (id, cipher) = parse(key)?;
            if keys.insert(id, cipher).is_some() {
                return Err(AgentLoopError::config("duplicate secret key id"));
            }
        }
        Ok(Self {
            primary: primary_id,
            keys: Arc::new(keys),
        })
    }

    pub(super) fn seal(&self, session: SessionId, name: &str, value: &str) -> Result<SealedSecret> {
        let kek = &self.keys[&self.primary];
        let dek = random::<KEY_SIZE>();
        let wrap_nonce = random::<NONCE_SIZE>();
        let wrapped = kek
            .encrypt(&Nonce::from(wrap_nonce), dek.as_ref())
            .map_err(|_| failure("could not wrap the secret data key"))?;
        let data_nonce = random::<NONCE_SIZE>();
        let ciphertext = Aes256Gcm::new_from_slice(&dek)
            .map_err(|_| failure("invalid secret data key"))?
            .encrypt(
                &Nonce::from(data_nonce),
                Payload {
                    msg: value.as_bytes(),
                    aad: &aad(session, name),
                },
            )
            .map_err(|_| failure("could not encrypt the secret"))?;
        let mut dek_wrapped = wrap_nonce.to_vec();
        dek_wrapped.extend_from_slice(&wrapped);
        Ok(SealedSecret {
            version: PAYLOAD_VERSION,
            alg: ALGORITHM.into(),
            key_id: self.primary.clone(),
            dek_wrapped: BASE64.encode(dek_wrapped),
            nonce: BASE64.encode(data_nonce),
            ciphertext: BASE64.encode(ciphertext),
        })
    }

    pub(super) fn open(
        &self,
        session: SessionId,
        name: &str,
        sealed: &SealedSecret,
    ) -> Result<String> {
        if sealed.version != PAYLOAD_VERSION || sealed.alg != ALGORITHM {
            return Err(failure("unsupported secret envelope"));
        }
        let kek = self
            .keys
            .get(&sealed.key_id)
            .ok_or_else(|| failure("secret sealed with an unavailable key"))?;
        let wrapped = BASE64
            .decode(&sealed.dek_wrapped)
            .map_err(|_| failure("corrupt secret envelope"))?;
        if wrapped.len() <= NONCE_SIZE {
            return Err(failure("corrupt secret envelope"));
        }
        let (wrap_nonce, wrapped) = wrapped.split_at(NONCE_SIZE);
        let wrap_nonce =
            Nonce::try_from(wrap_nonce).map_err(|_| failure("corrupt secret envelope"))?;
        let dek = kek
            .decrypt(&wrap_nonce, wrapped)
            .map_err(|_| failure("secret data key does not unwrap"))?;
        let nonce = BASE64
            .decode(&sealed.nonce)
            .map_err(|_| failure("corrupt secret envelope"))?;
        let nonce =
            Nonce::try_from(nonce.as_slice()).map_err(|_| failure("corrupt secret envelope"))?;
        let ciphertext = BASE64
            .decode(&sealed.ciphertext)
            .map_err(|_| failure("corrupt secret envelope"))?;
        let plaintext = Aes256Gcm::new_from_slice(&dek)
            .map_err(|_| failure("invalid secret data key"))?
            .decrypt(
                &nonce,
                Payload {
                    msg: &ciphertext,
                    aad: &aad(session, name),
                },
            )
            .map_err(|_| failure("secret does not decrypt for this session"))?;
        String::from_utf8(plaintext).map_err(|_| failure("secret is not UTF-8"))
    }
}
