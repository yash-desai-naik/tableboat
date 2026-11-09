fn main() {
    // The app icon is embedded in the exe at build time; rebuild when it changes.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
