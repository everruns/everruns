// Uses protox (pure Rust protobuf compiler) to avoid requiring external protoc binary
//
// Note: In proto3, message-type fields (like Uuid, Timestamp) are always optional
// at the wire level, so prost generates them as Option<T>. The conversion code in
// lib.rs handles validation for fields that must be present.
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
