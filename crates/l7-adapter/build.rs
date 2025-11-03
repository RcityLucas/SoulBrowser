fn main() {
    let protoc_path =
        protoc_bin_vendored::protoc_bin_path().expect("failed to find bundled protoc");
    std::env::set_var("PROTOC", protoc_path);

    println!("cargo:rerun-if-changed=proto/adapter.proto");
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile(&["proto/adapter.proto"], &["proto"])
        .expect("failed to compile gRPC definitions");
}
