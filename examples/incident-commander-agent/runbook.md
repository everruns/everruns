E-RUNBOOK
For checkout degradation:
1. Compare error rate and traffic with other services; inspect recent changes and logs.
2. Assign checkout-oncall as incident owner and payments-oncall to inspect gateway latency.
3. A recent deploy correlated with matching new errors makes rollback a candidate,
   not a confirmed root cause. Get incident-owner approval before production changes.
4. Recovery criteria: checkout errors below 1% and p95 below 300ms for five minutes.
Never say a rollback happened based on a proposal. This example has no rollback tool.
