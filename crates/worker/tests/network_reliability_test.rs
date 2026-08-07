// Scaffold for worker↔control-plane transport-fault tests using `turmoil`
// (https://github.com/tokio-rs/turmoil). Covers scenario 3 of
// knowledge/runtime-resources/agent-reliability-tests.md: deterministic network partitions, latency,
// and reorder between the worker and the control plane, which `fail-rs` cannot
// model because it injects at the store layer rather than the transport.
//
// What this file currently exercises:
//   * One simulated host ("cp") and one simulated client ("driver")
//     communicating over turmoil's TCP shim, modelling a heartbeat-like
//     request/response loop.
//   * `partition` / `repair` to verify the client observes I/O errors during
//     an outage and reconnects after recovery.
//
// Next step (not yet wired): drive the real tonic `WorkerService` client from
// `everruns_worker::grpc_durable_store` through turmoil. That requires a
// custom tonic connector that calls `turmoil::net::TcpStream::connect` instead
// of `tokio::net::TcpStream::connect`. See turmoil's `examples/grpc.rs`.

use std::io;
use std::net::{IpAddr, Ipv4Addr};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use turmoil::Builder;
use turmoil::net::{TcpListener, TcpStream};

const CP_PORT: u16 = 9001;

/// Minimal "control plane": accepts a connection and echoes a single byte per
/// request. Stands in for a heartbeat RPC handler until the real tonic server
/// is wired through turmoil's connector.
async fn run_control_plane() -> turmoil::Result {
    let listener = TcpListener::bind((IpAddr::V4(Ipv4Addr::UNSPECIFIED), CP_PORT)).await?;
    loop {
        let (mut sock, _peer) = listener.accept().await?;
        tokio::spawn(async move {
            let mut buf = [0u8; 1];
            while sock.read_exact(&mut buf).await.is_ok() {
                if sock.write_all(&buf).await.is_err() {
                    break;
                }
            }
        });
    }
}

async fn heartbeat_once() -> io::Result<()> {
    let mut sock = TcpStream::connect(("cp", CP_PORT)).await?;
    sock.write_all(&[1u8]).await?;
    let mut buf = [0u8; 1];
    sock.read_exact(&mut buf).await?;
    Ok(())
}

/// Partition mid-flight, then repair: the worker's heartbeat must fail during
/// the partition and succeed after `repair`. Mirrors scenario 3's "extended
/// outage" variant from knowledge/runtime-resources/agent-reliability-tests.md.
///
/// `turmoil::partition` and `turmoil::repair` resolve the world via a
/// thread-local that is only set while a host/client task is running, so the
/// driver itself runs as a client.
#[test]
fn heartbeat_survives_partition_and_repair() {
    let mut sim = Builder::new().build();
    sim.host("cp", run_control_plane);

    sim.client("driver", async move {
        heartbeat_once().await.expect("healthy heartbeat");

        turmoil::partition("driver", "cp");
        let during = heartbeat_once().await;
        assert!(
            during.is_err(),
            "heartbeat must fail while partitioned, got {during:?}"
        );

        turmoil::repair("driver", "cp");
        heartbeat_once()
            .await
            .expect("heartbeat recovers after repair");
        Ok(())
    });

    sim.run().unwrap();
}

/// Smoke test: a clean run with no faults must succeed. Guards against
/// turmoil-version regressions in the simulator setup itself.
#[test]
fn heartbeat_clean_run() {
    let mut sim = Builder::new().build();
    sim.host("cp", run_control_plane);
    sim.client("worker", async {
        for _ in 0..5 {
            heartbeat_once().await?;
        }
        Ok(())
    });
    sim.run().unwrap();
}

// TODO(turmoil-grpc): replace the byte-echo control plane with the real tonic
// WorkerService and drive `GrpcDurableStore` from the client side. Plan:
//   1. Build a `tonic::transport::Channel` via `Endpoint::connect_with_connector`,
//      passing a `tower::service_fn` that returns
//      `turmoil::net::TcpStream::connect((host, port))`.
//   2. Serve `WorkerServiceServer` on a `turmoil::net::TcpListener` inside the
//      "cp" host.
//   3. Port scenario 3 variants from knowledge/runtime-resources/agent-reliability-tests.md:
//      transient failure, extended outage, late completion → TaskNotOwned.
