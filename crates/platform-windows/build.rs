fn main() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/snapline/windows-app-manifest.xml");
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
}
