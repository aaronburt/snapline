use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=windows-app-manifest.xml");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_owned());
    let mut numbers = version
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(str::to_owned);
    let major = numbers.next().unwrap_or_else(|| "0".to_owned());
    let minor = numbers.next().unwrap_or_else(|| "0".to_owned());
    let patch = numbers.next().unwrap_or_else(|| "0".to_owned());
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let escape = |path: PathBuf| path.display().to_string().replace('\\', "\\\\");
    let icon = escape(manifest_dir.join("icons").join("icon.ico"));
    let manifest = escape(manifest_dir.join("windows-app-manifest.xml"));
    let script = format!(
        r#"1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3F
FILEFLAGS 0
FILEOS 0x40004
FILETYPE 1
FILESUBTYPE 0
{{
    BLOCK "StringFileInfo"
    {{
        BLOCK "040904B0"
        {{
            VALUE "CompanyName", "Snapline"
            VALUE "FileDescription", "Snapline"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "snapline"
            VALUE "OriginalFilename", "snapline.exe"
            VALUE "ProductName", "Snapline"
            VALUE "ProductVersion", "{version}"
        }}
    }}
    BLOCK "VarFileInfo"
    {{
        VALUE "Translation", 0x0409, 0x04B0
    }}
}}
"#
    );
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("snapline.rc");
    if let Err(error) = std::fs::write(&out, script) {
        panic!("could not write {}: {error}", out.display());
    }
    if let Err(error) = embed_resource::compile(&out, embed_resource::NONE).manifest_optional() {
        panic!("the Windows resources could not be embedded: {error}");
    }
}
