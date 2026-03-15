use super::LanguageRuntime;
use std::path::Path;

pub struct JavaRuntime;

impl LanguageRuntime for JavaRuntime {
    fn source_extension(&self) -> &'static str {
        "java"
    }

    fn compile_step(&self, source_path: &Path, work_dir: &Path) -> Option<(String, Vec<String>)> {
        Some((
            "sh".to_string(),
            vec![
                "-c".to_string(),
                format!(
                    "cp {} {}/Main.java && javac -d {} {}/Main.java && jar cfe {}/bin.jar Main -C {} .",
                    source_path.display(),
                    work_dir.display(),
                    work_dir.display(),
                    work_dir.display(),
                    work_dir.display(),
                    work_dir.display()
                ),
            ],
        ))
    }

    fn run_step(&self, _source_path: &Path, work_dir: &Path) -> (String, Vec<String>) {
        (
            "java".to_string(),
            vec![
                "-jar".to_string(),
                work_dir.join("bin.jar").display().to_string(),
            ],
        )
    }
}
