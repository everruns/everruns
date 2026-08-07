# Security review

Every change that touches code, configuration, or infrastructure gets this review. Perceived low
risk is not a reason to skip it. Purely docs, comment, spec, or test-only changes may instead state
"No security-relevant code changes" with a one-line justification.

## 1. Identify the threat surface

Read `git diff origin/main...HEAD` and map it to the categories in
[`knowledge/security/threat-model.md`](../../../../knowledge/security/threat-model.md) — auth (`TM-AUTH`), authorization
(`TM-AUTHZ`), API surface (`TM-API`), tools and MCP (`TM-TOOL`), prompts and provider keys
(`TM-LLM`), tenancy (`TM-TENANT`), filesystem (`TM-FS`), SQL (`TM-SQL`), sandboxed execution
(`TM-BASH`), frontend (`TM-WEB`), and resource exhaustion (`TM-DOS`). The spec is the current list;
trust it over this summary.

## 2. Review each touched category

Check the diff for injection (SQL, command, prompt, XSS, path traversal), auth/authz bypass, data
exposure in logs/responses/errors/traces, missing validation at trust boundaries, dependency and
supply-chain risk, and unbounded work or allocations.

## 3. Keep `THREAT` comments true

Code carries `// THREAT[TM-XXX-NNN]` markers next to mitigations. If the diff touches code near one,
verify the mitigation still holds. If it opens new surface, add a marker.

## 4. Update the threat model

Update `knowledge/security/threat-model.md` when the change introduces a genuinely new threat or materially
changes a mitigation. Small changes at trust boundaries often qualify.

## 5. Document it

The PR body's **Security** section lists the categories reviewed, findings and how they were
addressed, or an explicit statement that no security-relevant surface was touched, with reasoning.
