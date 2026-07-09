# SDK OpenAPI extensions (`x-sdk-*`)

SDK extensions encode server-owned model semantics that standard OpenAPI does
not preserve in the generated document. Language casing, builders, validators,
helper methods, and compatibility aliases remain SDK generator configuration.

## `x-sdk-response-wrapper`

Marks a component Schema Object whose generated wire shape wraps or flattens a
canonical resource model. The value is an object with exactly these fields:

| Field | Type | Allowed values | Meaning |
| --- | --- | --- | --- |
| `kind` | string | `resource`, `list` | Whether the schema wraps one resource or a list of resources. |
| `model` | string | Local `#/components/schemas/...` reference | Canonical resource schema represented by the wrapper. |

Placement is limited to named schemas under `components.schemas`. The wrapper's
ordinary OpenAPI shape remains authoritative for wire validation. SDK generators
use the extension only to retain canonical model identity when `utoipa` has
inlined the generic resource inside `WithUrls<T>` or a paginated list wrapper.

For `kind: resource`, an SDK may expose the canonical model while accepting the
wrapper's additional link and count fields on the wire. For `kind: list`, the
`model` identifies each resource in the standard `data` array; whether a language
returns the full pagination object or only its data is SDK-owned configuration.

The extension must not contain language names, generated identifiers, field
casing, source syntax, or helper behavior. Standard OpenAPI composition and
array/property schemas are sufficient everywhere else and require no extension.

## Schema names and aliases

Public wire models use accurate component names directly. Response-only Rust
DTO suffixes such as `Response` are removed with `#[schema(as = ...)]` when the
suffix does not describe a distinct wire envelope. SDK-only compatibility names
such as an endpoint-specific request alias, a shortened batch request name, or
a legacy language-specific response name remain aliases in generator
configuration rather than additional OpenAPI extensions.

The current compatibility aliases are `AnalyzeAgentRequest` for
`PreviewAgentRequest`, `MessageInput` for `InputMessage`,
`SetConnectionRequest` for `ApiKeyConnectionRequest`, `SetSecretsRequest` for
`BatchSetSecretsRequest`, and Rust's `DeleteResponse` for
`DeleteFileResponse`. Generic list types, streaming/list option types, and
event helper views are also SDK generator concerns because they describe client
ergonomics rather than distinct wire schemas.
