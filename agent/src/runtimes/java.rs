use super::LanguageRuntime;

pub struct JavaRuntime;

impl LanguageRuntime for JavaRuntime {
    fn base_image(&self) -> &'static str {
        "openjdk:21-alpine"
    }

    fn run_command(&self) -> &'static str {
        "java"
    }

    fn source_extension(&self) -> &'static str {
        "java"
    }

    fn compile_command(&self) -> Option<&'static str> {
        Some("javac -d /lambda /lambda/code.java && jar cfe /lambda/bin.jar Main -C /lambda .")
    }

    fn execute_path(&self) -> Option<&'static str> {
        Some("java -jar /lambda/bin.jar")
    }
}
