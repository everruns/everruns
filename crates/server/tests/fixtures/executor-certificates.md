# Executor proof transport fixtures

`executor-b-cert.pem` is a public, self-signed client-authentication certificate
for local transport tests. It reuses the existing synthetic private key at
`crates/worker/tests/fixtures/test-server-key.pem`. Executor A uses the existing
worker client fixture with a different test key. The test listener explicitly
trusts both certificates, while the database binds them to different executors.

No private key is added or generated, no production identity is enrolled, and
no machine trust store is changed. Production enrollment, certificate validity
policy and isolated delivery of credentials remain unimplemented.
