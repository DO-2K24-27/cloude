use super::LanguageRuntime;

pub struct GoRuntime;

impl LanguageRuntime for GoRuntime {
    fn base_image(&self) -> &'static str {
        "golang:alpine"
    }

    fn run_command(&self) -> &'static str {
        "/lambda/bin"
    }

    fn source_extension(&self) -> &'static str {
        "go"
    }

    fn compile_command(&self) -> Option<&'static str> {
        Some("go build -o /lambda/bin /lambda/code.go")
    }

    fn execute_path(&self) -> Option<&'static str> {
        Some("/lambda/bin")
    }
}
