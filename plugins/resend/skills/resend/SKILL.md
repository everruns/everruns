---
name: resend
description: Send emails through the Resend remote MCP server (https://mcp.resend.com/mcp) - sending, scheduling, audiences, broadcasts, and how to handle a missing Resend connection.
---

# Resend

Resend (<https://resend.com>) is an email platform for developers. This plugin
wires the official Resend remote MCP server into the agent, so email is sent
under the user's own Resend account via OAuth, no API key is ever placed in
agent context.

## Tools

Tool names are prefixed `mcp_resend__`. The server exposes Resend's email
infrastructure, including:

- send, schedule, and batch transactional emails
- manage audiences, contacts, and broadcasts
- manage domains and inspect API logs

Exact tool names and schemas come from live discovery; prefer the discovered
tool descriptions over assumptions.

## Sending email

- Always confirm the recipient, subject, and body with the user before
  sending unless they provided all three explicitly.
- Use a `from` address on a domain verified in the user's Resend account.
  If the user has no verified domain, `onboarding@resend.dev` works for
  testing but can only deliver to the account owner's own address.
- Prefer plain text bodies unless the user asks for HTML.
- After sending, report the email id returned by the tool.

## When Resend is not connected

The Resend MCP server uses OAuth. If Resend tools are missing from the tool
list, or a call fails with an authentication error, tell the user to connect
Resend under **Settings → Connections** (provider "Resend"), then retry.
Do not ask the user for a Resend API key.
