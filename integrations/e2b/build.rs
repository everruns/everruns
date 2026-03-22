fn main() {
    println!("cargo:rerun-if-changed=proto/process.proto");

    connectrpc_build::Config::new()
        .files(&["proto/process.proto"])
        .includes(&["proto"])
        .include_file("_e2b.rs")
        .compile()
        .expect("failed to compile E2B process proto");
}
