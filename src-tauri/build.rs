fn main() {
    // The app icon is embedded in the exe at build time; rebuild when it changes.
    println!("cargo:rerun-if-changed=icons");

    // The git commit, for `tb version` (the app's About box gets it from vite.config.ts).
    // "+" when there were uncommitted changes.
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    if let Some(commit) = git(&["rev-parse", "--short", "HEAD"]) {
        let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
        println!("cargo:rustc-env=TABLEBOAT_COMMIT={commit}{}", if dirty { "+" } else { "" });
    }
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");

    tauri_build::build()
}
