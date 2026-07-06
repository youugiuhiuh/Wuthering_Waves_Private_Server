use std::path::Path;

fn main() {
    prost_build::compile_protos(&["../../proto/sni.proto"], &["../../proto"])
        .expect("Failed to compile protobuf");

    let proto_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../proto");
    let proto_file = proto_dir.join("subscription.proto");
    if proto_file.exists() {
        prost_build::compile_protos(&[&proto_file], &[&proto_dir]).expect("protobuf compilation failed");
    }
    println!("cargo:rerun-if-changed=../../proto/subscription.proto");
}
