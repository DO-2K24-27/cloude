use super::LanguageRuntime;

pub struct NodeRuntime;

impl LanguageRuntime for NodeRuntime {
    fn base_image(&self) -> &'static str {
        "node:20-alpine"
    }

    fn run_command(&self) -> &'static str {
        "node"
    }

    fn source_extension(&self) -> &'static str {
        "js"
    }
}
