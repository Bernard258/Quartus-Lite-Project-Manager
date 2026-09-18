use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

struct Workspace(std::path::PathBuf);
impl Workspace {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("qlm-test-{}-{stamp}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_qlm"))
        .current_dir(cwd)
        .args(args)
        .env("XDG_CONFIG_HOME", cwd.join(".qlm-test-config"))
        .output()
        .unwrap()
}
fn ok(cwd: &Path, args: &[&str]) -> String {
    let result = run(cwd, args);
    assert!(
        result.status.success(),
        "{args:?}: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}
#[test]
fn manifest_workflow_and_batch_validation() {
    let workspace = Workspace::new();
    ok(&workspace.0, &["new", "demo", "--family", "Cyclone V"]);
    let root = workspace.0.join("demo");
    let src = root.join("src");
    assert!(src.join("demo.sv").exists());
    fs::write(src.join("extra module.v"), "module extra; endmodule\n").unwrap();
    ok(&src, &["add", "extra module.v"]);
    ok(&root, &["add", "src/extra module.v"]);
    let list = ok(&src, &["list"]);
    assert_eq!(list.lines().count(), 2);
    let before = fs::read(root.join("Quartus.toml")).unwrap();
    assert!(
        !run(&root, &["remove", "src/extra module.v", "missing.sv"])
            .status
            .success()
    );
    assert_eq!(before, fs::read(root.join("Quartus.toml")).unwrap());
    assert!(!run(&root, &["add", "missing.sv"]).status.success());
    ok(&src, &["remove", "extra module.v"]);
    assert!(src.join("extra module.v").exists());
    assert_eq!(ok(&root, &["list"]), "src/demo.sv\n");
    assert!(
        !run(&workspace.0, &["new", "demo", "--family", "Cyclone V"])
            .status
            .success()
    );
}
#[test]
fn init_preserves_existing_files_and_supports_languages() {
    for language in ["verilog", "vhdl", "systemverilog"] {
        let workspace = Workspace::new();
        fs::write(workspace.0.join(".gitignore"), "keep-me").unwrap();
        ok(
            &workspace.0,
            &[
                "init",
                "--name",
                "demo",
                "--family",
                "Cyclone V",
                "--lang",
                language,
            ],
        );
        assert_eq!(
            fs::read_to_string(workspace.0.join(".gitignore")).unwrap(),
            "keep-me\n/.qlm/\n/output_files/\n/*.qpf\n/*.qsf\n"
        );
        assert!(
            !run(
                &workspace.0,
                &["init", "--name", "demo", "--family", "Cyclone V"]
            )
            .status
            .success()
        );
    }
}
#[test]
#[ignore = "requires installed Quartus Lite; run cargo test -- --ignored"]
fn real_quartus_sync() {
    let workspace = Workspace::new();
    ok(&workspace.0, &["new", "demo", "--family", "Cyclone V"]);
    let root = workspace.0.join("demo");
    // Tcl substitutions in filenames must remain literal.
    let source = "src/extra $value [literal].v";
    fs::write(root.join(source), "module extra; endmodule\n").unwrap();
    ok(&root, &["add", source]);
    fs::write(
        root.join("constraints.tcl"),
        "set_global_assignment -name SEED 7\n",
    )
    .unwrap();
    ok(&root, &["sync"]);
    let qsf = root.join("demo.qsf");
    let text = fs::read_to_string(&qsf).unwrap();
    assert!(root.join("demo.qpf").exists());
    assert!(text.contains("SYSTEMVERILOG_FILE"));
    assert!(text.contains("SEED 7"));
    assert!(text.contains("VERILOG_FILE"));
    assert!(text.contains("literal"));
    ok(&root, &["remove", source]);
    ok(&root, &["sync"]);
    let text = fs::read_to_string(qsf).unwrap();
    assert!(!text.contains("literal"), "stale source was not removed");
    assert!(root.join(source).exists());
}

#[cfg(unix)]
struct FakeQuartus {
    workspace: Workspace,
    shell: std::path::PathBuf,
    programmer: std::path::PathBuf,
    log: std::path::PathBuf,
}
#[cfg(unix)]
impl FakeQuartus {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let workspace = Workspace::new();
        let shell = workspace.0.join("quartus_sh");
        let programmer = workspace.0.join("quartus_pgm");
        let log = workspace.0.join("calls.log");
        fs::write(
            &shell,
            r#"#!/usr/bin/python3
import os, sys, pathlib
with open(os.environ['QLM_TEST_LOG'], 'a') as f: f.write('sh ' + repr(sys.argv[1:]) + '\n')
if sys.argv[1] == '-t':
    script = pathlib.Path(sys.argv[2]).read_text()
    if 'set requested' in script:
        if 'INVALID' in script: sys.exit(2)
        print('QLM:10M08DAF256C8G')
        print('QLM:MAX 10')
    elif 'foreach part' in script:
        print('QLM:10M08DAF256C8G')
    elif 'foreach family' in script:
        print('QLM:MAX 10')
elif sys.argv[1:3] == ['--flow', 'compile']:
    if os.environ.get('QLM_TEST_FAIL_BUILD'): sys.exit(3)
    pathlib.Path('output_files').mkdir(exist_ok=True)
    if not os.environ.get('QLM_TEST_NO_SOF'):
        pathlib.Path('output_files/' + sys.argv[3] + '.sof').write_text('test bitstream')
else: sys.exit(4)
"#,
        )
        .unwrap();
        fs::write(
            &programmer,
            r#"#!/usr/bin/python3
import os, sys
with open(os.environ['QLM_TEST_LOG'], 'a') as f: f.write('pgm ' + repr(sys.argv[1:]) + '\n')
if os.environ.get('QLM_TEST_FAIL_PROGRAM'): sys.exit(5)
"#,
        )
        .unwrap();
        for path in [&shell, &programmer] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        // Include operation delimiters in the project parent path.
        let parent = workspace.0.join("parent ; @ space");
        fs::create_dir(&parent).unwrap();
        ok(&parent, &["new", "demo"]);
        Self {
            workspace,
            shell,
            programmer,
            log,
        }
    }
    fn root(&self) -> std::path::PathBuf {
        self.workspace.0.join("parent ; @ space/demo")
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_qlm"));
        cmd.current_dir(self.root().join("src"))
            .args(args)
            .env("QUARTUS_SH", &self.shell)
            .env("QUARTUS_PGM", &self.programmer)
            .env("QLM_TEST_LOG", &self.log);
        cmd
    }
    fn ok(&self, args: &[&str]) -> String {
        let result = self.command(args).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    }
}
#[test]
#[cfg(unix)]
fn device_build_and_program_use_saved_project_context() {
    let tools = FakeQuartus::new();
    assert!(!tools.command(&["build"]).output().unwrap().status.success());
    tools.ok(&["device", "set", "10M08DAF256C8G"]);
    assert!(tools.ok(&["device", "show"]).contains("MAX 10"));
    assert!(tools.ok(&["device", "list", "10m08"]).contains("10M08"));
    let before = fs::read(tools.root().join("Quartus.toml")).unwrap();
    assert!(
        !tools
            .command(&["device", "set", "INVALID"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(before, fs::read(tools.root().join("Quartus.toml")).unwrap());
    assert!(
        !tools
            .command(&["cable", "set", "1", "--index", "0"])
            .output()
            .unwrap()
            .status
            .success()
    );
    tools.ok(&["cable", "set", "USB-Blaster [USB-0]", "--index", "2"]);
    tools.ok(&["program", "build"]);
    let calls = fs::read_to_string(&tools.log).unwrap();
    assert!(calls.contains("['--flow', 'compile', 'demo']"));
    assert!(
        calls.contains("pgm ['-c', 'USB-Blaster [USB-0]', '-m', 'jtag', '-o', 'p;demo.sof@2']")
    );
    assert!(tools.root().join("output_files/demo.sof").exists());
}
#[test]
#[cfg(unix)]
fn program_stops_on_build_failure_or_missing_bitstream() {
    let tools = FakeQuartus::new();
    tools.ok(&["device", "set", "10M08DAF256C8G"]);
    tools.ok(&["cable", "set", "1"]);
    tools.ok(&["build"]);
    assert!(
        !tools
            .command(&["program", "build"])
            .env("QLM_TEST_FAIL_BUILD", "1")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!tools.root().join("output_files/demo.sof").exists());
    assert!(
        !tools
            .command(&["program"])
            .env("QLM_TEST_NO_SOF", "1")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!fs::read_to_string(&tools.log).unwrap().contains("pgm "));
    assert!(
        !tools
            .command(&["program"])
            .env("QLM_TEST_FAIL_PROGRAM", "1")
            .output()
            .unwrap()
            .status
            .success()
    );
}
