use super::LanguageRuntime;
use std::path::Path;

pub struct NodeRuntime;

impl LanguageRuntime for NodeRuntime {
    fn source_extension(&self) -> &'static str {
        "js"
    }

    fn run_step(&self, source_path: &Path, _work_dir: &Path) -> (String, Vec<String>) {
        ("node".to_string(), vec![source_path.display().to_string()])
    }
}
