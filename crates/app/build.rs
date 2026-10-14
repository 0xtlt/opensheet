//! On Windows, puts the icon and the version details into opensheet.exe:
//! Explorer, the taskbar and GPUI (which loads icon resource 1 for its
//! windows) show that icon.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    #[cfg(windows)]
    {
        let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let icon = dir.join("../../packaging/windows/opensheet.ico");
        println!("cargo:rerun-if-changed={}", icon.display());
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon(&icon.to_string_lossy())
            .set("ProductName", "OpenSheet")
            .set("FileDescription", "OpenSheet")
            .set("LegalCopyright", "The OpenSheet contributors");
        resource.compile().expect("the resources of opensheet.exe");
    }
}
