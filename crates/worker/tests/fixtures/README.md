# Worker transport fixtures

The PEM private keys in this directory are existing synthetic test keys. The
`turn-server-cert.pem` and `turn-client-cert.pem` certificates reuse those keys
and are trusted only by explicitly configured local test listeners/clients.
They add endpoint SAN/EKU constraints for actual TLS/mTLS handshake tests; no
production trust store, identity or credential is created or changed.

The server certificate is valid for `localhost` only. The client certificate is
valid for client authentication only. The older certificates remain unchanged
for existing configuration-parser tests and the untrusted-root counterexample.
