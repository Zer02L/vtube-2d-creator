// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // WebKit/GStreamer: аппаратные декодеры (NVDEC/VA-API) отдают кадры в
    // memory:CUDAMemory, которые WebKit не может нарисовать в <canvas> —
    // превью в «Бесшовном цикле» получается прозрачным. Заставляем GStreamer
    // использовать программные декодеры. shell.nix задаёт то же значение,
    // но здесь оно выставляется до старта WebKit и работает в любом окружении.
    if std::env::var_os("GST_PLUGIN_FEATURE_RANK").is_none() {
        // Безопасно: это самый старт main(), до создания потоков и WebKit.
        unsafe {
            std::env::set_var(
                "GST_PLUGIN_FEATURE_RANK",
                "nvh264dec:0,nvh265dec:0,nvvp8dec:0,nvvp9dec:0,\
                 vah264dec:0,vah265dec:0,vavp8dec:0,vavp9dec:0,\
                 vaapih264dec:0,vaapih265dec:0,vaapivp8dec:0,vaapivp9dec:0",
            );
        }
    }

    tauri_app_lib::run()
}
