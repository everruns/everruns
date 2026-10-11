"""Datalayer's `personal-agent-protocol` discovery check (what `pap company
verify` runs) against the interop run's company front.

The front listens on a random port while the protocol asks for
`https://{domain}/`, so requests to the company domain are sent to
`127.0.0.1:FRONT_PORT` with their `Host` and TLS server name unchanged. Everything else is the
library's own code: its URL policy, redirects, models and issuer checks.
"""

import asyncio
import json
import os
import ssl
import sys

import httpx
from personal_agent_protocol.discovery import DiscoveryClient

DOMAIN = os.environ["COMPANY_DOMAIN"]
PORT = int(os.environ["FRONT_PORT"])


class ToFront(httpx.AsyncHTTPTransport):
    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        if request.url.host == DOMAIN:
            # Connect to the front; TLS still checks the certificate for DOMAIN.
            request.url = request.url.copy_with(host="127.0.0.1", port=PORT)
            request.extensions["sni_hostname"] = DOMAIN
        return await super().handle_async_request(request)


async def main() -> int:
    context = ssl.create_default_context(cafile=os.environ["COMPANY_CA_FILE"])
    http = httpx.AsyncClient(
        timeout=5.0, follow_redirects=False, transport=ToFront(verify=context)
    )

    async def resolve(host: str) -> list[str]:
        return ["127.0.0.1"] if host == DOMAIN else []

    async with DiscoveryClient(
        http_client=http, address_resolver=resolve, allow_private_networks=True
    ) as client:
        try:
            found = await client.discover(DOMAIN)
        except Exception as error:  # report every failure the library raises
            chain, cause = [], error
            while cause is not None:
                chain.append(f"{type(cause).__name__}: {cause}")
                cause = cause.__cause__ or cause.__context__
            print("FAIL pap company verify: " + " <- ".join(chain))
            return 1
    print(f"PASS pap company verify: {found.document.organization.domain} {found.document.auth and found.document.auth.issuer}")
    await http.aclose()
    return 0


sys.exit(asyncio.run(main()))
