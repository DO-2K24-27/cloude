pub mod c;
pub mod cpp;
pub mod go;
pub mod java;
pub mod node;
pub mod python;
pub mod rust;

pub trait LanguageRuntime: Send + Sync {
    fn base_image(&self) -> &'static str;

    fn run_command(&self) -> &'static str;

    fn source_extension(&self) -> &'static str;

    fn compile_command(&self) -> Option<&'static str> {
        None
    }

    fn execute_path(&self) -> Option<&'static str> {
        None
    }
}

pub type RuntimeBox = Box<dyn LanguageRuntime + Send + Sync>;

pub fn runtime_from_language(language: &str) -> Option<RuntimeBox> {
    match language.to_ascii_lowercase().as_str() {
        "python" | "py" => Some(Box::new(python::PythonRuntime)),
        "node" | "javascript" | "js" => Some(Box::new(node::NodeRuntime)),
        "rust" | "rs" => Some(Box::new(rust::RustRuntime)),
        "go" | "golang" => Some(Box::new(go::GoRuntime)),
        "java" => Some(Box::new(java::JavaRuntime)),
        "c" => Some(Box::new(c::CRuntime)),
        "cpp" | "c++" => Some(Box::new(cpp::CppRuntime)),
        _ => None,
    }
}
