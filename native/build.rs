// Иконка и сведения о программе в exe для Windows (работает и при кросс-сборке с Linux через mingw windres).
// Сведения (автор, лицензия, версия) и манифест — не украшение: Защитник Windows с подозрением смотрит
// на неподписанные exe «без паспорта», которые качают и запускают программы (yt-dlp, ffmpeg) и обновляют сами себя.
fn main() {
    println!("cargo:rerun-if-env-changed=WAVY_BUILD");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let build: u16 = std::env::var("WAVY_BUILD").ok().and_then(|b| b.parse().ok()).unwrap_or(0);
        let ver = format!("1.0.{build}.0");
        let num = (1u64 << 48) | (build as u64) << 16;
        let mut r = winresource::WindowsResource::new();
        r.set_icon("assets/icon.ico");
        r.set("ProductName", "Wavy");
        r.set("FileDescription", "Wavy — музыкальный плеер");
        r.set("CompanyName", "feelin-comatose");
        r.set("LegalCopyright", "GPL-3.0 · github.com/feelin-comatose/wavy");
        r.set("OriginalFilename", "Wavy.exe");
        r.set("InternalName", "Wavy");
        r.set("Comments", "Открытый исходный код: https://github.com/feelin-comatose/wavy");
        r.set("FileVersion", &ver);
        r.set("ProductVersion", &ver);
        r.set_version_info(winresource::VersionInfo::FILEVERSION, num);
        r.set_version_info(winresource::VersionInfo::PRODUCTVERSION, num);
        r.set_manifest(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="Wavy" version="1.0.0.0"/>
  <description>Wavy music player</description>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges><requestedExecutionLevel level="asInvoker" uiAccess="false"/></requestedPrivileges></security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application><supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/></application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
</assembly>"#);
        r.compile().expect("не удалось встроить иконку");
    }
}
