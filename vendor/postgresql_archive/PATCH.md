# PostgreSQL archive ZIP security patch

Upstream: `postgresql_archive` 0.21.0, https://github.com/theseus-rs/postgresql-embedded/tree/v0.21.0/postgresql_archive.
Original crates.io archive SHA-256: `274b4eeb875f1196cc577a69b17c3018a3b189d4017122648b1465066b8b08c1`.

The normalized Cargo manifest selects ZIP 9.0.4 instead of 8.1.0. Both
ZIP readers propagate ZIP9's fallible filename decoding with `?` and take
ownership of the decoded name; extraction behavior is otherwise unchanged.
The error-conversion unit fixture uses the deliberately malformed `https://`
instead of `http://`: it still fails request building without network access,
while retaining the ordinary CodeQL HTTPS rule.
The `zip_compatibility` regression target covers ordinary ZIP extraction and
Zonky's nested ZIP/TXZ installation path without external network access. ZIP 8 carries RUSTSEC-2026-0367, 0368 and 0369;
all are patched in ZIP 9. No advisory is suppressed.
The library's separate Cargo.lock and publishing metadata are omitted: the
consuming workspace owns resolution and auditing.

Cargo patches apply only at a workspace root. Both Everruns and its SaaS
wrapper select this patch explicitly so the hosted application does not
need an unrelated OSS pin upgrade. Keep the copies identical. Remove the
patch when a published upstream PostgreSQL dependency selects patched ZIP.

Original source is licensed `(Apache-2.0 OR MIT) AND PostgreSQL`; upstream
copyright and license notices remain in the source and README.
