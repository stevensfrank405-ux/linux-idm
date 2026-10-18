fn main() {
    println!("cargo:rerun-if-changed=assets/opendownload.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("assets/opendownload.ico")
            .set("ProductName", "OpenDownload")
            .set("FileDescription", "OpenDownload desktop download manager");
        resource
            .compile()
            .expect("Windows icon/resource compilation failed");
    }
}
