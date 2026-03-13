use crate::builder::init::InitScriptGenerator;
use crate::runtimes::LanguageRuntime;
use anyhow::{Context, Result};
use initramfs_builder::{Compression, InitramfsBuilder, RegistryAuth};
use std::path::{Path, PathBuf};

/// Builds an initramfs archive (.cpio.gz) from a container image for a given runtime.
///
/// Each build runs in its own UUID-named subdirectory under `work_dir`
/// so concurrent builds don't collide.
pub struct Builder {
    work_dir: PathBuf,
}

impl Builder {
    pub fn new<P: AsRef<Path>>(work_dir: P) -> Self {
        Self {
            work_dir: work_dir.as_ref().to_path_buf(),
        }
    }

    /// Pull the runtime's base container image, inject the user's source file
    /// and a generated init script, then pack everything into a .cpio.gz archive.
    ///
    /// The output file is what the VMM boots as its initramfs — the kernel
    /// extracts it and runs `/init` (our generated script) as PID 1.
    pub async fn build_image(
        &self,
        runtime: &dyn LanguageRuntime,
        source_code_path: &Path,
    ) -> Result<PathBuf> {
        tokio::fs::create_dir_all(&self.work_dir).await?;
        let build_id = uuid::Uuid::new_v4().to_string();
        let build_dir = self.work_dir.join(build_id);
        tokio::fs::create_dir_all(&build_dir).await?;

        let init_script_content = InitScriptGenerator::generate_script(
            runtime,
            &format!("/lambda/code.{}", runtime.source_extension()),
        );

        let init_script_path = build_dir.join("init.sh");
        tokio::fs::write(&init_script_path, init_script_content)
            .await
            .context("Failed to write init script")?;

        let output_path = build_dir.join(format!("agent-{}.cpio.gz", runtime.source_extension()));
        let base_image = runtime.base_image();

        let builder = InitramfsBuilder::new()
            .image(base_image)
            .compression(Compression::Gzip)
            .auth(RegistryAuth::Anonymous)
            .platform("linux", "amd64")
            .init_script(&init_script_path)
            .inject(
                source_code_path.to_path_buf(),
                PathBuf::from(format!("/lambda/code.{}", runtime.source_extension())),
            );

        builder
            .build(&output_path)
            .await
            .context("Failed to build initramfs")?;

        Ok(output_path)
    }
}
