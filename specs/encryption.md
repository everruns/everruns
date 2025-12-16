# Encryption Specification

## Abstract

Everruns uses envelope encryption to protect sensitive data (API keys, credentials) stored in the database. The design supports key rotation, allowing old data to be decrypted with old keys while new data uses the latest key. A background re-encryption process migrates data to new keys over time.

## Requirements

### Encryption Architecture

1. **Envelope Encryption**: Two-layer encryption using DEK (Data Encryption Key) and KEK (Key Encryption Key)
   - DEK: Random per-value symmetric key used to encrypt plaintext
   - KEK: Master key used to wrap (encrypt) the DEK
   - Separation allows key rotation without re-encrypting all data immediately

2. **Algorithm**: AES-256-GCM (authenticated encryption with associated data)
   - 256-bit key size
   - 96-bit (12-byte) nonce
   - 128-bit authentication tag

3. **Key Versioning**: KEKs are identified by version (e.g., `kek-v1`, `kek-v2`)
   - Ciphertext includes key_id reference to the KEK used
   - Multiple KEK versions may be active during rotation periods
   - Only the latest version is used for new encryptions

### Encrypted Payload Format

Encrypted values are stored as JSON with the following structure:

```json
{
  "version": 1,
  "alg": "AES-256-GCM",
  "key_id": "kek-v3",
  "dek_wrapped": "<base64-encoded wrapped DEK>",
  "nonce": "<base64-encoded 12-byte nonce>",
  "ciphertext": "<base64-encoded ciphertext with auth tag>"
}
```

Field descriptions:
- `version`: Payload format version for future compatibility
- `alg`: Encryption algorithm identifier
- `key_id`: Identifier of the KEK used to wrap the DEK
- `dek_wrapped`: DEK encrypted with the KEK (base64)
- `nonce`: Random nonce used for data encryption (base64)
- `ciphertext`: Encrypted data including authentication tag (base64)

### Configuration

1. **Environment Variables**:
   - `SECRETS_ENCRYPTION_KEY`: Primary KEK in format `<key_id>:<base64_key>` (e.g., `kek-v1:base64...`)
   - `SECRETS_ENCRYPTION_KEY_PREVIOUS`: Optional previous KEK for rotation (same format)

2. **Key Format**: 32-byte key, base64-encoded, with version prefix
   - Example: `kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=`

3. **Key Generation**: Use cryptographically secure random number generator
   - Example: `python3 -c "import os, base64; print('kek-v1:' + base64.b64encode(os.urandom(32)).decode())"`

### Key Rotation Process

1. **Preparation**:
   - Generate new KEK with incremented version (e.g., `kek-v2`)
   - Move current `SECRETS_ENCRYPTION_KEY` to `SECRETS_ENCRYPTION_KEY_PREVIOUS`
   - Set new key as `SECRETS_ENCRYPTION_KEY`

2. **Deployment**:
   - Deploy with both keys configured
   - New encryptions use the new key
   - Decryptions use whichever key matches the ciphertext's key_id

3. **Re-encryption**:
   - Background job reads encrypted values with old key_id
   - Decrypts with old key, re-encrypts with new key
   - Updates database record with new ciphertext

4. **Cleanup**:
   - After all values are re-encrypted, remove `SECRETS_ENCRYPTION_KEY_PREVIOUS`
   - Old key can be securely archived for disaster recovery

### Security Requirements

1. **Key Storage**: KEKs must never be committed to source control
2. **Key Length**: Minimum 256 bits (32 bytes)
3. **Nonce**: Must be unique per encryption operation (12 bytes, randomly generated)
4. **DEK**: Fresh random DEK generated for each encryption operation
5. **Authentication**: All decryption must verify the GCM authentication tag

### API Integration

1. **EncryptionService**: Stateless service initialized with configured KEKs
2. **Thread Safety**: Safe for concurrent use across async tasks
3. **Error Handling**: Clear errors for key mismatch, tampering, or corruption
4. **Backwards Compatibility**: Supports legacy non-versioned ciphertext during migration

### Encrypted Fields

The following database fields are encrypted:
- `llm_providers.api_key`: API keys for LLM provider integrations
- (Future) Additional sensitive credentials as needed
