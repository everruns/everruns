# SQLx SQLite compatibility backport

This is the crates.io `sqlx-sqlite` 0.9.0 source (upstream revision
`003b698e99e024f3621b8043a2426fde5b741171`), including its original licenses.
Only the `libsqlite3-sys` upper bound in the normalized and original manifests
changes: `<0.38.0` becomes `<0.40.0`.

The upstream `connection/explain.rs` exceeds this repository's source-file size
limit. Its complete `explain` function is moved verbatim to
`connection/explain/program.rs`, included at its original position. Replacing
the `include!` line with that file's contents reproduces the upstream source
byte-for-byte. Three upstream trailing-whitespace doc-comment lines in
`connection/deserialize.rs` are trimmed, and file permissions are normalized
to non-executable source files. No Rust logic is changed.

The bound matches upstream SQLx commit
[10462fa4414c6c7f1ed1d39bb31678e3b726878c](https://github.com/launchbadge/sqlx/commit/10462fa4414c6c7f1ed1d39bb31678e3b726878c).
It allows the workspace to resolve rusqlite 0.41 and bundled SQLite 3.54.1.
Cargo resolves optional dependencies for its lockfile and enforces a single
`links = "sqlite3"` package even though the platform uses SQLx only for PostgreSQL.

Remove this directory and the root `[patch.crates-io]` entry when a released
SQLx accepts libsqlite3-sys 0.39 or newer. Keep the patched SQLite version and
the advisory gate; do not restore an advisory ignore.
