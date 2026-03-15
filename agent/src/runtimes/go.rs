use super::LanguageRuntime;
use std::path::Path;

pub struct GoRuntime;

impl LanguageRuntime for GoRuntime {
    fn source_extension(&self) -> &'static str {
        "go"
    }

    fn compile_step(&self, source_path: &Path, work_dir: &Path) -> Option<(String, Vec<String>)> {
        let output = work_dir.join("bin");
        Some((
            "go".to_string(),
            vec![
                "build".to_string(),
                "-o".to_string(),
                output.display().to_string(),
                source_path.display().to_string(),
            ],
        ))
    }

    fn run_step(&self, _source_path: &Path, work_dir: &Path) -> (String, Vec<String>) {
        (work_dir.join("bin").display().to_string(), vec![])
    }
}
