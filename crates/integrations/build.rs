// Build script: a panic here is how codegen failure reaches the build.
#![allow(clippy::unwrap_used, clippy::expect_used)]
// Each vendor compiles its protos only when its feature is on, and its codegen
// build-dependencies are optional behind the same feature, so a consumer that
// names no vendor pays nothing. protox is a pure-Rust protobuf compiler, so no
// `protoc` binary is needed.
fn main() {
    #[cfg(feature = "modal")]
    modal();
}

#[cfg(feature = "modal")]
fn modal() {
    println!("cargo:rerun-if-changed=proto/modal/api.proto");
    println!("cargo:rerun-if-changed=proto/modal/task_command_router.proto");
    let fds = protox::compile(
        ["modal/api.proto", "modal/task_command_router.proto"],
        ["proto"],
    )
    .expect("failed to compile Modal protos");
    // The tool tests run a mock Modal server, so the server half is generated too.
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_fds(fds)
        .expect("failed to generate Modal gRPC code");
}
