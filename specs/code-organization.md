# Code Organization

## Abstract

Conventions for code organization, naming, type flow, testing, and error handling.

## Layer Separation

See `specs/architecture.md` for full layer diagrams. Summary:

```
Transport (HTTP/gRPC) → Services (Business Logic) → Storage (Database)
```

**Rules:**
- Transport handlers delegate ALL logic to services
- Services own validation, conversion, orchestration
- Storage does raw database operations only
- No direct DB access from transport layer

## Naming Conventions

| Layer | Pattern | Example |
|-------|---------|---------|
| Storage Row | `{Entity}Row` | `AgentRow`, `EventRow` |
| Storage Create | `Create{Entity}Row` | `CreateEventRow` |
| Storage Update | `Update{Entity}` | `UpdateAgent` |
| Core Domain | `{Entity}` | `Message`, `ContentPart` |
| API Input | `Input{Entity}` | `InputMessage`, `InputContentPart` |
| API Request | `{Action}{Entity}Request` | `CreateMessageRequest` |
| Service | `{Entity}Service` | `AgentService`, `SessionService` |

## Type Flow

```
HTTP:  Controller → Service → Repository → Database
gRPC:  Handler    → Service → Repository → Database
                      ↓
               Core types shared
```

- API receives request DTOs
- Service converts to storage types
- Service returns domain types
- Transport converts to response format

## Error Handling

**API errors:**
- Never expose internal details to clients
- Return generic messages: "Not found", "Invalid request", "Internal server error"
- Always log full error server-side with `tracing::error!()`

```rust
let result = db.operation().await.map_err(|e| {
    tracing::error!("Operation failed: {}", e);
    StatusCode::INTERNAL_SERVER_ERROR
})?;
```

## Testing

### Unit Tests

Inline `#[cfg(test)]` modules for:
- Serialization/deserialization
- Pure functions
- Business logic without external dependencies

### Integration Tests

`tests/integration_test.rs` for:
- API endpoints
- Multi-service workflows

### When to Add

- New features: always
- Bug fixes: regression tests
- Security fixes: verification tests

### Running

```bash
just test          # All tests (Rust + UI e2e)
just check         # Format + lint + test
```

## Content Types

`ContentPart` enum across all layers:
- `InputContentPart` - user input (text, image only)
- `ContentPart` - full enum (text, image, tool_call, tool_result)
- `From<InputContentPart> for ContentPart` - safe conversion

## File Organization

**Collocation principle:** Keep related code together.

- API contracts (DTOs) live with their route handlers
- Services in `control-plane/src/services/`
- Storage in `control-plane/src/storage/`
- Core types in `core/src/`

## OpenAPI Support

Types needing OpenAPI schema:
```rust
#[cfg_attr(feature = "openapi", derive(ToSchema))]
```
