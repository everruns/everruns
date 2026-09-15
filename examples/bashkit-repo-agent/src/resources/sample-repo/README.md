# fetchkit

A tiny sample repository used by the Everruns `bashkit-repo-agent` example.

Release process: every user-visible change adds a fragment under `changelog.d/`.
Cutting a release bumps the version in each crate manifest, folds the fragments
into `CHANGELOG.md`, and removes them.
