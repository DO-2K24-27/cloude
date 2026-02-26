use super::LanguageRuntime;

pub struct RustRuntime;

impl LanguageRuntime for RustRuntime {
    fn base_image(&self) -> &'static str {
        "rust:alpine"
    }

    fn run_command(&self) -> &'static str {
        "/lambda/bin"
    }

    fn source_extension(&self) -> &'static str {
        "rs"
    }

    fn compile_command(&self) -> Option<&'static str> {
        Some("rustc -o /lambda/bin /lambda/code.rs")
    }

    fn execute_path(&self) -> Option<&'static str> {
        Some("/lambda/bin")
    }
}
