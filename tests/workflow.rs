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
fn manifest_workflow_discovers_source_changes() {
    let workspace = Workspace::new();
    ok(&workspace.0, &["new", "demo", "--family", "Cyclone V"]);
    let root = workspace.0.join("demo");
    let src = root.join("src");
    assert!(src.join("demo.sv").exists());
    fs::write(src.join("extra module.v"), "module extra; endmodule\n").unwrap();
    let list = ok(&src, &["list"]);
    assert_eq!(list.lines().count(), 2);
    fs::write(src.join("new.v"), "module new_module; endmodule\n").unwrap();
    let list = ok(&src, &["list"]);
    assert_eq!(list.lines().count(), 3);
    assert!(list.contains("src/new.v"));
    assert_eq!(ok(&root, &["list"]), list);
    fs::remove_file(src.join("extra module.v")).unwrap();
    fs::remove_file(src.join("new.v")).unwrap();
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
    elif 'project_new' in script:
        pathlib.Path('generated-project.tcl').write_text(script)
elif sys.argv[1:3] == ['--flow', 'compile']:
    pathlib.Path('output_files').mkdir(exist_ok=True)
    if os.environ.get('QLM_TEST_FAIL_BUILD'):
        pathlib.Path('output_files/' + sys.argv[3] + '.sof').write_text('incomplete bitstream')
        sys.exit(3)
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
if sys.argv[1:] == ['-l']:
    if not os.environ.get('QLM_TEST_NO_CABLE'):
        print('1) USB-Blaster [USB-0]')
elif sys.argv[-1] == '-a':
    if os.environ.get('QLM_TEST_FAIL_DETECT'): sys.exit(6)
    print('10M08DAF256C8G')
elif os.environ.get('QLM_TEST_FAIL_PROGRAM'): sys.exit(5)
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
            .env("QLM_TEST_LOG", &self.log)
            .env("XDG_CONFIG_HOME", self.workspace.0.join(".qlm-test-config"));
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
    tools.ok(&["program"]);
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
            .command(&["build"])
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

#[test]
#[cfg(unix)]
fn build_detects_unset_hardware_and_syncs_sources_automatically() {
    let tools = FakeQuartus::new();
    let source = tools.root().join("src/extra.v");
    fs::write(&source, "module extra; endmodule\n").unwrap();
    let output = tools.ok(&["build"]);
    assert!(output.contains("Auto-syncing"));
    let manifest = fs::read_to_string(tools.root().join("Quartus.toml")).unwrap();
    assert!(manifest.contains("10M08DAF256C8G"));
    assert!(manifest.contains("USB-Blaster [USB-0]"));
    assert!(manifest.contains("src/extra.v"));
    let calls = fs::read_to_string(&tools.log).unwrap();
    assert!(calls.contains("pgm ['-l']"));
    assert!(calls.contains("pgm ['-c', 'USB-Blaster [USB-0]', '-a']"));
    assert!(!calls.contains("'-o'"));
    assert!(
        fs::read_to_string(tools.root().join("generated-project.tcl"))
            .unwrap()
            .contains("src/extra.v")
    );

    fs::remove_file(source).unwrap();
    fs::write(&tools.log, "").unwrap();
    tools.ok(&["build"]);
    assert!(
        !fs::read_to_string(tools.root().join("Quartus.toml"))
            .unwrap()
            .contains("src/extra.v")
    );
    assert!(
        !fs::read_to_string(tools.root().join("generated-project.tcl"))
            .unwrap()
            .contains("src/extra.v")
    );
    assert!(!fs::read_to_string(&tools.log).unwrap().contains("pgm "));
}

#[test]
#[cfg(unix)]
fn missing_hardware_detection_preserves_existing_selections() {
    let tools = FakeQuartus::new();
    tools.ok(&["device", "set", "10M08DAF256C8G"]);
    fs::write(&tools.log, "").unwrap();
    // Detect only the cable; an unavailable JTAG probe must not replace the target.
    assert!(
        tools
            .command(&["build"])
            .env("QLM_TEST_FAIL_DETECT", "1")
            .output()
            .unwrap()
            .status
            .success()
    );
    let calls = fs::read_to_string(&tools.log).unwrap();
    assert!(calls.contains("pgm ['-l']"));
    assert!(!calls.contains("'-a'"));

    let tools = FakeQuartus::new();
    tools.ok(&["cable", "set", "Saved cable", "--index", "2"]);
    tools.ok(&["build"]);
    let calls = fs::read_to_string(&tools.log).unwrap();
    assert!(!calls.contains("pgm ['-l']"));
    assert!(calls.contains("pgm ['-c', 'Saved cable', '-a']"));
    let manifest = fs::read_to_string(tools.root().join("Quartus.toml")).unwrap();
    assert!(manifest.contains("cable = \"Saved cable\""));
    assert!(manifest.contains("index = 2"));
}

#[test]
#[cfg(unix)]
fn program_builds_once_then_reuses_existing_bitstream() {
    let tools = FakeQuartus::new();
    let output = tools.ok(&["program"]);
    assert!(output.contains("Building automatically"));
    let calls = fs::read_to_string(&tools.log).unwrap();
    assert!(calls.find("'compile'").unwrap() < calls.find("'-o'").unwrap());
    fs::write(&tools.log, "").unwrap();
    fs::write(tools.root().join("src/demo.sv"), "changed source").unwrap();
    let result = tools
        .command(&["program"])
        .env("QLM_TEST_FAIL_BUILD", "1")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("Using existing build"));
    let calls = fs::read_to_string(&tools.log).unwrap();
    assert!(!calls.contains("sh "));
    assert!(calls.contains("'-o'"));
}

#[test]
#[cfg(unix)]
fn failed_detection_or_automatic_build_never_programs() {
    for variable in [
        "QLM_TEST_NO_CABLE",
        "QLM_TEST_FAIL_DETECT",
        "QLM_TEST_FAIL_BUILD",
    ] {
        let tools = FakeQuartus::new();
        let before = fs::read(tools.root().join("Quartus.toml")).unwrap();
        assert!(
            !tools
                .command(&["program"])
                .env(variable, "1")
                .output()
                .unwrap()
                .status
                .success()
        );
        let calls = fs::read_to_string(&tools.log).unwrap();
        assert!(!calls.contains("'-o'"));
        assert!(!tools.root().join("output_files/demo.sof").exists());
        if variable != "QLM_TEST_FAIL_BUILD" {
            assert!(!calls.contains("'compile'"));
            assert_eq!(before, fs::read(tools.root().join("Quartus.toml")).unwrap());
        }
    }
}

#[test]
fn retired_commands_are_absent_from_help_and_rejected() {
    let workspace = Workspace::new();
    let help = ok(&workspace.0, &["help"]);
    assert!(help.contains("Auto-sync"));
    for args in [
        &["add", "source.v"][..],
        &["add", "--auto"],
        &["a", "source.v"],
        &["remove", "source.v"],
        &["r", "source.v"],
        &["sync"],
        &["s"],
        &["program", "build"],
        &["program", "b"],
    ] {
        assert!(!run(&workspace.0, args).status.success());
    }
    for text in [
        help,
        ok(&workspace.0, &["completions", "bash", "--print"]),
        ok(&workspace.0, &["completions", "zsh", "--print"]),
        ok(&workspace.0, &["completions", "fish", "--print"]),
    ] {
        for retired in [
            "qlm add",
            "init i add a",
            "add:add HDL files",
            "a:add HDL files",
            "qlm remove",
            "qlm sync",
            "qlm program build",
            "remove r",
            " sync s ",
            "build b\"",
            "'action' build b",
            "program p' -a 'build b'",
        ] {
            assert!(!text.contains(retired), "Found retired command {retired}");
        }
    }
}

#[test]
fn completions_install_by_default_instead_of_printing_script() {
    let workspace = Workspace::new();
    let output = ok(&workspace.0, &["completions", "fish"]);
    let path = workspace
        .0
        .join(".qlm-test-config/fish/completions/qlm.fish");
    assert!(path.is_file());
    assert!(output.contains("Saved fish completions"));
    assert!(output.contains("Automatic loading enabled"));
    assert!(!output.contains("complete -c"));
    let script = fs::read_to_string(&path).unwrap();
    assert_eq!(
        script,
        ok(&workspace.0, &["completions", "fish", "--print"])
    );
    ok(&workspace.0, &["completions", "fish"]);
    assert_eq!(script, fs::read_to_string(&path).unwrap());
    let output = ok(&workspace.0, &["completions", "fish", "--remove"]);
    assert!(output.contains("Removed fish completions"));
    assert!(!path.exists());
    assert!(
        ok(&workspace.0, &["completions", "remove", "fish"])
            .contains("No installed fish completions")
    );
    assert!(
        !run(&workspace.0, &["completions", "invalid"])
            .status
            .success()
    );
}
