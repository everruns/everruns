---
name: send-email
description: Send an email via Resend
argument-hint: "<to> <subject> -- <body>"
---

Send an email through the Resend MCP tools (`mcp_resend__*`).

Parse the arguments as recipient, subject, and body (body follows `--`; if
parts are missing, ask for them). Confirm the three fields with the user,
then call the Resend send-email tool and report the resulting email id.

If Resend tools are unavailable, tell the user to connect Resend under
Settings → Connections and stop.

Arguments: `$ARGUMENTS`
