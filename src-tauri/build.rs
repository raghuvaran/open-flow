fn main() {
    // Required for llama.cpp's use of std::filesystem
    std::env::set_var("MACOSX_DEPLOYMENT_TARGET", "11.0");
    // Disable Metal in llama-cpp-sys's ggml to avoid symbol collision with whisper-rs-sys's ggml
    std::env::set_var("CMAKE_GGML_METAL", "OFF");
    tauri_build::build()
}
