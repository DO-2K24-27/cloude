use std::fs;
use std::future::Future;
use std::io::{Error, ErrorKind};
use std::path::PathBuf;

use initramfs_builder::{Compression, InitramfsBuilder};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InitramfsLanguage {
    pub name: String,       // e.g., "python", "rust", "node"
    pub version: String,    // compatibility/version info
    pub base_image: String, // docker image to use (e.g., "python:3.11-alpine")
}

impl InitramfsLanguage {
    /// Build the initramfs generically from the struct fields.
    /// Produces an image named `{name}-{version}.cpio.gz` in backend/tmp.
    /// After a successful build, older versions with the same `name` are removed from tmp/.
    pub fn setup_initramfs(self) -> impl Future<Output = Result<(), Error>> + Send {
        async move {
            let InitramfsLanguage {
                name,
                version,
                base_image,
            } = self;

            println!(
                "Setting up {} initramfs (version: {}, image: {})",
                name, version, base_image
            );

            let tmp_dir = "tmp";
            fs::create_dir_all(tmp_dir).map_err(|e| Error::new(ErrorKind::Other, e.to_string()))?;
            let out_path = PathBuf::from(tmp_dir).join(format!("{name}-{version}.cpio.gz"));
            let out_file = out_path.to_string_lossy().to_string();
            let current_filename = out_path
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .ok_or_else(|| Error::new(ErrorKind::Other, "invalid output filename"))?;
            let current_prefix = format!("{name}-");

            let build_result = InitramfsBuilder::new()
                .image(&base_image)
                .compression(Compression::Gzip)
                .exclude(&["/usr/share/doc/*", "/var/cache/*"])
                .inject("./cloude-agentd", "/usr/bin/cloude-agentd")
                .init_script("./init.sh")
                .build(out_file)
                .await;

            if let Err(e) = build_result {
                let _ = fs::remove_file(&out_path);
                return Err(Error::new(ErrorKind::Other, e.to_string()));
            }

            let metadata =
                fs::metadata(&out_path).map_err(|e| Error::new(ErrorKind::Other, e.to_string()))?;
            if metadata.len() == 0 {
                let _ = fs::remove_file(&out_path);
                return Err(Error::new(
                    ErrorKind::Other,
                    format!("initramfs {} is empty", out_path.display()),
                ));
            }

            // Cleanup old versions for the same language.
            if let Ok(entries) = fs::read_dir(tmp_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
                        if fname.starts_with(&current_prefix) && fname != current_filename {
                            fs::remove_file(&path)
                                .map_err(|e| Error::new(ErrorKind::Other, e.to_string()))?;
                        }
                    }
                }
            }

            Ok(())
        }
    }
}
