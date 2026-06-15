fn main() {
    prost_build::compile_protos(&["../../proto/sni.proto"], &["../../proto"])
        .expect("Failed to compile protobuf");
}
