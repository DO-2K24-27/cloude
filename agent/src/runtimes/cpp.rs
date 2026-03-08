use super::LanguageRuntime;

pub struct CppRuntime;

impl LanguageRuntime for CppRuntime {
    fn base_image(&self) -> &'static str {
        "gcc:alpine"
    }

    fn run_command(&self) -> &'static str {
        "/lambda/bin"
    }

    fn source_extension(&self) -> &'static str {
        "cpp"
    }

    fn compile_command(&self) -> Option<&'static str> {
        Some("g++ -o /lambda/bin /lambda/code.cpp")
    }

    fn execute_path(&self) -> Option<&'static str> {
        Some("/lambda/bin")
    }
}
