fn main() {
    let v = std::env::var("COLLECTEMOI_VERSION").unwrap_or_default();
    let v = v.trim();
    println!("cargo:rerun-if-env-changed=COLLECTEMOI_VERSION");
    println!("cargo:rustc-env=COLLECTEMOI_BUILD_VERSION={v}");
}
