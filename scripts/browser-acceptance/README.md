# Direct browser acceptance

Run the native Everruns capability against a fresh browser and synthetic pages. No
model calls, personal browser profiles, browser extensions, or credentials.

Prerequisites: Python 3, installed Chromium/Chrome (`CHROME_BIN` overrides discovery),
Rust. Fast harness checks: `python3 scripts/browser-acceptance/test_run.py`.
Container mode additionally needs a running Docker-compatible engine. The
script does not start or stop Docker/Colima or alter other containers.

```sh
python3 scripts/browser-acceptance/run.py -- \
  cargo test -p everruns-integrations --features browserless --lib \
  direct_browser_acceptance -- --nocapture

python3 scripts/browser-acceptance/run.py --authenticated -- \
  cargo test -p everruns-integrations --features browserless --lib \
  direct_browser_acceptance -- --nocapture

python3 scripts/browser-acceptance/run.py --container --authenticated -- \
  cargo test -p everruns-integrations --features browserless --lib \
  direct_browser_acceptance -- --nocapture
```

The host test receives `EVERRUNS_BROWSER_CDP_ENDPOINT`,
`EVERRUNS_BROWSER_FIXTURE_URL`, `EVERRUNS_BROWSER_ACCEPTANCE_REQUIRED=1`, and, for
`--authenticated`, `EVERRUNS_BROWSER_AUTHORIZATION`. The fixture URL is resolved by
the browser: in container mode it points to the fixture inside that container,
not the host. Treat authentication values as ephemeral test secrets; do not log
or persist them outside the ephemeral runtime. Container discovery uses a private
mode-0600 file rather than Docker logs.

The local mode uses a fresh temporary profile with Chrome's sandbox enabled.
Container mode builds a disposable Debian/Chromium image, runs as an unprivileged
user, drops Linux capabilities, restricts memory/processes, and publishes CDP only
on host loopback. No host directories are mounted. Chromium's internal sandbox is
disabled **only inside this test container**, so the container is its isolation
boundary. This is not a production hardened browser image. Browser/package
versions follow the Debian repository; record versions when capturing evidence.

The authenticated mode exercises the actual WebSocket handshake through a local
bearer-token proxy. By default it uses loopback `ws://`; add `--tls` for the
verified test-CA transport below. Neither mode validates production gateways,
remote networks, or provider access. Never interpret this result
as approval to expose an unauthenticated CDP port or use plaintext credentials
across an untrusted network. TLS verification is not disabled or system trust
modified.

## Verified TLS transport

Add `--tls --authenticated` to either local or container mode to wrap the CDP
transport in a loopback WSS gateway. OpenSSL generates a one-day test CA and
server certificate in a temporary directory. Only the child test process gets
`SSL_CERT_FILE`; system/browser trust is never modified, and TLS verification
is never disabled. The fixture tests also reject an untrusted CA and wrong
hostname. This tests the real TLS transport, not a production remote provider.

```sh
python3 scripts/browser-acceptance/run.py --container --tls --authenticated -- \
  cargo test -p everruns-integrations --features browserless --lib \
  direct_browser_acceptance -- --nocapture
```

## Acceptance matrix

The native test should cover navigation/snapshot, input and resulting page state,
screenshot, two independent session cookie/storage contexts, and closing one
session without killing the external browser, plus browser-origin WebSocket echo. Fixture pages also provide iframe,
popup, redirect and `/socket` WebSocket text-echo targets for expanded tests.

Additional qualification before full Browse parity: stale refs; target closure;
network disconnect/reconnect; mutation outcome ambiguity; real TLS/auth failure;
redirect/popup egress enforcement; personal-browser lending/revocation; durable
human continue/cancel/timeout; remote artifact transfer. The fixture alone is not
proof these paths passed. No personal-browser handoff is simulated as genuine
user approval.

The script owns and cleans up only its temporary profile, process and uniquely
named container. It leaves the cached acceptance image available for subsequent
runs. Ctrl-C is handled by Python context cleanup; killing the runner forcibly
may require removing its specifically named `everruns-browser-acceptance-*`
container.
