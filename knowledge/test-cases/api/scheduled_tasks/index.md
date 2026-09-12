# Scheduled tasks (API)

* [TC001: Max Concurrent Enforcement](TC001_max_concurrent.md) - Verify that the max_concurrent setting prevents overlapping executions when a previous execution is still running.
* [TC002: Schedule API CRUD Operations](TC002_api_crud.md) - Verify all CRUD operations work correctly via the REST API.
* [TC003: Organization Isolation](TC003_org_isolation.md) - Verify that schedules are isolated between organizations.
* [TC004: Max Schedules Per Organization Limit](TC004_max_schedules_per_org.md) - Verify that the `max_schedules_per_org` limit (default: 100) is enforced when creating schedules.
* [TC005: Minimum Cron Interval Validation](TC005_min_cron_interval.md) - Verify that the `min_cron_interval_seconds` limit (default: 60) is enforced when creating or updating schedules.
* [TC006: Rate Limit Enforcement (Max Executions Per Hour)](TC006_rate_limit_enforcement.md) - Verify that the `max_executions_per_hour` rate limit (default: 1000) is enforced per organization.
* [TC007: Fair Scheduling Across Organizations](TC007_fair_scheduling.md) - Verify that the scheduler uses round-robin fairness across organizations, preventing one organization with many schedules from starving others.
* [TC008: Horizontal Scaling with Multiple Scheduler Instances](TC008_horizontal_scaling.md) - Verify that multiple scheduler instances can run concurrently without duplicate triggers, race conditions, or missed schedules.
