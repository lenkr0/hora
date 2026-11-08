use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    // Slint sucht gebündelte Übersetzungen unter <sprache>/LC_MESSAGES/<crate>.po.
    // Das Setup nutzt dieselben Dateien wie die App (hora-app.po); deshalb
    // hierher kopieren, unter dem Namen dieses Crates.
    let source = manifest.join("..").join("..").join("translations");
    let bundle = out.join("translations");
    println!("cargo:rerun-if-changed={}", source.display());
    if let Ok(dir) = std::fs::read_dir(&source) {
        for item in dir.flatten() {
            let po = item.path().join("LC_MESSAGES").join("hora-app.po");
            if po.is_file() {
                let target = bundle.join(item.file_name()).join("LC_MESSAGES");
                std::fs::create_dir_all(&target).unwrap();
                std::fs::copy(&po, target.join("hora-setup.po")).unwrap();
                println!("cargo:rerun-if-changed={}", po.display());
            }
        }
    }
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_bundled_translations(&bundle)
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/setup.slint", config)
        .expect("Slint-Oberfläche kompilieren");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let icon = manifest
            .join("..")
            .join("app")
            .join("assets")
            .join("hora.ico");
        let manifest_file = manifest.join("setup.manifest");
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let numbers: Vec<u16> = version
            .split(['.', '-'])
            .filter_map(|p| p.parse().ok())
            .chain([0; 4])
            .take(4)
            .collect();
        let comma = numbers
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let escape = |p: &std::path::Path| p.display().to_string().replace('\\', "\\\\");
        let rc = format!(
            r#"1 ICON "{icon}"
1 24 "{manifest_file}"
1 VERSIONINFO
FILEVERSION {comma}
PRODUCTVERSION {comma}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "Hora Setup"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "hora-setup"
      VALUE "LegalCopyright", "Copyright (c) 2026 lenkr0, MIT License"
      VALUE "OriginalFilename", "Hora-Setup.exe"
      VALUE "ProductName", "Hora"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
            icon = escape(&icon),
            manifest_file = escape(&manifest_file),
        );
        let rc_path = out.join("hora-setup.rc");
        std::fs::write(&rc_path, rc).unwrap();
        println!("cargo:rerun-if-changed=setup.manifest");
        println!("cargo:rerun-if-changed=../app/assets/hora.ico");
        embed_resource::compile(&rc_path, embed_resource::NONE)
            .manifest_optional()
            .expect("Symbol und Manifest einbinden");
    }
}
