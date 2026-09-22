use sha2::{Digest, Sha256};

/// Stable, non-secret identifier for a credential crossing the worker boundary.
pub fn credential_fingerprint(credential: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"everruns:mcp-credential:v1\0");
    hasher.update(credential.as_bytes());
    format!("sha256:{}", lower_hex(&hasher.finalize()))
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::credential_fingerprint;

    #[test]
    fn fingerprint_is_stable_and_credential_specific() {
        assert_eq!(
            credential_fingerprint("old-access"),
            "sha256:340ac3b8510215272183c67dcadd1d12a18558294a40579cadde73b93b8fe1ab"
        );
        assert_ne!(
            credential_fingerprint("old-access"),
            credential_fingerprint("fresh-access")
        );
    }
}
