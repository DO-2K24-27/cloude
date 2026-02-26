pub mod python;
pub mod node;
pub mod rust;

pub trait LanguageRuntime {
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

pub fn detect_runtime<P: AsRef<std::path::Path>>(path: P) -> Option<Box<dyn LanguageRuntime>> {
    let ext = path.as_ref().extension()?.to_str()?;
    match ext {
        "py" => Some(Box::new(python::PythonRuntime)),
        "js" => Some(Box::new(node::NodeRuntime)),
        "rs" => Some(Box::new(rust::RustRuntime)),
        _ => None,
    }
}
