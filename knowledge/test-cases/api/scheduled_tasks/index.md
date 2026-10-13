# Scheduled tasks (API)

* [TC001: Max Concurrent Enforcement](TC001_max_concurrent.md) - Verify that the max_concurrent setting prevents overlapping executions when a previous execution is still running.
* [TC002: Schedule API CRUD Operations](TC002_api_crud.md) - Verify all CRUD operations work correctly via the REST API.
* [TC003: Durable Schedules - Platform Access](TC003_platform_access.md) - Verify durable schedules require explicit platform-user access rather than organization ownership, and read-only credentials cannot mutate them.
* [TC005: Durable Schedules - Cron Validation](TC005_cron_validation.md) - Verify five-field normalization, seven-field acceptance, invalid cron rejection, and unchanged persisted state after rejected updates.
* [TC008: Horizontal Scaling with Multiple Scheduler Instances](TC008_horizontal_scaling.md) - Verify that multiple scheduler instances can run concurrently without duplicate triggers, race conditions, or missed schedules.
