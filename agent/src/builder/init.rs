use crate::runtimes::LanguageRuntime;

/// Generates the `/init` shell script that runs as PID 1 inside the VM.
///
/// The script mounts the essential pseudo-filesystems (proc, sysfs, devtmpfs),
/// runs the user's code, and shuts the VM down cleanly via `poweroff -f`.
/// Output is wrapped between `--- PROGRAM OUTPUT ---` / `--- END OUTPUT ---`
/// markers so the agent can reliably extract it from the serial console stream.
pub struct InitScriptGenerator;

impl InitScriptGenerator {
    /// Build the init script for a given runtime and source file path.
    ///
    /// For compiled languages, a compile step runs first — if it fails the VM
    /// exits immediately without printing misleading output markers.
    pub fn generate_script(runtime: &dyn LanguageRuntime, code_path: &str) -> String {
        let mut script = String::from("#!/bin/sh\n\n");

        script.push_str("mount -t proc proc /proc\n");
        script.push_str("mount -t sysfs sysfs /sys\n");
        script.push_str("mount -t devtmpfs dev /dev\n\n");
        script.push_str(
            "export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\n\n",
        );

        if let Some(compile_cmd) = runtime.compile_command() {
            script.push_str(&format!("{}\n", compile_cmd));
            script.push_str("COMPILE_EXIT=$?\n");
            script.push_str("if [ $COMPILE_EXIT -ne 0 ]; then\n");
            script.push_str("  echo '--- PROGRAM OUTPUT ---'\n");
            script.push_str("  echo '--- END OUTPUT ---'\n");
            script.push_str("  echo \"Exit code: $COMPILE_EXIT\"\n");
            script.push_str("  poweroff -f 2>/dev/null || exit $COMPILE_EXIT\n");
            script.push_str("fi\n");
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

        script.push_str("poweroff -f 2>/dev/null || exit $EXIT_CODE\n");

        script
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtimes::{
        c::CRuntime, cpp::CppRuntime, go::GoRuntime, java::JavaRuntime, node::NodeRuntime,
        python::PythonRuntime, rust::RustRuntime,
    };

    // Python: interpreted runtime, direct run, no compile step
    #[test]
    fn test_python_script_generation() {
        let runtime = PythonRuntime;
        let script = InitScriptGenerator::generate_script(&runtime, "/lambda/code.py");
        assert!(script.contains("python3 /lambda/code.py"));
        assert!(script.contains("--- PROGRAM OUTPUT ---"));
    }

    // Node: same as Python, no compile step
    #[test]
    fn test_node_script_generation() {
        let runtime = NodeRuntime;
        let script = InitScriptGenerator::generate_script(&runtime, "/lambda/code.js");
        assert!(script.contains("node /lambda/code.js"));
        assert!(script.contains("--- PROGRAM OUTPUT ---"));
    }

    // Rust: compiled runtime, checks compile exit code before running the binary
    #[test]
    fn test_rust_script_generation() {
        let runtime = RustRuntime;
        let script = InitScriptGenerator::generate_script(&runtime, "/lambda/code.rs");
        assert!(script.contains("rustc -o /lambda/bin /lambda/code.rs"));
        assert!(script.contains("COMPILE_EXIT=$?"));
        assert!(script.contains("if [ $COMPILE_EXIT -ne 0 ]; then"));
        assert!(script.contains("/lambda/bin"));
    }

    // C: compiled with gcc, same compile-guard pattern as Rust
    #[test]
    fn test_c_script_generation() {
        let runtime = CRuntime;
        let script = InitScriptGenerator::generate_script(&runtime, "/lambda/code.c");
        assert!(script.contains("gcc -o /lambda/bin /lambda/code.c"));
        assert!(script.contains("COMPILE_EXIT=$?"));
        assert!(script.contains("if [ $COMPILE_EXIT -ne 0 ]; then"));
        assert!(script.contains("/lambda/bin"));
    }

    // C++: compiled with g++, same compile-guard pattern as C
    #[test]
    fn test_cpp_script_generation() {
        let runtime = CppRuntime;
        let script = InitScriptGenerator::generate_script(&runtime, "/lambda/code.cpp");
        assert!(script.contains("g++ -o /lambda/bin /lambda/code.cpp"));
        assert!(script.contains("COMPILE_EXIT=$?"));
        assert!(script.contains("if [ $COMPILE_EXIT -ne 0 ]; then"));
        assert!(script.contains("/lambda/bin"));
    }

    // Go: compiled with `go build`, runs the resulting binary
    #[test]
    fn test_go_script_generation() {
        let runtime = GoRuntime;
        let script = InitScriptGenerator::generate_script(&runtime, "/lambda/code.go");
        assert!(script.contains("go build -o /lambda/bin /lambda/code.go"));
        assert!(script.contains("COMPILE_EXIT=$?"));
        assert!(script.contains("if [ $COMPILE_EXIT -ne 0 ]; then"));
        assert!(script.contains("/lambda/bin"));
    }

    // Java: compile step renames the file, compiles with javac, packages a jar, runs with `java -jar`
    #[test]
    fn test_java_script_generation() {
        let runtime = JavaRuntime;
        let script = InitScriptGenerator::generate_script(&runtime, "/lambda/code.java");
        assert!(script.contains("mv /lambda/code.java /lambda/Main.java"));
        assert!(script.contains("javac -d /lambda /lambda/Main.java"));
        assert!(script.contains("jar cfe /lambda/bin.jar Main"));
        assert!(script.contains("COMPILE_EXIT=$?"));
        assert!(script.contains("if [ $COMPILE_EXIT -ne 0 ]; then"));
        assert!(script.contains("java -jar /lambda/bin.jar"));
    }
}
