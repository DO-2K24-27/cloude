pub mod node;
pub mod python;
pub mod rust;

use std::path::Path;

pub trait LanguageRuntime: Send + Sync {
    fn source_extension(&self) -> &'static str;

    fn compile_step(&self, _source_path: &Path, _work_dir: &Path) -> Option<(String, Vec<String>)> {
        None
    }

    fn run_step(&self, source_path: &Path, work_dir: &Path) -> (String, Vec<String>);
}

pub type RuntimeBox = Box<dyn LanguageRuntime + Send + Sync>;

pub fn runtime_from_language(language: &str) -> Option<RuntimeBox> {
    match language.to_ascii_lowercase().as_str() {
        "python" | "py" => Some(Box::new(python::PythonRuntime)),
        "node" | "javascript" | "js" => Some(Box::new(node::NodeRuntime)),
        "rust" | "rs" => Some(Box::new(rust::RustRuntime)),
        _ => None,
    }
}
