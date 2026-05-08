fn main() {
    println!("cargo:rerun-if-changed=../presets");
    println!("cargo:rerun-if-env-changed=SHINE_VERSION_METADATA");

    if let Ok(metadata) = std::env::var("SHINE_VERSION_METADATA")
        && !metadata.trim().is_empty()
    {
        println!("cargo:rustc-env=SHINE_VERSION_METADATA={metadata}");
    }
}
