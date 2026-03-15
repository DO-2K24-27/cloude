use super::LanguageRuntime;
use std::env;
use std::path::Path;

pub struct RustRuntime;

impl LanguageRuntime for RustRuntime {
    fn source_extension(&self) -> &'static str {
        "rs"
    }

    fn compile_step(&self, source_path: &Path, work_dir: &Path) -> Option<(String, Vec<String>)> {
        let output = work_dir.join("bin");
        Some((
            "rustc".to_string(),
            vec![
                "-o".to_string(),
                output.display().to_string(),
                source_path.display().to_string(),
            ],
        ))
    }

    fn compile_candidates(
        &self,
        source_path: &Path,
        work_dir: &Path,
    ) -> Option<Vec<(String, Vec<String>)>> {
        let args = self.compile_step(source_path, work_dir)?.1;
        let mut programs = Vec::new();

        if let Ok(fallback) = env::var("AGENT_RUSTC_FALLBACK") {
            programs.push(fallback);
        } else {
            programs.push(
                "/usr/local/rustup/toolchains/stable-x86_64-unknown-linux-musl/bin/rustc"
                    .to_string(),
            );
            programs.push(
                "/usr/local/rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/rustc"
                    .to_string(),
            );
            programs.push(
                "/root/.rustup/toolchains/stable-x86_64-unknown-linux-musl/bin/rustc".to_string(),
            );
            programs.push(
                "/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/rustc".to_string(),
            );
        }

        programs.push("/root/.cargo/bin/rustc".to_string());
        programs.push("/usr/local/cargo/bin/rustc".to_string());
        programs.push("/usr/local/bin/rustc".to_string());
        programs.push("/usr/bin/rustc".to_string());
        programs.push("rustc".to_string());

        Some(
            programs
                .into_iter()
                .map(|program| (program, args.clone()))
                .collect(),
        )
    }

    fn run_step(&self, _source_path: &Path, work_dir: &Path) -> (String, Vec<String>) {
        (work_dir.join("bin").display().to_string(), vec![])
    }
}
