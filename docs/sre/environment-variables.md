# Environment Variables

## API_PREFIX

Optional prefix for all API routes.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Empty (no prefix) |

**Example:**

```bash
# Routes at /api/v1/agents
API_PREFIX=/api
```

**Notes:**
- `/health`, `/swagger-ui`, and `/api-doc/openapi.json` are not affected by this prefix
- Use when running behind a reverse proxy or API gateway that expects a path prefix
