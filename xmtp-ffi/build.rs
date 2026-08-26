use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=XMTP_GEN_HEADER");

    // Committed include/xmtp_ffi.h is the source of truth. cbindgen needs
    // nightly (`-Zunpretty=expanded`); skip unless explicitly requested.
    if env::var("XMTP_GEN_HEADER").as_deref() != Ok("1") {
        return;
    }

    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-changed=src");

    let crate_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let output_dir = PathBuf::from(&crate_dir).join("include");
    std::fs::create_dir_all(&output_dir).expect("Failed to create include directory");

    let config =
        cbindgen::Config::from_file("cbindgen.toml").expect("Failed to read cbindgen.toml");

    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .expect("Failed to generate C bindings")
        .write_to_file(output_dir.join("xmtp_ffi.h"));
}
