fn main() {
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["../proto/kirk.proto"], &["../proto"])
        .expect("tonic-build failed");
    println!("cargo:rerun-if-changed=../proto/kirk.proto");
}
