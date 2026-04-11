use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("tai")
        .join("ghostty");

    let ghostty_dir = cache_dir.clone();
    let pinned_commit = "fdb6e3d2c8543e2e756b7e07f44372efbc0fba4b";

    if !ghostty_dir.join(".git").exists() {
        std::fs::create_dir_all(&cache_dir).expect("Failed to create cache dir");

        let status = Command::new("git")
            .args(["clone", "https://github.com/ghostty-org/ghostty.git"])
            .arg(&ghostty_dir)
            .status()
            .expect("Failed to run git clone");
        assert!(status.success(), "git clone failed");
    }

    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&ghostty_dir)
        .output()
        .expect("Failed to get HEAD");
    let current_head = String::from_utf8_lossy(&head.stdout).trim().to_string();

    if current_head != pinned_commit {
        let status = Command::new("git")
            .args(["fetch", "origin", pinned_commit])
            .current_dir(&ghostty_dir)
            .status()
            .expect("Failed to fetch commit");
        assert!(status.success(), "git fetch failed");

        let status = Command::new("git")
            .args(["checkout", pinned_commit])
            .current_dir(&ghostty_dir)
            .status()
            .expect("Failed to checkout commit");
        assert!(status.success(), "git checkout failed");
    }

    let lib_path = ghostty_dir.join("zig-out").join("lib");
    let static_lib = lib_path.join("libghostty-vt.a");

    if !static_lib.exists() {
        println!("cargo:warning=Building libghostty-vt static library with zig (this may take a while)...");
        let status = Command::new("zig")
            .args([
                "build",
                "-Demit-lib-vt=true",
                "-Doptimize=ReleaseFast",
            ])
            .current_dir(&ghostty_dir)
            .status()
            .expect("Failed to run zig build. Is zig >= 0.14 on PATH?");
        assert!(status.success(), "zig build failed");
    }

    let include_dir = ghostty_dir.join("include");

    let link_dir = out_dir.join("ghostty-link");
    std::fs::create_dir_all(&link_dir).expect("Failed to create link dir");
    std::fs::copy(&static_lib, link_dir.join("libghostty-vt.a"))
        .expect("Failed to copy static lib");

    println!("cargo:rustc-link-search=native={}", link_dir.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt");

    println!("cargo:rustc-link-lib=c++");

    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=c");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
    }

    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=build.rs");

    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg(format!("-I{}", include_dir.display()))
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .allowlist_function("ghostty_.*")
        .allowlist_type("Ghostty.*")
        .allowlist_var("GHOSTTY_.*")
        .generate()
        .expect("Unable to generate bindings");

    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("Couldn't write bindings");
}

mod dirs {
    use std::path::PathBuf;

    pub fn cache_dir() -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join("Library").join("Caches"))
        }
        #[cfg(target_os = "linux")]
        {
            std::env::var("XDG_CACHE_HOME")
                .ok()
                .map(PathBuf::from)
                .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".cache")))
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            None
        }
    }
}
