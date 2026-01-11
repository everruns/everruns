// Uses protox (pure Rust protobuf compiler) to avoid requiring external protoc binary
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Ensure rebuild triggers when proto files change (protox doesn't emit these automatically)
    println!("cargo:rerun-if-changed=proto/worker.proto");

    let file_descriptors = protox::compile(["proto/worker.proto"], ["proto"])?;
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_fds(file_descriptors)?;
    Ok(())
}
