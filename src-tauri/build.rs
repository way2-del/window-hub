fn main() {
    // Ensure frontend embed invalidates when Vite output changes.
    // Without this, release-fast incremental builds can keep a stale/empty
    // asset map → runtime "asset not found: index.html".
    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dist_index = manifest_dir.join("../dist/index.html");
    let dist_dir = manifest_dir.join("../dist");
    println!("cargo:rerun-if-changed={}", dist_index.display());
    println!("cargo:rerun-if-changed={}", dist_dir.join("assets").display());
    if !dist_index.is_file() {
        println!(
            "cargo:warning=frontend missing at {} — run `npm run build` before tauri build",
            dist_index.display()
        );
    }

    #[cfg(windows)]
    build_trayhook();
    #[cfg(windows)]
    stage_everything_dll();
    tauri_build::build();
}

#[cfg(windows)]
fn stage_everything_dll() {
    use std::env;
    use std::path::PathBuf;

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = manifest_dir
        .join("vendor")
        .join("everything-sdk")
        .join("dll")
        .join("Everything64.dll");
    println!("cargo:rerun-if-changed={}", src.display());
    if !src.is_file() {
        println!(
            "cargo:warning=Everything64.dll missing at {}",
            src.display()
        );
        return;
    }

    let res_dir = manifest_dir.join("resources");
    let _ = std::fs::create_dir_all(&res_dir);
    let res_dest = res_dir.join("Everything64.dll");
    if let Err(e) = std::fs::copy(&src, &res_dest) {
        println!("cargo:warning=failed to stage Everything64.dll resource: {e}");
    }

    let package_profile_dir = env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .and_then(|out| {
            out.parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf())
        });
    if let Some(dir) = package_profile_dir {
        let dest = dir.join("Everything64.dll");
        if let Err(e) = std::fs::copy(&src, &dest) {
            println!(
                "cargo:warning=failed to copy Everything64.dll next to binary: {e}"
            );
        }
    }
}

#[cfg(windows)]
fn build_trayhook() {
    use std::env;
    use std::path::PathBuf;
    use std::process::Command;

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let trayhook_manifest = manifest_dir.join("trayhook").join("Cargo.toml");
    // PROFILE is only "debug"|"release" even for custom profiles — derive the real
    // output dir from OUT_DIR (.../target/<profile>/build/<pkg>/out).
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let trayhook_release = profile == "release";
    let package_profile_dir = env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .and_then(|out| {
            out.parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf())
        })
        .unwrap_or_else(|| {
            let target_dir = env::var("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| manifest_dir.join("target"));
            target_dir.join(&profile)
        });

    println!("cargo:rerun-if-changed={}", trayhook_manifest.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("trayhook").join("src").display()
    );

    // Put trayhook artifacts under the package `target/` (ignored by `tauri dev`
    // watcher). NEVER use `trayhook/target` — writes there restart DevCommand forever.
    let trayhook_target_dir = env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("target"))
        .join("trayhook");

    let status = Command::new("cargo")
        .arg("build")
        .arg("--manifest-path")
        .arg(&trayhook_manifest)
        .arg("--target-dir")
        .arg(&trayhook_target_dir)
        .args(if trayhook_release {
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
    let trayhook_built_profile = if trayhook_release { "release" } else { "debug" };
    let built = trayhook_target_dir
        .join(trayhook_built_profile)
        .join(dll_name);
    if !built.is_file() {
        println!(
            "cargo:warning=trayhook DLL missing after build: {}",
            built.display()
        );
        return;
    }

    // Next to the package binary (target/release-fast, target/release, …).
    let dest = package_profile_dir.join(dll_name);
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
    // Skip rewrite when identical — avoids `tauri dev` watching resources/*.dll.
    let res_dir = manifest_dir.join("resources");
    let _ = std::fs::create_dir_all(&res_dir);
    let res_dest = res_dir.join(dll_name);
    let need_copy = match (std::fs::metadata(&built), std::fs::metadata(&res_dest)) {
        (Ok(src), Ok(dst)) => src.len() != dst.len() || {
            let a = std::fs::read(&built).ok();
            let b = std::fs::read(&res_dest).ok();
            a != b
        },
        (Ok(_), Err(_)) => true,
        _ => true,
    };
    if need_copy {
        if let Err(e) = std::fs::copy(&built, &res_dest) {
            println!(
                "cargo:warning=failed to stage trayhook resource {}: {e}",
                res_dest.display()
            );
        }
    }
}
