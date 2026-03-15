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
                r#"cp "$1" "$2/Main.java" && javac -d "$2" "$2/Main.java" && jar cfe "$2/bin.jar" Main -C "$2" ."#.to_string(),
                "sh".to_string(),
                source_path.display().to_string(),
                work_dir.display().to_string(),
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
