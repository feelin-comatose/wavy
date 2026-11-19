// Иконка exe для Windows (работает и при кросс-сборке с Linux через mingw windres)
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut r = winresource::WindowsResource::new();
        r.set_icon("assets/icon.ico");
        r.set("ProductName", "Wavy");
        r.set("FileDescription", "Wavy — музыкальный плеер");
        r.compile().expect("не удалось встроить иконку");
    }
}
