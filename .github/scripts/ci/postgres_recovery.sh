#!/usr/bin/env bash
# Only the disposable service container belonging to this Actions job is eligible.
set -euo pipefail
: "${PG_CONTAINER:?Pass job.services.postgres.id}"
[[ "$PG_CONTAINER" =~ ^[0-9a-f]{12,64}$ ]] || { echo 'Invalid service container id' >&2; exit 1; }
psql_in() { docker exec "$PG_CONTAINER" psql -U everruns -v ON_ERROR_STOP=1 -At "$@"; }
settings=$(psql_in -d postgres -c "SELECT current_setting('fsync') = 'on' AND current_setting('synchronous_commit') = 'on' AND current_setting('full_page_writes') = 'on'")
[[ "$settings" == t ]] || { echo 'Durability settings are disabled' >&2; exit 1; }
psql_in -d postgres -c 'CREATE DATABASE ci_recovery_probe'
psql_in -d ci_recovery_probe -c 'CREATE TABLE committed_probe (value integer); INSERT INTO committed_probe VALUES (1)'
docker kill --signal=KILL "$PG_CONTAINER" >/dev/null
docker start "$PG_CONTAINER" >/dev/null
ready=false
for attempt in {1..60}; do
  if docker exec "$PG_CONTAINER" pg_isready -U everruns >/dev/null 2>&1; then ready=true; break; fi
  sleep 1
done
[[ "$ready" == true ]] || { echo 'PostgreSQL did not recover' >&2; exit 1; }
[[ "$(psql_in -d ci_recovery_probe -c 'SELECT count(*) FROM committed_probe WHERE value = 1')" == 1 ]]
psql_in -d postgres -c 'DROP DATABASE ci_recovery_probe'
echo 'Committed data survived abrupt PostgreSQL termination with durability enabled.'
