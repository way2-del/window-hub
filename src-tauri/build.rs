fn main() {
    #[cfg(windows)]
    build_trayhook();
    tauri_build::build();
}

#[cfg(windows)]
fn build_trayhook() {
    use std::env;
    use std::path::PathBuf;
    use std::process::Command;

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let trayhook_manifest = manifest_dir.join("trayhook").join("Cargo.toml");
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let target_dir = env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("target"));

    println!("cargo:rerun-if-changed={}", trayhook_manifest.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("trayhook").join("src").display()
    );

    let status = Command::new("cargo")
        .arg("build")
        .arg("--manifest-path")
        .arg(&trayhook_manifest)
        .arg("--target-dir")
        .arg(manifest_dir.join("trayhook").join("target"))
        .args(if profile == "release" {
            vec!["--release"]
        } else {
            vec![]
        })
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            println!("cargo:warning=trayhook build failed with status {s}");
            return;
        }
        Err(e) => {
            println!("cargo:warning=trayhook build spawn failed: {e}");
            return;
        }
    }

    let dll_name = "window_hub_trayhook.dll";
    let built = manifest_dir
        .join("trayhook")
        .join("target")
        .join(&profile)
        .join(dll_name);
    if !built.is_file() {
        println!(
            "cargo:warning=trayhook DLL missing after build: {}",
            built.display()
        );
        return;
    }

    // Copy next to the package binary output so runtime resolve finds it.
    let dest = target_dir.join(&profile).join(dll_name);
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::copy(&built, &dest) {
        println!(
            "cargo:warning=failed to copy trayhook DLL to {}: {e}",
            dest.display()
        );
    } else {
        println!("cargo:warning=trayhook DLL → {}", dest.display());
    }

    // Stage for Tauri bundle resources (must exist before tauri_build::build).
    let res_dir = manifest_dir.join("resources");
    let _ = std::fs::create_dir_all(&res_dir);
    let res_dest = res_dir.join(dll_name);
    if let Err(e) = std::fs::copy(&built, &res_dest) {
        println!(
            "cargo:warning=failed to stage trayhook resource {}: {e}",
            res_dest.display()
        );
    }
}
