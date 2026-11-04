#!/usr/bin/env python3
"""Run a local example through the same managed cloud proxy using its IP.

The provider DNS guard otherwise rejects the trusted proxy's private hostname.
This changes only the representation of the configured proxy endpoint. It never
removes a proxy, changes its port, disables TLS, or changes destination policy.
"""
import json
import os
import socket
import sys
from urllib.parse import urlsplit, urlunsplit


def main():
    if len(sys.argv) < 2:
        raise SystemExit("Usage: cloud_run.py COMMAND [ARG ...]")
    with open("/etc/codex/network-policy.json", encoding="utf-8") as stream:
        policy = json.load(stream)
    if policy.get("version") != 1:
        raise SystemExit("Unsupported managed network policy")
    trusted = urlsplit(policy["http_proxy"])
    if trusted.scheme != "http" or not trusted.hostname or not trusted.port:
        raise SystemExit("Unsupported managed proxy configuration")
    address = socket.gethostbyname(trusted.hostname)
    environment = os.environ.copy()
    changed = False
    for variable in ("HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "ALL_PROXY", "all_proxy"):
        value = environment.get(variable)
        if not value:
            continue
        parsed = urlsplit(value)
        if (parsed.scheme, parsed.hostname, parsed.port) != (trusted.scheme, trusted.hostname, trusted.port):
            raise SystemExit("Proxy differs from the managed policy; refusing to alter it")
        if parsed.username is not None or parsed.password is not None:
            raise SystemExit("Authenticated proxy URL requires a host-specific adapter")
        environment[variable] = urlunsplit(parsed._replace(netloc=f"{address}:{parsed.port}"))
        changed = True
    if not changed:
        raise SystemExit("No inherited managed proxy to canonicalize")
    os.execvpe(sys.argv[1], sys.argv[1:], environment)


if __name__ == "__main__":
    main()
