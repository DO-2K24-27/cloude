use super::LanguageRuntime;
use std::path::Path;

pub struct PythonRuntime;

impl LanguageRuntime for PythonRuntime {
    fn source_extension(&self) -> &'static str {
        "py"
    }

    fn run_step(&self, source_path: &Path, _work_dir: &Path) -> (String, Vec<String>) {
        (
            "python3".to_string(),
            vec![source_path.display().to_string()],
        )
    }
}
