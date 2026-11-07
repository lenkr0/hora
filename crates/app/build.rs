use std::path::PathBuf;

fn main() {
    // Übersetzungen aus translations/<sprache>/LC_MESSAGES/hora-app.po fest
    // einbinden; ohne Standardkontext, damit Rust (crates/i18n) dieselben
    // Einträge findet.
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_bundled_translations("../../translations")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint-Oberfläche kompilieren");

    // Programmsymbol und Versionsangaben (Explorer, Task-Manager, Startmenü)
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let icon = manifest.join("assets").join("hora.ico");
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
        let rc = format!(
            r#"1 ICON "{icon}"
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
      VALUE "FileDescription", "Hora Calendar"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "hora"
      VALUE "LegalCopyright", "Copyright (c) 2026 lenkr0, MIT License"
      VALUE "OriginalFilename", "hora.exe"
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
            icon = icon.display().to_string().replace('\\', "\\\\"),
        );
        let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("hora.rc");
        std::fs::write(&out, rc).unwrap();
        println!("cargo:rerun-if-changed=assets/hora.ico");
        embed_resource::compile(&out, embed_resource::NONE)
            .manifest_optional()
            .expect("Programmsymbol einbinden");
    }
}
