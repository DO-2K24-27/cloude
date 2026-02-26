use crate::runtimes::LanguageRuntime;

pub struct InitScriptGenerator;

impl InitScriptGenerator {
    pub fn generate_script(runtime: &dyn LanguageRuntime, code_path: &str) -> String {
        let mut script = String::from("#!/bin/sh\n\n");
        
        script.push_str("mount -t proc proc /proc\n");
        script.push_str("mount -t sysfs sysfs /sys\n");
        script.push_str("mount -t devtmpfs dev /dev\n\n");
        
        script.push_str("export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\n\n");
        
        script.push_str("echo '=== Cloude Agent Init ==='\n\n");
        
        if let Some(compile_cmd) = runtime.compile_command() {
            script.push_str("echo 'Compiling...'\n");
            script.push_str(&format!("{} || {{ echo 'Compilation failed'; sync; exit 1; }}\n", compile_cmd));
            script.push_str("echo 'Compilation successful'\n\n");
        }
        
        script.push_str("echo '--- PROGRAM OUTPUT ---'\n");
        
        let run_cmd = if let Some(exec_path) = runtime.execute_path() {
            exec_path.to_string()
        } else {
            format!("{} {}", runtime.run_command(), code_path)
        };
        
        script.push_str(&format!("{}\n", run_cmd));
        script.push_str("EXIT_CODE=$?\n");
        script.push_str("echo '--- END OUTPUT ---'\n");
        script.push_str("echo \"Exit code: $EXIT_CODE\"\n\n");
        
        script.push_str("sync\n");
        script.push_str("poweroff -f 2>/dev/null || exit $EXIT_CODE\n");
        
        script
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtimes::{node::NodeRuntime, python::PythonRuntime, rust::RustRuntime};

    #[test]
    fn test_python_script_generation() {
        let runtime = PythonRuntime;
        let script = InitScriptGenerator::generate_script(&runtime, "/lambda/code.py");
        assert!(script.contains("python3 /lambda/code.py"));
        assert!(!script.contains("Compiling..."));
    }

    #[test]
    fn test_node_script_generation() {
        let runtime = NodeRuntime;
        let script = InitScriptGenerator::generate_script(&runtime, "/lambda/code.js");
        assert!(script.contains("node /lambda/code.js"));
        assert!(!script.contains("Compiling..."));
    }

    #[test]
    fn test_rust_script_generation() {
        let runtime = RustRuntime;
        let script = InitScriptGenerator::generate_script(&runtime, "/lambda/code.rs");
        assert!(script.contains("rustc -o /lambda/bin /lambda/code.rs"));
        assert!(script.contains("Compiling..."));
        assert!(script.contains("/lambda/bin")); // Execution path
    }
}
