fn main() {
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("android") => {
            // Support Android devices using 16 KiB memory pages, including NDK 27 builds.
            println!("cargo:rustc-link-arg=-Wl,-z,max-page-size=16384");
        }
        Ok("windows") => println!("cargo:rerun-if-changed=icons/icon.ico"),
        _ => {}
    }
    tauri_build::build()
}
