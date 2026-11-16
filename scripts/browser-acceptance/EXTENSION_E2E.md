# Disposable upstream extension acceptance

`extension-e2e.mjs` exercises the reviewed upstream Browse extension through its real settings pairing form and plain MCP tools. It does **not** install anything, use a personal profile, or grant OS permissions. Node 26+ is required for upstream TypeScript helpers. Browser sandbox remains enabled.

Supply every path explicitly. The source must be the unmodified official checkout at `c3b29be5299658281fe4ecdcb926bb7b5a9d6f8c`; build its extension with Bun 1.4.4, frozen lockfile, and test hooks disabled. Build its Rust CLI with `cargo build --locked -p browse`. Use an official Chrome for Testing executable, not a personally used browser. Prepare assets separately; never run upstream `setup`/`install` for this test.

```sh
BROWSE_E2E_SOURCE=/absolute/path/to/pinned/browse \
BROWSE_E2E_BINARY=/absolute/path/to/browse-binary \
BROWSE_E2E_CHROME='/absolute/path/to/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing' \
BROWSE_E2E_EXTENSION=/absolute/path/to/extension/dist \
BROWSE_E2E_OUTPUT=/absolute/path/to/new-output-directory \
node scripts/browser-acceptance/extension-e2e.mjs
```

The output directory must not exist. Default/fallback Browse ports must be unused; the test never stops another listener. Runtime children receive an explicit minimal environment, private HOME, private relay state, and a fresh browser profile. Cleanup terminates only owned process groups and removes private browser/relay state. Logs and non-secret evidence remain in the output directory. Do not publish raw logs without review.

The fixture uses snapshot refs for fill/click, matching the coordinator's narrow adapter surface. Planned assertions cover screenshot output, denial of another MCP owner's tab, genuine handoff Continue via trusted **synthetic test input**, refusal of an untrusted DOM click, timeout, and tab closure. Synthetic input is not a person's approval. This does not test production Framework/Yolop dispatch end-to-end or actual toolbar lending/revocation; those are separate qualification gates.

## Mac mini evidence, 2026-11-16

- Official CFT version: `157.0.8103.78`, mac-arm64.
- Download: `https://storage.googleapis.com/chrome-for-testing-public/157.0.8103.78/mac-arm64/chrome-mac-arm64.zip`.
- Locally calculated archive SHA-256: `bf3b49d0fe62a62397e98641aa736ef813c46014f03ec13d4913442e6966c7fc`. This is a reproducibility fingerprint, not a separately verified publisher checksum.
- Locked extension production build succeeded. Pinned Rust CLI build succeeded in 25.65 seconds.
- Headless and headed disposable CFT launched; production extension service worker loaded; real settings form submitted.
- **Blocked:** extension WebSocket to owned loopback relay never opened. Extension `fetch` to `127.0.0.1:19988` timed out while host `curl` succeeded. Relay reported `pairingState: not-seen`; no pairing rejection reached it. Cause not established.
- Native inspection was blocked because the Mac was locked and automatic unlock was paused after physical input. No permission was granted and no sandbox/network setting was weakened. User unlock is needed to inspect any possible prompt; no prompt has been confirmed.
- macOS CFT's crash reporter used its standard OS-user Application Support/Crashpad location despite private HOME and disable-crash-reporter flags. No personal Chrome profile or account was opened. Do not describe the run as guaranteeing zero incidental OS-user writes.
- Own launched processes and private profiles/state were cleaned after each completed attempt. Existing unrelated data was not removed.
- **Not passed:** pairing, fixture actions, actual handoff completion, ownership isolation, or toolbar lending/revocation. The script contains assertions for these but stopped at pairing readiness. Do not mistake its cleanup record for overall success.

Resume after manual unlock with a fresh output directory. Inspect unexpected OS/browser permissions instead of automatically accepting them. Keep any required approval separate from synthetic fixture consent.
