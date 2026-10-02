fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    let ext = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let sidecar_name = format!("binaries/cdus-agent-{}{}", target, ext);
    let sidecar_path = std::path::Path::new(&sidecar_name);
    if !sidecar_path.exists() {
        if let Some(parent) = sidecar_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(sidecar_path, b"");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = std::fs::metadata(sidecar_path) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o755);
                let _ = std::fs::set_permissions(sidecar_path, perms);
            }
        }
    }

    tauri_build::build()
}
