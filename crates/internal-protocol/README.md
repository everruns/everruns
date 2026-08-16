# everruns-internal-protocol

> gRPC protocol for Everruns worker ↔ control-plane communication.

`everruns-internal-protocol` defines the gRPC service and message types that
Everruns workers use to talk to the control-plane server. It owns the generated
protobuf client and server stubs and the conversions between protobuf and
domain types (for example UUID mapping). It is an internal building block of the
Everruns workspace and is not a stable public API.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents.

## Quick Example

```rust
use everruns_internal_protocol::{proto_uuid_to_uuid, uuid_to_proto_uuid};

// Domain UUIDs convert to and from their protobuf wire representation.
let id = uuid::Uuid::now_v7();
let wire = uuid_to_proto_uuid(id);
assert_eq!(proto_uuid_to_uuid(&wire).unwrap(), id);
```

The `WorkerServiceClient` / `WorkerServiceServer` tonic stubs build on these
conversions for worker ↔ control-plane calls.

## What It Provides

- The `WorkerService` gRPC client and server stubs
- Protobuf message types for worker ↔ control-plane calls
- Conversions between protobuf and Everruns domain types

## Documentation

- [Architecture](https://docs.everruns.com/explanation/architecture/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
