fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/atk-ut70-rs.ico")
            .compile()
            .expect("无法嵌入 Windows 应用图标");
    }
}
