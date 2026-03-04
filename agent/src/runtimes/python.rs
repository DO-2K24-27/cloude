use super::LanguageRuntime;

pub struct PythonRuntime;

impl LanguageRuntime for PythonRuntime {
    fn base_image(&self) -> &'static str {
        "python:3.12-alpine"
    }

    fn run_command(&self) -> &'static str {
        "python3"
    }

    fn source_extension(&self) -> &'static str {
        "py"
    }
}
