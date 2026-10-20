// Generated protocol clients stay behind the owning vendor feature.
#![allow(clippy::unwrap_used, clippy::expect_used)]

fn main() {
    #[cfg(feature = "modal")]
    modal();
    #[cfg(feature = "e2b")]
    e2b();
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
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_fds(fds)
        .expect("failed to generate Modal gRPC code");
}

#[cfg(feature = "e2b")]
fn e2b() {
    use prost::Message;
    println!("cargo:rerun-if-changed=proto/e2b/process.proto");
    let fds = protox::compile(["e2b/process.proto"], ["proto"])
        .expect("failed to compile E2B process proto");
    let fds_path =
        std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("e2b.fds.bin");
    std::fs::write(&fds_path, fds.encode_to_vec()).expect("failed to write E2B descriptor set");
    // The descriptor is generated above, not an input. Watching it invalidates
    // every warm build; track the source proto and decoder setting instead.
    println!("cargo:rerun-if-env-changed=BUFFA_ELEMENT_MEMORY_LIMIT");
    connectrpc_build::Config::new()
        .emit_rerun_directives(false)
        .descriptor_set(&fds_path)
        .files(&["e2b/process.proto"])
        .include_file("_e2b.rs")
        .compile()
        .expect("failed to generate E2B ConnectRPC code");
}
