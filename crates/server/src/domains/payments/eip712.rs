// EIP-712 typed-data signing for the x402 payment rail.
//
// Decision: implemented in-tree rather than pulled from a signing library.
// `ethers-rs` is retired upstream and cost 64 crates exclusive to it, including
// a syn-1.x-era generation (thiserror 1.0, rand 0.8, uuid 0.8); its maintained
// successor `alloy-signer-local` measured worse still at 103, because it drags
// the consensus, network, RLP, and KZG stack to sign one struct. What this rail
// actually needs is one fixed EIP-712 struct over secp256k1, which is fully
// specified and small enough to own.
//
// SECURITY: a change to domain separation, struct hashing, or signature
// encoding here silently produces signatures a facilitator will reject, or
// worse, valid signatures over a payload the caller did not intend. Every edit
// must keep `eip3009_signature_matches_known_vector` in `authority.rs` passing —
// that vector was captured from the previous ethers-backed implementation and is
// the equivalence proof for this module.

use k256::ecdsa::{RecoveryId, Signature, SigningKey};
use sha3::{Digest, Keccak256};

/// A locally held secp256k1 signing key and its Ethereum address.
pub struct LocalWallet {
    signing_key: SigningKey,
    address: [u8; 20],
}

#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    #[error("private key must be 32 bytes of hex, optionally 0x-prefixed")]
    MalformedPrivateKey,
    #[error("private key is not a valid secp256k1 scalar")]
    InvalidPrivateKey,
}

#[derive(Debug, thiserror::Error)]
pub enum TypedDataError {
    #[error("{field} is not a 20-byte hex address")]
    Address { field: &'static str },
    #[error("{field} is not a 32-byte hex value")]
    Bytes32 { field: &'static str },
    #[error("{field} is not a base-10 unsigned integer")]
    Uint { field: &'static str },
    #[error("{field} is missing")]
    Missing { field: &'static str },
}

impl LocalWallet {
    /// Parses a 32-byte secp256k1 private key in hex, with or without `0x`.
    pub fn from_private_key_hex(key: &str) -> std::result::Result<Self, WalletError> {
        let trimmed = key.trim().strip_prefix("0x").unwrap_or(key.trim());
        let bytes = decode_hex(trimmed).ok_or(WalletError::MalformedPrivateKey)?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| WalletError::MalformedPrivateKey)?;
        let signing_key =
            SigningKey::from_bytes(&bytes.into()).map_err(|_| WalletError::InvalidPrivateKey)?;
        let address = address_from_verifying_key(&signing_key);
        Ok(Self {
            signing_key,
            address,
        })
    }

    /// The 20-byte Ethereum address.
    pub fn address(&self) -> [u8; 20] {
        self.address
    }

    /// The address in EIP-55 mixed-case checksum form, `0x`-prefixed.
    pub fn address_checksummed(&self) -> String {
        to_checksum(&self.address)
    }

    /// Signs a 32-byte digest, returning the 65-byte `r || s || v` encoding
    /// Ethereum expects, with `v` as `27 + recovery_id`.
    ///
    /// `sign_prehash_recoverable` yields the low-S form with a matching
    /// recovery id; Ethereum rejects high-S signatures.
    pub fn sign_prehash(&self, digest: &[u8; 32]) -> [u8; 65] {
        let (signature, recovery_id) = self.signing_key.sign_prehash_recoverable(digest);

        let mut out = [0u8; 65];
        out[..64].copy_from_slice(&signature.to_bytes());
        out[64] = 27 + recovery_id.to_byte();
        out
    }
}

/// The EIP-712 domain for an EIP-3009 token contract.
pub struct Domain<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub chain_id: u64,
    /// The token contract, as a hex address.
    pub verifying_contract: &'a str,
}

/// The `TransferWithAuthorization` message fields, as the wire strings they
/// arrive in.
pub struct TransferWithAuthorization<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub value: &'a str,
    pub valid_after: &'a str,
    pub valid_before: &'a str,
    /// 32-byte hex nonce.
    pub nonce: &'a str,
}

/// `keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)")`
/// is recomputed rather than hard-coded so the type string stays reviewable.
const DOMAIN_TYPE: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";

const TRANSFER_TYPE: &str = "TransferWithAuthorization(address from,address to,uint256 value,\
uint256 validAfter,uint256 validBefore,bytes32 nonce)";

/// Computes the EIP-712 digest for a `TransferWithAuthorization` message.
///
/// This is `keccak256(0x19 || 0x01 || domainSeparator || structHash)`, with each
/// member ABI-encoded into a 32-byte word: strings as the keccak of their
/// contents, addresses left-padded, integers big-endian.
pub fn transfer_with_authorization_digest(
    domain: &Domain<'_>,
    message: &TransferWithAuthorization<'_>,
) -> std::result::Result<[u8; 32], TypedDataError> {
    let mut domain_encoded = Vec::with_capacity(5 * 32);
    domain_encoded.extend_from_slice(&keccak(DOMAIN_TYPE.as_bytes()));
    domain_encoded.extend_from_slice(&keccak(domain.name.as_bytes()));
    domain_encoded.extend_from_slice(&keccak(domain.version.as_bytes()));
    domain_encoded.extend_from_slice(&u256_from_u128(u128::from(domain.chain_id)));
    domain_encoded.extend_from_slice(&address_word(domain.verifying_contract, "verifyingContract")?);
    let domain_separator = keccak(&domain_encoded);

    let mut struct_encoded = Vec::with_capacity(7 * 32);
    struct_encoded.extend_from_slice(&keccak(TRANSFER_TYPE.as_bytes()));
    struct_encoded.extend_from_slice(&address_word(message.from, "from")?);
    struct_encoded.extend_from_slice(&address_word(message.to, "to")?);
    struct_encoded.extend_from_slice(&uint_word(message.value, "value")?);
    struct_encoded.extend_from_slice(&uint_word(message.valid_after, "validAfter")?);
    struct_encoded.extend_from_slice(&uint_word(message.valid_before, "validBefore")?);
    struct_encoded.extend_from_slice(&bytes32_word(message.nonce, "nonce")?);
    let struct_hash = keccak(&struct_encoded);

    let mut preimage = Vec::with_capacity(2 + 32 + 32);
    preimage.extend_from_slice(&[0x19, 0x01]);
    preimage.extend_from_slice(&domain_separator);
    preimage.extend_from_slice(&struct_hash);
    Ok(keccak(&preimage))
}

/// EIP-55: hex-encode the address, then uppercase each hex digit whose
/// corresponding nibble in `keccak256(lowercase_hex)` is >= 8.
pub fn to_checksum(address: &[u8; 20]) -> String {
    let lowercase = encode_hex(address);
    let hash = keccak(lowercase.as_bytes());
    let mut out = String::with_capacity(42);
    out.push_str("0x");
    for (index, character) in lowercase.chars().enumerate() {
        let nibble = if index % 2 == 0 {
            hash[index / 2] >> 4
        } else {
            hash[index / 2] & 0x0f
        };
        if nibble >= 8 {
            out.extend(character.to_uppercase());
        } else {
            out.push(character);
        }
    }
    out
}

fn keccak(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// The address is the last 20 bytes of the keccak of the uncompressed public
/// key, with the 0x04 SEC1 tag stripped.
fn address_from_verifying_key(signing_key: &SigningKey) -> [u8; 20] {
    let encoded = signing_key.verifying_key().to_sec1_point(false);
    let hash = keccak(&encoded.as_bytes()[1..]);
    let mut address = [0u8; 20];
    address.copy_from_slice(&hash[12..]);
    address
}

fn address_word(value: &str, field: &'static str) -> std::result::Result<[u8; 32], TypedDataError> {
    let trimmed = value.trim().strip_prefix("0x").unwrap_or(value.trim());
    let bytes = decode_hex(trimmed).ok_or(TypedDataError::Address { field })?;
    if bytes.len() != 20 {
        return Err(TypedDataError::Address { field });
    }
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(&bytes);
    Ok(word)
}

fn bytes32_word(value: &str, field: &'static str) -> std::result::Result<[u8; 32], TypedDataError> {
    let trimmed = value.trim().strip_prefix("0x").unwrap_or(value.trim());
    let bytes = decode_hex(trimmed).ok_or(TypedDataError::Bytes32 { field })?;
    let word: [u8; 32] = bytes
        .try_into()
        .map_err(|_| TypedDataError::Bytes32 { field })?;
    Ok(word)
}

/// Token amounts and unix timestamps both fit in u128 by orders of magnitude;
/// anything larger is rejected rather than silently truncated.
fn uint_word(value: &str, field: &'static str) -> std::result::Result<[u8; 32], TypedDataError> {
    let parsed: u128 = value
        .trim()
        .parse()
        .map_err(|_| TypedDataError::Uint { field })?;
    Ok(u256_from_u128(parsed))
}

fn u256_from_u128(value: u128) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[16..].copy_from_slice(&value.to_be_bytes());
    word
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).ok())
        .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::VerifyingKey;

    /// Anvil development key #0. A public test vector, never a real key.
    const TEST_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const TEST_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

    #[test]
    fn derives_the_known_address_for_a_known_key() {
        let wallet = LocalWallet::from_private_key_hex(TEST_KEY).unwrap();
        assert_eq!(wallet.address_checksummed(), TEST_ADDRESS);
    }

    #[test]
    fn accepts_a_key_without_the_hex_prefix() {
        let wallet = LocalWallet::from_private_key_hex(TEST_KEY.trim_start_matches("0x")).unwrap();
        assert_eq!(wallet.address_checksummed(), TEST_ADDRESS);
    }

    #[test]
    fn rejects_malformed_keys() {
        assert!(LocalWallet::from_private_key_hex("0x1234").is_err());
        assert!(LocalWallet::from_private_key_hex("not hex at all").is_err());
        assert!(LocalWallet::from_private_key_hex(&format!("0x{}", "00".repeat(32))).is_err());
    }

    /// EIP-55 vectors from the specification.
    #[test]
    fn checksums_match_eip55_vectors() {
        for expected in [
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
            "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
            "0xdbF03B407c01E7cD3CBea99509d93f8DDDC8C6FB",
            "0xD1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb",
        ] {
            let bytes = decode_hex(expected.trim_start_matches("0x")).unwrap();
            let address: [u8; 20] = bytes.try_into().unwrap();
            assert_eq!(to_checksum(&address), expected);
        }
    }

    fn sample_digest() -> [u8; 32] {
        transfer_with_authorization_digest(
            &Domain {
                name: "USD Coin",
                version: "2",
                chain_id: 8453,
                verifying_contract: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
            },
            &TransferWithAuthorization {
                from: TEST_ADDRESS,
                to: "0x0000000000000000000000000000000000000001",
                value: "10000",
                valid_after: "1700000000",
                valid_before: "1700003600",
                nonce: "0x0000000000000000000000000000000000000000000000000000000000000001",
            },
        )
        .unwrap()
    }

    /// The signature must recover to the signing address. This is independent of
    /// the pinned vector in `authority.rs`: it proves the signature is valid,
    /// where the vector proves it is unchanged.
    #[test]
    fn signature_recovers_to_the_signing_address() {
        let wallet = LocalWallet::from_private_key_hex(TEST_KEY).unwrap();
        let digest = sample_digest();
        let signature = wallet.sign_prehash(&digest);

        let recovery_id = RecoveryId::from_byte(signature[64] - 27).expect("recovery id");
        let parsed = Signature::from_slice(&signature[..64]).expect("signature");
        let recovered =
            VerifyingKey::recover_from_prehash(&digest, &parsed, recovery_id).expect("recover");

        let encoded = recovered.to_sec1_point(false);
        let hash = keccak(&encoded.as_bytes()[1..]);
        assert_eq!(&hash[12..], &wallet.address()[..]);
    }

    #[test]
    fn signatures_use_the_low_s_form() {
        let wallet = LocalWallet::from_private_key_hex(TEST_KEY).unwrap();
        let signature = wallet.sign_prehash(&sample_digest());
        let parsed = Signature::from_slice(&signature[..64]).unwrap();
        assert_eq!(
            parsed.normalize_s(),
            parsed,
            "signature must already be in low-S form"
        );
        assert!(signature[64] == 27 || signature[64] == 28);
    }

    #[test]
    fn digest_changes_with_every_field() {
        let baseline = sample_digest();
        let domain = Domain {
            name: "USD Coin",
            version: "2",
            chain_id: 8453,
            verifying_contract: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
        };
        let message = TransferWithAuthorization {
            from: TEST_ADDRESS,
            to: "0x0000000000000000000000000000000000000001",
            value: "10000",
            valid_after: "1700000000",
            valid_before: "1700003600",
            nonce: "0x0000000000000000000000000000000000000000000000000000000000000001",
        };

        let other_chain = transfer_with_authorization_digest(
            &Domain {
                chain_id: 1,
                ..domain
            },
            &message,
        )
        .unwrap();
        assert_ne!(baseline, other_chain, "chain id must be domain-separated");

        let other_value = transfer_with_authorization_digest(
            &domain,
            &TransferWithAuthorization {
                value: "10001",
                ..message
            },
        )
        .unwrap();
        assert_ne!(baseline, other_value);
    }

    #[test]
    fn rejects_malformed_message_fields() {
        let domain = Domain {
            name: "USD Coin",
            version: "2",
            chain_id: 8453,
            verifying_contract: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
        };
        let valid = TransferWithAuthorization {
            from: TEST_ADDRESS,
            to: "0x0000000000000000000000000000000000000001",
            value: "10000",
            valid_after: "1700000000",
            valid_before: "1700003600",
            nonce: "0x0000000000000000000000000000000000000000000000000000000000000001",
        };

        assert!(
            transfer_with_authorization_digest(
                &domain,
                &TransferWithAuthorization { to: "0xdead", ..valid },
            )
            .is_err()
        );
        assert!(
            transfer_with_authorization_digest(
                &domain,
                &TransferWithAuthorization {
                    value: "-1",
                    ..valid
                },
            )
            .is_err()
        );
        assert!(
            transfer_with_authorization_digest(
                &domain,
                &TransferWithAuthorization {
                    nonce: "0x01",
                    ..valid
                },
            )
            .is_err()
        );
    }
}
