# Agent setup

This is a DeepSec scanning workspace. Read `README.md` for the configured
project and `node_modules/deepsec/SKILL.md` for commands matching the installed
version. The full docs ship at `node_modules/deepsec/dist/docs/`.

- For a scan session, check `deepsec setup --status --output json`, then run
  `deepsec scan` and `deepsec process` from this directory. The process command
  resumes pending files.
- For changes to the target codebase or matcher set, run `deepsec setup` to
  reconcile the surface inventory and coverage before processing.
- For a new project, run `deepsec init-project <root>` and then follow its
  `data/<id>/SETUP.md` prompt.
- Review generated matchers before committing. Write a hand-authored matcher
  only after a confirmed finding warrants one; read `writing-matchers.md` first.
