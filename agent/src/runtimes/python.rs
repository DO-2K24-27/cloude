use super::LanguageRuntime;
use std::env;
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

    fn run_candidates(&self, source_path: &Path, work_dir: &Path) -> Vec<(String, Vec<String>)> {
        let args = self.run_step(source_path, work_dir).1;
        let mut programs = vec!["python3".to_string()];

        if let Ok(fallback) = env::var("AGENT_PYTHON_FALLBACK") {
            programs.push(fallback);
        } else {
            programs.push("/usr/local/bin/python3".to_string());
            programs.push("/usr/bin/python3".to_string());
        }

        programs
            .into_iter()
            .map(|program| (program, args.clone()))
            .collect()
    }
}
