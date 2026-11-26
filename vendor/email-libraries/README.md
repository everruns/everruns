# Temporary email dependency snapshots

These MIT source snapshots make the email implementation build from this checkout
without sibling repositories or unpublished registry versions. Source of truth is
[Channelkit](https://github.com/everruns/channelkit); each package records its base revision
in UPSTREAM and contains its license. They include the coordinated email changes.

Do not develop separate implementations here. Once the upstream email APIs are
released, replace server path dependencies with those exact registry versions and
remove these snapshots. No release or publication has been performed by this change.

Markkit now resolves from crates.io at exact version `0.3.2`.
