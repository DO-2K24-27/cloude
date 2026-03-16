use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

use initramfs_builder::{Compression, InitramfsBuilder};
use serde::Deserialize;
use serde_json;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InitramfsLanguage {
    pub name: String,       // e.g., "python", "rust", "node"
    pub version: String,    // compatibility/version info
    pub base_image: String, // docker image to use (e.g., "python:3.11-alpine")
}

#[derive(Debug, Deserialize)]
struct LanguageConfig {
    version: String,
    base_image: String,
}

impl InitramfsLanguage {
    /// Build the initramfs generically from the struct fields.
    /// Produces an image named `{name}-{version}.cpio.gz` in backend/tmp.
    /// After a successful build, older versions with the same `name` are removed from tmp/.
    pub fn setup_initramfs(
        self,
        agent_binary: &str,
        init_script: &str,
        initramfs_dir: &str,
    ) -> impl Future<Output = Result<(), Error>> + Send {
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

            let (tmp_dir, out_path, out_file, current_filename, current_prefix) =
                Self::prepare_paths(initramfs_dir, &name, &version)?;

            // Skip rebuild if existing non-empty file is present, but still cleanup old versions.
            if let Ok(meta) = fs::metadata(&out_path) {
                if meta.len() > 0 {
                    if Self::should_rebuild(&out_path, agent_binary, init_script, &base_image)? {
                        let _ = fs::remove_file(&out_path);
                    } else {
                        Self::cleanup_old_versions(
                            tmp_dir.as_str(),
                            &current_prefix,
                            &current_filename,
                        )?;
                        return Ok(());
                    }
                } else {
                    let _ = fs::remove_file(&out_path);
                }
            }

            Self::build_initramfs(&base_image, out_file, &out_path, &agent_binary, init_script)
                .await?;

            let metadata =
                fs::metadata(&out_path).map_err(|e| Error::new(ErrorKind::Other, e.to_string()))?;
            if metadata.len() == 0 {
                let _ = fs::remove_file(&out_path);
                return Err(Error::new(
                    ErrorKind::Other,
                    format!("initramfs {} is empty", out_path.display()),
                ));
            }

            Self::write_build_metadata(&out_path, &base_image)?;

            Self::cleanup_old_versions(tmp_dir.as_str(), &current_prefix, &current_filename)?;

            Ok(())
        }
    }

    fn prepare_paths(
        initramfs_dir: &str,
        name: &str,
        version: &str,
    ) -> Result<(String, PathBuf, String, String, String), Error> {
        let tmp_dir = initramfs_dir.to_string();
        fs::create_dir_all(&tmp_dir).map_err(|e| Error::new(ErrorKind::Other, e.to_string()))?;
        let out_path = PathBuf::from(&tmp_dir).join(format!("{name}-{version}.cpio.gz"));
        let out_file = out_path.to_string_lossy().to_string();
        let current_filename = out_path
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .ok_or_else(|| Error::new(ErrorKind::Other, "invalid output filename"))?;
        let current_prefix = format!("{name}-");
        Ok((
            tmp_dir,
            out_path,
            out_file,
            current_filename,
            current_prefix,
        ))
    }

    async fn build_initramfs(
        base_image: &str,
        out_file: String,
        out_path: &Path,
        agent_binary: &str,
        init_script: &str,
    ) -> Result<(), Error> {
        let build_result = InitramfsBuilder::new()
            .image(base_image)
            .compression(Compression::Gzip)
            .exclude(&["/usr/share/doc/*", "/var/cache/*"])
            .inject(agent_binary, "/usr/bin/cloude-agentd")
            .init_script(init_script)
            .build(out_file)
            .await;

        if let Err(e) = build_result {
            let _ = fs::remove_file(out_path);
            return Err(Error::new(ErrorKind::Other, e.to_string()));
        }

        Ok(())
    }

    fn cleanup_old_versions(
        tmp_dir: &str,
        current_prefix: &str,
        current_filename: &str,
    ) -> Result<(), Error> {
        let current_metadata_filename = format!("{}.meta", current_filename);
        if let Ok(entries) = fs::read_dir(tmp_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
                    if fname.starts_with(current_prefix)
                        && fname != current_filename
                        && fname != current_metadata_filename
                    {
                        fs::remove_file(&path)
                            .map_err(|e| Error::new(ErrorKind::Other, e.to_string()))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn should_rebuild(
        out_path: &Path,
        agent_binary: &str,
        init_script: &str,
        base_image: &str,
    ) -> Result<bool, Error> {
        let out_mtime = fs::metadata(out_path)
            .and_then(|m| m.modified())
            .map_err(|e| Error::new(ErrorKind::Other, format!("failed to stat {}: {}", out_path.display(), e)))?;

        let agent_mtime = fs::metadata(agent_binary)
            .and_then(|m| m.modified())
            .map_err(|e| Error::new(ErrorKind::Other, format!("failed to stat agent binary '{}': {}", agent_binary, e)))?;

        let init_mtime = fs::metadata(init_script)
            .and_then(|m| m.modified())
            .map_err(|e| Error::new(ErrorKind::Other, format!("failed to stat init script '{}': {}", init_script, e)))?;

        let image_mismatch = match Self::read_build_metadata(out_path) {
            Ok(Some(previous_base_image)) => previous_base_image != base_image,
            Ok(None) => true,
            Err(e) => {
                return Err(Error::new(
                    ErrorKind::Other,
                    format!("failed to read build metadata for {}: {}", out_path.display(), e),
                ));
            }
        };

        Ok(agent_mtime > out_mtime || init_mtime > out_mtime || image_mismatch)
    }

    fn metadata_path(out_path: &Path) -> PathBuf {
        PathBuf::from(format!("{}.meta", out_path.display()))
    }

    fn read_build_metadata(out_path: &Path) -> Result<Option<String>, std::io::Error> {
        let metadata_path = Self::metadata_path(out_path);
        match fs::read_to_string(metadata_path) {
            Ok(content) => Ok(Some(content.trim().to_string())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write_build_metadata(out_path: &Path, base_image: &str) -> Result<(), Error> {
        let metadata_path = Self::metadata_path(out_path);
        fs::write(metadata_path, format!("{}\n", base_image))
            .map_err(|e| Error::new(ErrorKind::Other, e.to_string()))
    }
}

pub fn get_languages_config(path: &str) -> Result<Vec<InitramfsLanguage>, Error> {
    let content = fs::read_to_string(path).map_err(|e| match e.kind() {
        ErrorKind::NotFound => Error::new(
            ErrorKind::NotFound,
            format!("languages config file not found at '{}'", path),
        ),
        _ => Error::new(
            ErrorKind::Other,
            format!("failed to read config '{}': {}", path, e),
        ),
    })?;

    let map: HashMap<String, LanguageConfig> = serde_json::from_str(&content).map_err(|e| {
        Error::new(
            ErrorKind::InvalidData,
            format!("invalid JSON in '{}': {}", path, e),
        )
    })?;

    let languages = map
        .into_iter()
        .map(|(name, cfg)| InitramfsLanguage {
            name,
            version: cfg.version,
            base_image: cfg.base_image,
        })
        .collect();
    Ok(languages)
}
