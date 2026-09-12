# Account recovery policy (fictional service)
- Never request passwords, one-time codes, or recovery-code contents.
- A password reset does not disable MFA.
- When MFA is enabled and neither authenticator nor recovery codes are available,
  direct the customer to verified identity recovery at https://support.example.com/recovery.
  Support must not disable MFA based solely on a chat request.
- An active lockout must expire before retrying. Repeated reset requests do not clear it.
- With no MFA blocker or lockout, retry in a private browser window to exclude stale
  cookies/autofill; escalate to support if that fails.
