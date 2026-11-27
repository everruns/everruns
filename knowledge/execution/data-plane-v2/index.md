# Data plane v2

* [Data plane v2: shards](design.md) - Shard-native execution for 75 000 agents: control plane, data plane v1 and Raft shards with S3.
* [Control plane and data plane protocol](cp-dp-protocol.md) - Config stream, org admin, feed, spend allowances and org tokens over HTTPS and JSON.
* [Segment packs](segment-packs.md) - One object per store pass for every session's history segments on a shard, swept once no restore point names it.
* [Data plane v2 file system](fs.md) - Workspaces as versioned trees in the shard with contents in S3.
* [Platform sessions on the Everruns Engine](platform-on-engine.md) - Running every platform feature under Engine hosting.
* [Operating data plane v2 shards](operations.md) - Inspecting a fleet, the per-shard SLO gauges, and runbooks for a lost leader, a stuck move and a drain.
* [What data plane v2 guarantees](guarantees.md) - Writes, reads, side effects, spend and restores per failure, and what is not promised.
* [How data plane v2 is tested](testing.md) - The layers that check each guarantee today, the rules tests follow, and the open gaps.
* [Full-org workflow checks on data plane v2](e2e-workflows.md) - The nightly check that a real org's everyday workflows work on v2 end to end with a real model.
* [Data plane v2 protocol models](models.md) - TLA+ models of moves, spend grants and the feed, each with a bug configuration that must keep failing.
* [Retiring data plane v1](v1-retirement.md) - What must hold before the worker data plane is removed, the order it goes in, and what stays.
