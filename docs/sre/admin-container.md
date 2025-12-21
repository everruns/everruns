# Admin Container

The admin container provides tools for running database migrations, key rotation, and other administrative tasks in production environments.

## Building

```bash
docker build --target admin -f docker/Dockerfile.unified -t everruns-admin .
```

## Commands

| Command | Description |
|---------|-------------|
| `migrate` | Run pending database migrations |
| `migrate-info` | Show migration status |
| `reencrypt` | Re-encrypt secrets with new key |
| `shell` | Interactive shell for debugging |
| `help` | Show usage information |

## Usage

### Run Migrations

```bash
docker run --rm \
    -e DATABASE_URL="postgres://user:pass@host:5432/db" \
    everruns-admin migrate
```

### Check Migration Status

```bash
docker run --rm \
    -e DATABASE_URL="postgres://user:pass@host:5432/db" \
    everruns-admin migrate-info
```

### Re-encrypt Secrets (Dry Run)

```bash
docker run --rm \
    -e DATABASE_URL="postgres://user:pass@host:5432/db" \
    -e SECRETS_ENCRYPTION_KEY="kek-v2:..." \
    -e SECRETS_ENCRYPTION_KEY_PREVIOUS="kek-v1:..." \
    everruns-admin reencrypt --dry-run
```

### Re-encrypt Secrets (Execute)

```bash
docker run --rm \
    -e DATABASE_URL="postgres://user:pass@host:5432/db" \
    -e SECRETS_ENCRYPTION_KEY="kek-v2:..." \
    -e SECRETS_ENCRYPTION_KEY_PREVIOUS="kek-v1:..." \
    everruns-admin reencrypt --batch-size 50
```

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `DATABASE_URL` | Yes | PostgreSQL connection string |
| `SECRETS_ENCRYPTION_KEY` | For reencrypt | Primary encryption key |
| `SECRETS_ENCRYPTION_KEY_PREVIOUS` | For rotation | Previous encryption key |
| `RUST_LOG` | No | Log level (default: info) |

## TLS/SSL Connections

The admin container supports TLS connections to PostgreSQL. Use the `sslmode` parameter in your connection string:

```bash
DATABASE_URL="postgres://user:pass@host:5432/db?sslmode=require"
```

## Production Deployment

The admin container can be run as a one-off task in any container orchestration platform:

- **Kubernetes**: Use a Job or run via `kubectl run`
- **ECS**: Use `aws ecs run-task` with command override
- **Docker Compose**: Use `docker compose run`
- **Nomad**: Use a batch job

See [Production Migrations Runbook](./runbooks/production-migrations.md) for detailed procedures.
