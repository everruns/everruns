---
title: boxd
description: boxd machines for code execution, persistent Linux VMs with Docker that hibernate between turns with running processes intact, in Frankfurt or New York.
appliesTo: [platform]
---

Everruns integrates with [boxd](https://boxd.sh/) machines: persistent Linux virtual machines (Ubuntu 24.04) with their own kernel and Docker preinstalled. A machine stays until it is deleted. When a session goes idle, Everruns hibernates it: memory, running processes and files are kept, and boxd bills only for the disk until the next command wakes it.

## What You Get

- **Full VMs**: Docker, system services and anything else that needs a real kernel run as they would on a server
- **Sleep without losing work**: hibernation keeps running processes, so a dev server or a container started in one turn is still up in the next
- **Regions**: Frankfurt (`fra`) or New York (`nyc`), chosen per template or per machine
- **Sizes**: `1x4`, `2x8`, `4x16` or `8x32` (vCPUs x GiB of memory)
- **Snapshots**: boot from a boxd snapshot instead of the default image
- **Network rules** that boxd enforces outside the machine, DNS included

## Quick Start

### 1. Create a boxd API key

1. Sign in at [app.boxd.sh](https://app.boxd.sh/)
2. Open [API keys](https://app.boxd.sh/keys) and create a key in the organization that should pay for the machines
3. Copy the key (`bxd_...`); it is shown once

### 2. Connect in Everruns

1. Go to **Settings** > **Connections**
2. Find **boxd** in the available providers
3. Click **Connect** and paste the key

Everruns checks the key with boxd before saving it.

## Sandbox Templates

boxd is a **managed** target in [Sandbox Templates](/features/sandbox-templates/). Pick **boxd** under **Runs in**, then choose:

| Option | Meaning |
|---|---|
| Region | `fra` or `nyc`. Left at the provider default, machines go to your boxd organization's default region |
| Compute size | `1x4`, `2x8`, `4x16` or `8x32`. Left at the provider default, boxd uses your organization's default |
| Snapshot | A boxd snapshot to boot from. A machine from a snapshot keeps the snapshot's size, so size and snapshot cannot both be set |
| Workspace path | Where commands and relative paths start (default `/home/boxd/workspace`) |
| Outbound network | Open, only listed domains, or blocked |

The agent then uses the standard sandbox tools, and Everruns manages the machine for the session: it creates it on first use, hibernates it when the session goes idle, wakes it on the next command, and deletes it with the session. Recovery for boxd templates is `provider_snapshot`: the machine and its disk persist, and if boxd loses a machine, Everruns starts a fresh one.

In the API, the same options go in `target.options`:

```json
{
  "target": {
    "kind": "managed",
    "provider": "boxd",
    "options": { "region": "nyc", "size": "4x16" }
  },
  "durability": "provider_snapshot"
}
```

## Agent tools

Add the **boxd** capability to an agent to let it manage machines itself:

| Tool | Description |
|------|-------------|
| `boxd_create_sandbox` | Create a machine, optionally choosing `region`, `size` or `snapshot` |
| `boxd_exec` | Run a shell command and return stdout, stderr and the exit code |
| `boxd_read_file` | Read a text file from the machine |
| `boxd_write_file` | Write a text file, creating parent directories |
| `boxd_list_sandboxes` | List machines created in this session |
| `boxd_manage_sandbox` | Check status, hibernate, wake or delete a machine |

Commands and relative file paths start in `/home/boxd/workspace`. Each machine also gets an HTTPS URL, returned by `boxd_create_sandbox`.

## Network

Templates' **Outbound network** setting and the session's network access list become the machine's egress allowlist on boxd, set when the machine is created and updated when it wakes:

- **Blocked**: no outbound traffic, DNS included
- **Only listed domains**: just those hosts (`*.example.com` for subdomains) and IPv4 ranges
- **Open**: unrestricted

boxd machines have no IPv6 egress, and an allowlist holds at most 256 entries; a template that asks for more is refused when it is saved. Open egress leaves directly from boxd, not through an Everruns egress proxy.

## Lifecycle and cost

Machines run until the session deletes them or the session's lease on them expires: Everruns deletes leased machines when the lease runs out, so an abandoned session does not keep one. boxd's own idle timers are turned off on these machines, so a long build that makes no network traffic is not put to sleep mid-command; Everruns' idle policy decides when a machine hibernates. boxd bills for the vCPU a machine actively uses, the memory in use and the disk written.

## Security

- **Isolation**: each machine is a KVM virtual machine created with boxd's `isolated` setting, which also keeps boxd's own CLI and credentials out of it
- **Encrypted credentials**: the key is stored in user connections, encrypted at rest, and never enters the machine
- **Session ownership**: tools only act on machines this session created
- **Network access**: machines have open outbound access unless you block or allowlist it

## Not yet supported

- Computer use (a desktop the agent sees and drives)
- Checkpointed recovery and rewind
- Building a template's setup into a boxd snapshot
