use super::LanguageRuntime;

pub struct CRuntime;

impl LanguageRuntime for CRuntime {
    fn base_image(&self) -> &'static str {
        "gcc:alpine"
    }

    fn run_command(&self) -> &'static str {
        "/lambda/bin"
    }

    fn source_extension(&self) -> &'static str {
        "c"
    }

    fn compile_command(&self) -> Option<&'static str> {
        Some("gcc -o /lambda/bin /lambda/code.c")
    }

    fn execute_path(&self) -> Option<&'static str> {
        Some("/lambda/bin")
    }
}
