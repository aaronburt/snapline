fn main() {
    let attributes = tauri_build::Attributes::new().windows_attributes(
        tauri_build::WindowsAttributes::new()
            .app_manifest(include_str!("windows-app-manifest.xml")),
    );
    if let Err(error) = tauri_build::try_build(attributes) {
        panic!("tauri-build failed: {error}");
    }
}
