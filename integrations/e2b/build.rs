// Uses protox (pure Rust protobuf compiler) to avoid requiring external protoc binary.
// Compiles proto to a FileDescriptorSet, then feeds it to connectrpc-build.
fn main() {
    println!("cargo:rerun-if-changed=proto/process.proto");

    // Compile proto with protox (pure Rust, no protoc needed)
    let fds = protox::compile(["proto/process.proto"], ["proto"])
        .expect("failed to compile E2B process proto with protox");

    // Serialize the FileDescriptorSet to a temp file for connectrpc-build
    use prost::Message;
    let fds_bytes = fds.encode_to_vec();
    let fds_path = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("e2b.fds.bin");
    std::fs::write(&fds_path, &fds_bytes).expect("failed to write descriptor set");

    connectrpc_build::Config::new()
        .descriptor_set(&fds_path)
        .files(&["process.proto"])
        .include_file("_e2b.rs")
        .compile()
        .expect("failed to generate E2B connectrpc code");
}
