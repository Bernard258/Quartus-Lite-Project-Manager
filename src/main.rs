use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env, fs,
    io::{IsTerminal, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const MANIFEST: &str = "Quartus.toml";
const DE10_LITE_PART: &str = "10M50DAF484C7G";
const DE10_LITE_CABLE: &str = "USB-Blaster [USB-0]";
const HELP: &str = "qlm — Quartus Lite project manager

PROJECTS
  qlm new (n) [dir] [options]   Create a project
  qlm init (i) [options]        Initialize the current directory
  qlm add (a) <file>...         Register HDL files (or `--auto`)
  qlm remove (r) <file>...      Unregister HDL files
  qlm list (l)                  List registered sources
  qlm sync (s)                  Generate Quartus project files
  qlm build (b)                 Compile without programming
  qlm program (p)               Program the existing .sof
  qlm program build             Build, then program the FPGA

HARDWARE
  qlm device (d) <command>      Device commands
  qlm cable (c) <command>       Cable commands
  qlm pin <command>             Pin commands

SETTINGS AND TOOLS
  qlm config <command>          Settings commands
  qlm completions <shell>       Generate Bash, Zsh, or Fish completion
  qlm tui                       Open the optional terminal UI

Run `qlm <command> help` for the commands in that group. Quartus must be on PATH.
";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    project: Project,
    sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    settings: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    programmer: Option<Programmer>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Project {
    name: String,
    #[serde(default)]
    family: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    board: Option<String>,
    top: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Programmer {
    cable: String,
    #[serde(default = "first_device")]
    index: u32,
}
fn first_device() -> u32 {
    1
}

#[derive(Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct UserSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    board: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cable: Option<String>,
    #[serde(default = "first_device")]
    jtag_index: u32,
    #[serde(default = "default_language")]
    language: String,
}

fn default_language() -> String {
    "systemverilog".to_owned()
}

fn settings_path() -> Result<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or("could not determine config directory; set XDG_CONFIG_HOME")?;
    Ok(base.join("qlm/settings.toml"))
}

fn load_settings() -> Result<UserSettings> {
    let path = settings_path()?;
    if !path.is_file() {
        return Ok(UserSettings {
            jtag_index: 1,
            language: default_language(),
            ..Default::default()
        });
    }
    Ok(toml::from_str(&fs::read_to_string(path)?)?)
}

fn save_settings(settings: &UserSettings) -> Result<PathBuf> {
    let path = settings_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, toml::to_string_pretty(settings)?)?;
    Ok(path)
}

fn main() {
    if let Err(error) = run(env::args().skip(1).collect()) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
fn run(args: Vec<String>) -> Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        print!("{HELP}");
        return Ok(());
    };
    let command = match command {
        "n" => "new",
        "i" => "init",
        "a" => "add",
        "r" => "remove",
        "l" => "list",
        "s" => "sync",
        "d" => "device",
        "c" => "cable",
        "b" => "build",
        "p" => "program",
        "h" => "help",
        command => command,
    };
    match command {
        "--help" | "-h" | "help" if args.len() == 1 => print!("{HELP}"),
        "--version" if args.len() == 1 => println!("qlm {}", env!("CARGO_PKG_VERSION")),
        "new" => create(
            &args[1..],
            args.get(1).is_none_or(|arg| arg.starts_with('-')),
        )?,
        "init" => create(&args[1..], true)?,
        "add" | "remove" if args.len() >= 2 => sources(command, &args[1..])?,
        "list" if args.len() == 1 => {
            let (_, state) = discover()?;
            for source in state.sources {
                println!("{source}");
            }
        }
        "sync" if args.len() == 1 => sync()?,
        "device" => device(&args[1..])?,
        "cable" => cable(&args[1..])?,
        "pin" => pin(&args[1..])?,
        "completions" => completions(&args[1..])?,
        "tui" => tui(&args[1..])?,
        "config" => config(&args[1..])?,
        "build" if args.len() == 1 => {
            build()?;
        }
        "program" => program(&args[1..])?,
        _ => return Err(format!("invalid command or arguments\n\n{HELP}").into()),
    }
    Ok(())
}

fn config(args: &[String]) -> Result<()> {
    match args {
        [action] if action == "help" || action == "h" => println!(
            "CONFIG COMMANDS\n  qlm config show (s)\n  qlm config path (p)\n  qlm config edit (e)\n  qlm config set <board|cable|jtag-index|language> <value>"
        ),
        [] => println!(
            "CONFIG COMMANDS\n  qlm config show (s)\n  qlm config path (p)\n  qlm config edit (e)\n  qlm config set <board|cable|jtag-index|language> <value>"
        ),
        [action] if action == "show" || action == "s" => {
            let path = settings_path()?;
            let settings = load_settings()?;
            println!("{}", path.display());
            print!("{}", toml::to_string_pretty(&settings)?);
        }
        [action] if action == "path" || action == "p" => println!("{}", settings_path()?.display()),
        [action] if action == "edit" || action == "e" => {
            let settings = load_settings()?;
            let path = save_settings(&settings)?;
            let editor = env::var_os("VISUAL")
                .or_else(|| env::var_os("EDITOR"))
                .unwrap_or_else(|| "vi".into());
            let status = Command::new(&editor)
                .arg(&path)
                .status()
                .map_err(|error| format!("could not open settings editor {:?}: {error}", editor))?;
            if !status.success() {
                return Err(format!("settings editor failed ({status})").into());
            }
        }
        [action, key, value] if action == "set" => {
            let mut settings = load_settings()?;
            match key.as_str() {
                "board" => {
                    if value.trim().is_empty() {
                        return Err("board cannot be empty".into());
                    }
                    settings.board = Some(value.clone());
                    if settings.cable.is_none() && is_de10_lite(value) {
                        settings.cable = Some(DE10_LITE_CABLE.to_owned());
                    }
                }
                "cable" => {
                    if value.trim().is_empty() {
                        return Err("cable cannot be empty".into());
                    }
                    settings.cable = Some(if is_de10_lite(value) {
                        DE10_LITE_CABLE.to_owned()
                    } else {
                        value.clone()
                    });
                }
                "jtag-index" => {
                    settings.jtag_index = value.parse()?;
                    if settings.jtag_index == 0 {
                        return Err("JTAG position starts at 1".into());
                    }
                }
                "language" | "default-language" => {
                    if !matches!(value.as_str(), "systemverilog" | "verilog" | "vhdl") {
                        return Err("language must be systemverilog, verilog, or vhdl".into());
                    }
                    settings.language = value.clone();
                }
                _ => {
                    return Err(
                        "settings keys: board, cable, jtag-index, language (or default-language)"
                            .into(),
                    );
                }
            }
            let path = save_settings(&settings)?;
            println!("Saved {}", path.display());
        }
        _ => {
            return Err(
                "usage: qlm config [show | path | edit | set <board|cable|jtag-index|language> <value>]"
                    .into(),
            );
        }
    }
    Ok(())
}
// Tcl double-quoted words: escape every substitution and word terminator.
fn quote(value: &str) -> String {
    let mut result = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' | '"' | '$' | '[' | ']' => {
                result.push('\\');
                result.push(ch);
            }
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            _ => result.push(ch),
        }
    }
    result.push('"');
    result
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| "paths must be valid UTF-8".into())
}

fn identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn create(args: &[String], init: bool) -> Result<()> {
    let (path, options) = if init {
        (env::current_dir()?, args)
    } else {
        (
            absolute(Path::new(args.first().ok_or("new requires a directory")?))?,
            &args[1..],
        )
    };
    let mut name = path
        .file_name()
        .and_then(|p| p.to_str())
        .ok_or("invalid directory name")?
        .replace('-', "_");
    let mut family = None;
    let mut device = None;
    let mut top = None;
    let mut language = default_language();
    if let Ok(settings_path) = settings_path()
        && settings_path.is_file()
    {
        language = load_settings()?.language;
    }
    let mut seen = std::collections::HashSet::new();
    let mut options = options.iter();
    while let Some(option) = options.next() {
        if !seen.insert(option) {
            return Err(format!("repeated option: {option}").into());
        }
        let value = options
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        if value.is_empty() || value.starts_with("--") {
            return Err(format!("missing value for {option}").into());
        }
        match option.as_str() {
            "--family" => family = Some(value.clone()),
            "--device" => device = Some(value.clone()),
            "--name" => name = value.clone(),
            "--top" => top = Some(value.clone()),
            "--lang" => language = value.clone(),
            _ => return Err(format!("unknown option: {option}").into()),
        }
    }
    let mut family = family.unwrap_or_default();
    if let Some(part) = &device {
        let (canonical, detected_family) = selected_part(&env::current_dir()?, part)?;
        if !family.is_empty() && family != detected_family {
            return Err("--family does not match --device".into());
        }
        family = detected_family;
        device = Some(canonical);
    }
    let top = top.unwrap_or_else(|| name.clone());
    if !identifier(&name) || !identifier(&top) {
        return Err("project name and top must be HDL identifiers (letters, digits, underscores; no leading digit)".into());
    }
    let (extension, template) = match language.as_str() {
        "systemverilog" => (
            "sv",
            format!(
                "module {top} (\n    input logic clk,\n    output logic led\n);\n    assign led = clk;\nendmodule\n"
            ),
        ),
        "verilog" => (
            "v",
            format!(
                "module {top} (\n    input wire clk,\n    output wire led\n);\n    assign led = clk;\nendmodule\n"
            ),
        ),
        "vhdl" => (
            "vhd",
            format!(
                "library ieee;\nuse ieee.std_logic_1164.all;\n\nentity {top} is\n    port (clk : in std_logic; led : out std_logic);\nend entity;\n\narchitecture rtl of {top} is\nbegin\n    led <= clk;\nend architecture;\n"
            ),
        ),
        _ => return Err("--lang must be systemverilog, verilog, or vhdl".into()),
    };
    let source = format!("src/{top}.{extension}");
    let mut state = Manifest {
        version: 1,
        project: Project {
            name,
            family,
            device,
            board: None,
            top,
        },
        sources: vec![source.clone()],
        settings: Some("constraints.tcl".to_owned()),
        programmer: None,
    };
    if let Ok(settings) = load_settings()
        && settings.board.as_deref().is_some_and(is_de10_lite)
    {
        state.project.board = Some("de10-lite".to_owned());
    }
    if init {
        for target in [MANIFEST, source.as_str(), "constraints.tcl"] {
            if path.join(target).exists() {
                return Err(format!("refusing to overwrite {target}").into());
            }
        }
    } else {
        fs::create_dir(&path)?;
    }
    fs::create_dir_all(path.join("src"))?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.join(&source))?
        .write_all(template.as_bytes())?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.join(MANIFEST))?
        .write_all(toml::to_string_pretty(&state)?.as_bytes())?;
    fs::write(
        path.join("constraints.tcl"),
        "# Board-specific pin and timing assignments. Paths are relative to the project root.\n# set_location_assignment PIN_A1 -to clk\n# set_instance_assignment -name IO_STANDARD \"3.3-V LVTTL\" -to clk\n# set_global_assignment -name SDC_FILE [file normalize constraints.sdc]\n",
    )?;
    apply_default_board_pins(&path, &state)?;
    let ignore = path.join(".gitignore");
    let mut old = if ignore.exists() {
        fs::read_to_string(&ignore)?
    } else {
        String::new()
    };
    for entry in ["/.qlm/", "/output_files/", "/*.qpf", "/*.qsf"] {
        if old.lines().any(|line| line == entry) {
            continue;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&ignore)?;
        if !old.is_empty() && !old.ends_with('\n') {
            writeln!(file)?;
        }
        writeln!(file, "{entry}")?;
        old.push_str(entry);
        old.push('\n');
    }
    println!(
        "Created {} in {}\nEdit {source} and constraints.tcl, select a target with qlm device set <part>, then run qlm build.",
        state.project.name,
        path.display()
    );
    offer_hardware_setup(&path, &mut state)?;
    Ok(())
}

fn validate(state: &Manifest) -> Result<()> {
    if state.version != 1 {
        return Err("unsupported Quartus.toml version; expected 1".into());
    }
    if !identifier(&state.project.name) || !identifier(&state.project.top) {
        return Err(
            "invalid project name, top-level entity, or FPGA family in Quartus.toml".into(),
        );
    }
    if let Some(programmer) = &state.programmer
        && (programmer.cable.trim().is_empty() || programmer.index == 0)
    {
        return Err("programmer cable must be nonempty and JTAG index must be at least 1".into());
    }
    if let Some(settings) = &state.settings
        && (settings.is_empty() || Path::new(settings).is_absolute())
    {
        return Err("settings must be a relative path".into());
    }
    let mut seen = std::collections::HashSet::new();
    for source in &state.sources {
        let path = Path::new(source);
        source_kind(path)?;
        if path.is_absolute() || !seen.insert(source) {
            return Err("manifest sources must be unique relative paths".into());
        }
    }
    Ok(())
}
fn discover() -> Result<(PathBuf, Manifest)> {
    let cwd = env::current_dir()?;
    for directory in cwd.ancestors() {
        let path = directory.join(MANIFEST);
        if path.is_file() {
            let state: Manifest = toml::from_str(&fs::read_to_string(path)?)?;
            validate(&state)?;
            return Ok((directory.to_owned(), state));
        }
    }
    Err("no Quartus.toml found; run qlm new or qlm init first".into())
}
fn save(root: &Path, state: &Manifest) -> Result<()> {
    let temporary = root.join(format!(".Quartus.toml.{}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let cleanup = Script(temporary);
    file.write_all(toml::to_string_pretty(state)?.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(&cleanup.0, root.join(MANIFEST))?;
    Ok(())
}
fn sources(action: &str, files: &[String]) -> Result<()> {
    let (root, mut state) = discover()?;
    let auto = action == "add" && files.len() == 1 && (files[0] == "--auto" || files[0] == "auto");
    let discovered;
    let inputs: &[String] = if auto {
        discovered = discover_hdl_files(&root)?;
        &discovered
    } else {
        files
    };
    if inputs.is_empty() {
        return Err("no HDL files found".into());
    }
    for file in inputs {
        let input = Path::new(file);
        source_kind(input)?;
        if action == "add" && !input.is_file() {
            return Err(format!("source file does not exist: {file}").into());
        }
        let full = absolute(input)?;
        let stored = path_text(&relative(&full, &root))?.to_owned();
        if action == "add" {
            if !state.sources.contains(&stored) {
                state.sources.push(stored);
            }
        } else {
            let old_len = state.sources.len();
            state.sources.retain(|source| source != &stored);
            if old_len == state.sources.len() {
                return Err(format!("source is not registered: {file}").into());
            }
        }
    }
    save(&root, &state)?;
    if auto {
        println!("Registered HDL files found under {}.", root.display());
    }
    println!("Updated Quartus.toml; run qlm sync to apply.");
    Ok(())
}

fn discover_hdl_files(root: &Path) -> Result<Vec<String>> {
    fn walk(directory: &Path, root: &Path, files: &mut Vec<String>) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                let name = entry.file_name();
                if matches!(
                    name.to_str(),
                    Some(".git" | "target" | "output_files" | ".qlm")
                ) {
                    continue;
                }
                walk(&path, root, files)?;
            } else if file_type.is_file() && source_kind(&path).is_ok() {
                files.push(path_text(&relative(&path, root))?.to_owned());
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn reconcile_sources(root: &Path, state: &mut Manifest) -> Result<bool> {
    let discovered = discover_hdl_files(root)?;
    let mut sources: Vec<String> = state
        .sources
        .iter()
        .filter(|source| root.join(source).is_file())
        .cloned()
        .collect();
    for source in discovered {
        if !sources.contains(&source) {
            sources.push(source);
        }
    }
    sources.sort();
    let changed = sources != state.sources;
    if changed {
        state.sources = sources;
        save(root, state)?;
        println!("Updated registered HDL files in Quartus.toml.");
    }
    Ok(changed)
}

fn sync() -> Result<()> {
    let (root, mut state) = discover()?;
    reconcile_sources(&root, &mut state)?;
    sync_project(&root, &state)
}
fn sync_project(root: &Path, state: &Manifest) -> Result<()> {
    if state.project.family.trim().is_empty() {
        return Err(
            "select an FPGA first: qlm device set de10-lite (or qlm device set <part>)".into(),
        );
    }
    if let Some(settings) = &state.settings
        && !root.join(settings).is_file()
    {
        return Err(format!("missing settings: {settings}").into());
    }
    for source in &state.sources {
        if !root.join(source).is_file() {
            return Err(format!("missing source: {source}").into());
        }
    }
    let output = root.to_owned();
    fs::create_dir_all(&output)?;
    let project = &state.project;
    let device = project_device(project);
    let mut body = format!(
        "project_new -overwrite {}\nset_global_assignment -name FAMILY {}\nset_global_assignment -name DEVICE {}\nset_global_assignment -name TOP_LEVEL_ENTITY {}\nset_global_assignment -name PROJECT_OUTPUT_DIRECTORY output_files\n",
        quote(&project.name),
        quote(&project.family),
        quote(device.as_deref().unwrap_or("AUTO")),
        quote(&project.top)
    );
    for source in &state.sources {
        let stored = relative(&root.join(source), &output);
        body.push_str(&format!(
            "set_global_assignment -name {} {}\n",
            source_kind(Path::new(source))?,
            quote(path_text(&stored)?)
        ));
    }
    if let Some(settings) = &state.settings {
        body.push_str(&format!(
            "cd {}\nsource {}\ncd {}\n",
            quote(path_text(root)?),
            quote(settings),
            quote(path_text(&output)?)
        ));
    }
    body.push_str("export_assignments\nproject_close\n");
    quartus(&output, &body)?;
    println!(
        "Synchronized {}",
        output.join(format!("{}.qpf", project.name)).display()
    );
    Ok(())
}

fn project_device(project: &Project) -> Option<String> {
    project
        .device
        .clone()
        .filter(|device| !device.is_empty() && device != "AUTO")
        .or_else(|| {
            project
                .board
                .as_deref()
                .and_then(de10_lite_part)
                .map(str::to_owned)
        })
}

fn selected_part(root: &Path, part: &str) -> Result<(String, String)> {
    let body = format!(
        "package require ::quartus::device\nset requested {}\nset found [lsearch -exact -nocase [get_part_list] $requested]\nif {{$found < 0}} {{error \"FPGA part is unavailable in this Quartus installation: $requested\"}}\nset part [lindex [get_part_list] $found]\nputs \"QLM:$part\"\nputs \"QLM:[lindex [get_part_info -family $part] 0]\"\n",
        quote(part)
    );
    let values = quartus(root, &body)?;
    if values.len() != 2 || values.iter().any(|v| v.trim().is_empty()) {
        return Err("Quartus did not return a valid part and family".into());
    }
    Ok((values[0].clone(), values[1].clone()))
}

fn resolve_detected_part(root: &Path, candidate: &str) -> Result<(String, String)> {
    if let Ok(part) = selected_part(root, candidate) {
        return Ok(part);
    }
    let normalized = candidate.to_ascii_lowercase();
    // quartus_pgm reports the DE10-Lite as the family alias 10M50DA(.|ES),
    // without the package/speed suffix needed by Quartus project files.
    if normalized == "10m50da" {
        return selected_part(root, DE10_LITE_PART);
    }
    let matches: Vec<String> = quartus(
        root,
        "package require ::quartus::device\nforeach part [get_part_list] {puts \"QLM:$part\"}\n",
    )?
    .into_iter()
    .filter(|part| part.to_ascii_lowercase().starts_with(&normalized))
    .collect();
    if matches.len() == 1 {
        return selected_part(root, &matches[0]);
    }
    Err(format!("detected device alias {candidate} is not a unique Quartus part").into())
}

fn is_de10_lite(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().replace(['_', ' '], "").as_str(),
        "de10lite" | "de10-lite"
    )
}

fn de10_lite_part(value: &str) -> Option<&'static str> {
    is_de10_lite(value).then_some(DE10_LITE_PART)
}

fn device(args: &[String]) -> Result<()> {
    let (root, mut state) = discover()?;
    match args {
        [action] if action == "help" || action == "h" => println!(
            "DEVICE COMMANDS\n  qlm device show\n  qlm device families (f)\n  qlm device list (l) [filter]\n  qlm device select (s) [filter]\n  qlm device detect (d) [--default]\n  qlm device set (set) <part-or-board>"
        ),
        [] => println!(
            "DEVICE COMMANDS\n  qlm device show\n  qlm device families (f)\n  qlm device list (l) [filter]\n  qlm device select (s) [filter]\n  qlm device detect (d) [--default]\n  qlm device set <part-or-board>"
        ),
        [action] if action == "show" => println!(
            "{} ({})",
            state.project.device.as_deref().unwrap_or("not selected"),
            state.project.family
        ),
        [action] if action == "families" || action == "f" => {
            for family in quartus(
                &root,
                "package require ::quartus::device\nforeach family [get_family_list] {puts \"QLM:$family\"}\n",
            )? {
                println!("{family}");
            }
        }
        [action, rest @ ..] if (action == "list" || action == "l") && rest.len() <= 1 => {
            let filter = rest
                .first()
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            for part in quartus(
                &root,
                "package require ::quartus::device\nforeach part [get_part_list] {puts \"QLM:$part\"}\n",
            )? {
                if part.to_ascii_lowercase().contains(&filter) {
                    println!("{part}");
                }
            }
        }
        [action, rest @ ..] if (action == "select" || action == "s") && rest.len() <= 1 => {
            let filter = rest
                .first()
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            let parts = quartus(
                &root,
                "package require ::quartus::device\nforeach part [get_part_list] {puts \"QLM:$part\"}\n",
            )?;
            let choices: Vec<_> = parts
                .into_iter()
                .filter(|part| part.to_ascii_lowercase().contains(&filter))
                .collect();
            if choices.is_empty() {
                return Err("no matching FPGA parts found".into());
            }
            for (index, part) in choices.iter().enumerate() {
                println!("{:>3}: {part}", index + 1);
            }
            let selected = choose("FPGA number or part", &choices)?;
            let requested = de10_lite_part(&selected).unwrap_or(&selected);
            let (part, family) = selected_part(&root, requested)?;
            state.project.device = Some(part.clone());
            state.project.family = family.clone();
            if part.eq_ignore_ascii_case(DE10_LITE_PART) {
                state.project.board = Some("de10-lite".to_owned());
            }
            save(&root, &state)?;
            apply_default_board_pins(&root, &state)?;
            println!("Saved device {part} ({family}) in Quartus.toml");
        }
        [action] if action == "detect" || action == "d" => detect_board(&root, &mut state, false)?,
        [action, flag]
            if (action == "detect" || action == "d")
                && (flag == "--default" || flag == "--set-default") =>
        {
            detect_board(&root, &mut state, true)?
        }
        [action, part] if action == "set" => {
            let requested = de10_lite_part(part).unwrap_or(part.as_str());
            let (part, family) = selected_part(&root, requested)?;
            state.project.device = Some(part.clone());
            state.project.family = family.clone();
            if part.eq_ignore_ascii_case(DE10_LITE_PART) {
                state.project.board = Some("de10-lite".to_owned());
            }
            save(&root, &state)?;
            apply_default_board_pins(&root, &state)?;
            println!("Saved device {part} ({family}) in Quartus.toml");
        }
        [action] if action == "set" => {
            let settings = load_settings()?;
            let board = settings
                .board
                .as_deref()
                .ok_or("no default board configured; run qlm config set board de10-lite")?;
            let requested = de10_lite_part(board).unwrap_or(board);
            let (part, family) = selected_part(&root, requested)?;
            state.project.device = Some(part.clone());
            state.project.family = family.clone();
            if part.eq_ignore_ascii_case(DE10_LITE_PART) {
                state.project.board = Some("de10-lite".to_owned());
            }
            save(&root, &state)?;
            apply_default_board_pins(&root, &state)?;
            println!("Saved device {part} ({family}) in Quartus.toml");
        }
        [part] => {
            let requested = de10_lite_part(part).unwrap_or(part.as_str());
            let (part, family) = selected_part(&root, requested)?;
            state.project.device = Some(part.clone());
            state.project.family = family.clone();
            save(&root, &state)?;
            println!("Saved device {part} ({family}) in Quartus.toml");
        }
        _ => {
            return Err(
                "usage: qlm device [show | families | list [filter] | select [filter] | detect [--default] | set <part-or-board>]".into(),
            );
        }
    }
    Ok(())
}

fn constraints_path(root: &Path, state: &Manifest) -> Result<PathBuf> {
    let path = state.settings.as_deref().unwrap_or("constraints.tcl");
    if Path::new(path).is_absolute() {
        return Err("constraints path must be relative".into());
    }
    Ok(root.join(path))
}

fn editor_for(path: &Path) -> Result<()> {
    let editor = env::var_os("VISUAL")
        .or_else(|| env::var_os("EDITOR"))
        .unwrap_or_else(|| "vi".into());
    let status = Command::new(&editor)
        .arg(path)
        .status()
        .map_err(|error| format!("could not open editor {:?}: {error}", editor))?;
    if !status.success() {
        return Err(format!("editor failed ({status})").into());
    }
    Ok(())
}

fn pin(args: &[String]) -> Result<()> {
    let (root, mut state) = discover()?;
    let path = constraints_path(&root, &state)?;
    match args {
        [action] if action == "help" || action == "h" => println!(
            "PIN COMMANDS\n  qlm pin list (l)\n  qlm pin add (a) <signal> <pin>\n  qlm pin import (i) <csv>\n  qlm pin detect (d)\n  qlm pin auto\n  qlm pin edit (e)"
        ),
        [] => println!(
            "PIN COMMANDS\n  qlm pin list (l)\n  qlm pin add (a) <signal> <pin>\n  qlm pin import (i) <csv>\n  qlm pin auto\n  qlm pin edit (e)"
        ),
        [action] if action == "list" || action == "l" => {
            list_pin_assignments(&path)?;
        }
        [action] if action == "edit" || action == "e" => editor_for(&path)?,
        [action] if action == "auto" || action == "detect" || action == "d" => {
            if state.project.device.is_none() {
                detect_board(&root, &mut state, false)?;
            }
            apply_pin_profile(&root, &state, &path)?;
        }
        [action, signal, pin, options @ ..] if action == "add" || action == "a" => {
            let iostandard = match options {
                [] => None,
                [flag, value] if flag == "--iostandard" => Some(value.as_str()),
                _ => return Err("usage: qlm pin add <signal> <pin> [--iostandard <standard>]".into()),
            };
            update_pin_file(&path, &[(signal.as_str(), pin.as_str(), iostandard)])?;
            println!("Saved pin {signal} -> {pin} in {}", path.display());
        }
        [action, file] if action == "import" || action == "i" => {
            let text = fs::read_to_string(file)?;
            let mut assignments = Vec::new();
            for (line_number, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || line.starts_with("signal,") {
                    continue;
                }
                let fields: Vec<_> = line.split(',').map(str::trim).collect();
                if fields.len() < 2 || fields.len() > 3 {
                    return Err(format!("{}:{}: expected signal,pin[,iostandard]", file, line_number + 1).into());
                }
                assignments.push((fields[0], fields[1], fields.get(2).copied()));
            }
            update_pin_file(&path, &assignments)?;
            println!("Imported {} pin assignments into {}", assignments.len(), path.display());
        }
        _ => return Err("usage: qlm pin [list | add <signal> <pin> [--iostandard <standard>] | import <csv> | auto | edit]".into()),
    }
    Ok(())
}

fn completions(args: &[String]) -> Result<()> {
    let shell = args.first().map(String::as_str).unwrap_or("");
    if args.len() != 1 {
        return Err("usage: qlm completions <bash|zsh|fish>".into());
    }
    match shell {
        "bash" => {
            let script = r#"_qlm_complete() {
    local cur prev words
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    case "${COMP_WORDS[1]}" in
        device|d) COMPREPLY=( $(compgen -W "show families f list l select s detect d set de10-lite 10M50DAF484C7G" -- "$cur") ) ;;
        cable|c) COMPREPLY=( $(compgen -W "show list l select s set permission permissions setup p de10-lite USB-Blaster" -- "$cur") ) ;;
        config) COMPREPLY=( $(compgen -W "show s path p edit e set board cable jtag-index language default-language" -- "$cur") ) ;;
        pin) COMPREPLY=( $(compgen -W "list l add a import i detect d auto edit e" -- "$cur") ) ;;
        program|p) COMPREPLY=( $(compgen -W "build b" -- "$cur") ) ;;
        completions) COMPREPLY=( $(compgen -W "bash zsh fish" -- "$cur") ) ;;
        *) COMPREPLY=( $(compgen -W "new n init i add a remove r list l sync s device d cable c board build b program p config pin tui completions help h" -- "$cur") ) ;;
    esac
}
complete -F _qlm_complete qlm
"#;
            std::io::stdout().write_all(script.as_bytes())?;
        }
        "zsh" => {
            let script = r#"#compdef qlm
_qlm() {
  local -a commands
  commands=(
    'new:create a project' 'n:create a project' 'init:initialize current directory' 'i:initialize current directory'
    'add:add HDL files' 'a:add HDL files' 'remove:remove HDL files' 'r:remove HDL files' 'list:list HDL files' 'l:list HDL files'
    'sync:generate Quartus files' 's:generate Quartus files' 'device:select an FPGA' 'd:select an FPGA'
    'cable:select a cable' 'c:select a cable' 'board:detect a board' 'build:compile' 'b:compile' 'program:program FPGA' 'p:program'
    'config:edit user settings' 'pin:edit pin assignments' 'tui:open terminal UI' 'completions:install shell completion'
  )
  if (( CURRENT == 2 )); then
    _describe 'command' commands
    return
  fi
  case $words[2] in
    device|d) _values 'action' show families f list l select s detect d set de10-lite 10M50DAF484C7G ;;
    cable|c) _values 'action' show list l select s set permission permissions setup p de10-lite 'USB-Blaster [USB-0]' ;;
    config) _values 'setting' show s path p edit e set board cable jtag-index language default-language ;;
    pin) _values 'action' list l add a import i detect d auto edit e ;;
    program|p) _values 'action' build b ;;
    completions) _values 'shell' bash zsh fish ;;
  esac
}
if ! (( $+functions[compdef] )); then
  autoload -Uz compinit && compinit
fi
compdef _qlm qlm
"#;
            std::io::stdout().write_all(script.as_bytes())?;
        }
        "fish" => {
            let script = r#"complete -c qlm -f -n '__fish_use_subcommand' -a 'new n init i add a remove r list l sync s device d cable c board build b program p config pin tui completions help h'
complete -c qlm -f -n '__fish_seen_subcommand_from device d' -a 'show families f list l select s detect d set de10-lite 10M50DAF484C7G'
complete -c qlm -f -n '__fish_seen_subcommand_from cable c' -a 'show list l select s set permission permissions setup p de10-lite "USB-Blaster [USB-0]"'
complete -c qlm -f -n '__fish_seen_subcommand_from config' -a 'show s path p edit e set board cable jtag-index language default-language'
complete -c qlm -f -n '__fish_seen_subcommand_from pin' -a 'list l add a import i detect d auto edit e'
complete -c qlm -f -n '__fish_seen_subcommand_from program p' -a 'build b'
complete -c qlm -f -n '__fish_seen_subcommand_from completions' -a 'bash zsh fish'
"#;
            std::io::stdout().write_all(script.as_bytes())?;
        }
        _ => return Err("usage: qlm completions <bash|zsh|fish>".into()),
    }
    Ok(())
}

fn list_pin_assignments(path: &Path) -> Result<()> {
    if !path.is_file() {
        return Err(format!("missing constraints file: {}", path.display()).into());
    }
    let mut assignments: BTreeMap<String, (Option<String>, Option<String>)> = BTreeMap::new();
    for line in fs::read_to_string(path)?.lines() {
        let words: Vec<_> = line.split_whitespace().collect();
        if words.first() == Some(&"set_location_assignment") {
            if let (Some(pin), Some(target)) = (words.get(1), words.get(3)) {
                let target = target.trim_matches(['{', '}']);
                assignments.entry(target.to_owned()).or_default().0 = Some(pin.to_string());
            }
        } else if words.first() == Some(&"set_instance_assignment")
            && words.get(2) == Some(&"IO_STANDARD")
            && let (Some(standard), Some(target)) = (words.get(3), words.get(5))
        {
            let standard = standard.trim_matches('"');
            let target = target.trim_matches(['{', '}']);
            assignments.entry(target.to_owned()).or_default().1 = Some(standard.to_owned());
        }
    }
    if assignments.is_empty() {
        println!("No pin assignments in {}", path.display());
    } else {
        for (signal, (pin, standard)) in assignments {
            println!(
                "{signal}: {}{}",
                pin.unwrap_or_else(|| "(no location)".to_owned()),
                standard
                    .map(|value| format!("  I/O standard: {value}"))
                    .unwrap_or_default()
            );
        }
    }
    Ok(())
}

fn update_pin_file(path: &Path, assignments: &[(&str, &str, Option<&str>)]) -> Result<()> {
    let old = if path.is_file() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };
    let mut lines: Vec<_> = old
        .lines()
        .filter(|line| {
            !assignments.iter().any(|(signal, _, _)| {
                line.trim_start().starts_with("set_location_assignment")
                    && line.trim_end().ends_with(&format!("-to {signal}"))
            })
        })
        .map(str::to_owned)
        .collect();
    if !lines.is_empty() && lines.last().is_some_and(|line| !line.is_empty()) {
        lines.push(String::new());
    }
    lines.push("# Pin assignments managed by qlm".to_owned());
    for (signal, pin, iostandard) in assignments {
        let pin = if pin.to_ascii_uppercase().starts_with("PIN_") {
            pin.to_ascii_uppercase()
        } else {
            format!("PIN_{}", pin.to_ascii_uppercase())
        };
        lines.push(format!("set_location_assignment {pin} -to {signal}"));
        if let Some(standard) = iostandard {
            lines.push(format!(
                "set_instance_assignment -name IO_STANDARD \"{standard}\" -to {signal}"
            ));
        }
    }
    fs::write(path, format!("{}\n", lines.join("\n")))?;
    Ok(())
}

fn apply_pin_profile(root: &Path, state: &Manifest, path: &Path) -> Result<()> {
    let device = state.project.device.as_deref().unwrap_or("");
    if !device.eq_ignore_ascii_case(DE10_LITE_PART)
        && !state.project.board.as_deref().is_some_and(is_de10_lite)
    {
        return Err("automatic pins are currently available for DE10-Lite (10M50DAF484C7G); use qlm pin add/import for other boards".into());
    }
    update_pin_file(
        path,
        &[
            ("clk", "P11", Some("3.3-V LVTTL")),
            ("led", "A8", Some("3.3-V LVTTL")),
        ],
    )?;
    println!("Applied DE10-Lite starter pins to {}", path.display());
    let _ = root;
    Ok(())
}

fn apply_default_board_pins(root: &Path, state: &Manifest) -> Result<()> {
    if state
        .project
        .device
        .as_deref()
        .is_some_and(|device| device.eq_ignore_ascii_case(DE10_LITE_PART))
        || state.project.board.as_deref().is_some_and(is_de10_lite)
    {
        apply_pin_profile(root, state, &constraints_path(root, state)?)?;
    }
    Ok(())
}

#[cfg(not(feature = "tui"))]
fn tui(_args: &[String]) -> Result<()> {
    Err("the TUI is optional; rebuild with `cargo build --features tui`".into())
}

#[cfg(feature = "tui")]
fn tui(args: &[String]) -> Result<()> {
    if !args.is_empty() {
        return Err("usage: qlm tui".into());
    }
    tui_app::run()
}

fn detect_board(root: &Path, state: &mut Manifest, set_default: bool) -> Result<()> {
    let settings = load_settings()?;
    let configured_cable = state
        .programmer
        .as_ref()
        .map(|programmer| programmer.cable.clone());
    let (cable, auto_selected) = if let Some(cable) = configured_cable {
        (cable, false)
    } else {
        let listing = Command::new(executable("quartus_pgm"))
            .current_dir(root)
            .arg("-l")
            .output()
            .map_err(|error| format!("could not run quartus_pgm: {error}"))?;
        let choices = if listing.status.success() {
            cable_choices(&String::from_utf8_lossy(&listing.stdout))
        } else {
            Vec::new()
        };
        let cable = match choices.as_slice() {
            [] => settings
                .cable
                .clone()
                .or_else(|| {
                    settings
                        .board
                        .as_ref()
                        .filter(|board| is_de10_lite(board))
                        .map(|_| DE10_LITE_CABLE.to_owned())
                })
                .ok_or("no programming cables detected; connect a cable and retry")?,
            [cable] => {
                println!("Detected one cable: {cable}");
                cable.clone()
            }
            _ => {
                for (index, cable) in choices.iter().enumerate() {
                    println!("{:>3}: {cable}", index + 1);
                }
                choose("Cable number or name", &choices)?
            }
        };
        (cable, true)
    };
    let output = Command::new(executable("quartus_pgm"))
        .current_dir(root)
        // `-a` asks quartus_pgm to enumerate devices on the selected cable.
        // The lower-case `i` operation is not a valid Programmer operation
        // (and newer Quartus versions reject it with error 213008).
        .args(["-c", &cable, "-a"])
        .output()
        .map_err(|error| format!("could not run quartus_pgm: {error}"))?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let lower = text.to_ascii_lowercase();
    if lower.contains("insufficient port permissions")
        || lower.contains("permission denied")
        || (lower.contains("unable to lock chain") && lower.contains("permission"))
    {
        return Err(format!(
            "Quartus can see the cable but cannot access the JTAG port. Install or enable the Quartus USB-Blaster udev rule, reload udev, reconnect the cable, and ensure your user is in the rule's group.\n\nQuartus output:\n{}",
            text.trim()
        )
        .into());
    }
    if !output.status.success() {
        return Err(format!("quartus_pgm failed ({})\n{}", output.status, text.trim()).into());
    }
    let mut candidates = Vec::new();
    for token in text.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        if token.len() < 6
            || !token.chars().any(|ch| ch.is_ascii_alphabetic())
            || !token.chars().any(|ch| ch.is_ascii_digit())
            || candidates
                .iter()
                .any(|candidate: &String| candidate.eq_ignore_ascii_case(token))
        {
            continue;
        }
        candidates.push(token.to_owned());
    }
    let (part, family) = candidates
        .iter()
        .find_map(|candidate| resolve_detected_part(root, candidate).ok())
        .ok_or_else(|| {
            format!(
                "could not identify a Quartus-supported FPGA from JTAG output:\n{}",
                text.trim()
            )
        })?;
    state.project.device = Some(part.clone());
    state.project.family = family.clone();
    if auto_selected {
        state.programmer = Some(Programmer {
            cable: cable.clone(),
            index: settings.jtag_index,
        });
    }
    if part.eq_ignore_ascii_case(DE10_LITE_PART) {
        state.project.board = Some("de10-lite".to_owned());
        if set_default {
            let mut global = settings;
            global.board = Some("de10-lite".to_owned());
            save_settings(&global)?;
            println!("Saved de10-lite as the default board.");
        }
    }
    save(root, state)?;
    apply_default_board_pins(root, state)?;
    if part.eq_ignore_ascii_case(DE10_LITE_PART) {
        println!("Detected DE10-Lite: {part} ({family})");
    } else {
        println!("Detected FPGA: {part} ({family})");
    }
    Ok(())
}

fn executable(name: &str) -> std::ffi::OsString {
    let variable = name.to_ascii_uppercase();
    if let Some(path) = env::var_os(variable) {
        return path;
    }
    // Keep programming and compilation in the same Quartus installation.
    if name == "quartus_pgm"
        && let Some(shell) = env::var_os("QUARTUS_SH")
    {
        let shell = PathBuf::from(shell);
        if shell.is_absolute() {
            return shell
                .with_file_name(if cfg!(windows) {
                    "quartus_pgm.exe"
                } else {
                    "quartus_pgm"
                })
                .into_os_string();
        }
    }
    name.into()
}

fn run_tool(directory: &Path, name: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(executable(name))
        .current_dir(directory)
        .args(args)
        .status()
        .map_err(|error| {
            format!(
                "could not run {name}: {error}; check PATH or {}",
                name.to_ascii_uppercase()
            )
        })?;
    if !status.success() {
        return Err(format!("{name} failed ({status})").into());
    }
    Ok(())
}

fn read_line(prompt: &str) -> Result<String> {
    print!("{prompt}: ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let input = input.trim();
    if input.is_empty() {
        return Err("a selection is required".into());
    }
    Ok(input.to_owned())
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt} [Y/n]: ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    ))
}

fn offer_hardware_setup(root: &Path, state: &mut Manifest) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        return Ok(());
    }
    let settings = load_settings()?;
    if state.programmer.is_none() && settings.cable.is_none() {
        let listing = match Command::new(executable("quartus_pgm"))
            .current_dir(root)
            .arg("-l")
            .output()
        {
            Ok(output) if output.status.success() => output,
            _ => return Ok(()),
        };
        let choices = cable_choices(&String::from_utf8_lossy(&listing.stdout));
        if !choices.is_empty() {
            let cable = if choices.len() == 1 {
                choices[0].clone()
            } else {
                println!("Detected programming cables:");
                for (index, cable) in choices.iter().enumerate() {
                    println!("{:>3}: {cable}", index + 1);
                }
                choose("Cable number or name", &choices)?
            };
            if confirm(&format!("Use cable {cable}?"))? {
                state.programmer = Some(Programmer {
                    cable,
                    index: settings.jtag_index,
                });
                save(root, state)?;
            }
        }
    }
    if state.project.device.is_none()
        && state.programmer.is_some()
        && confirm("Probe the connected board and use its detected device?")?
        && let Err(error) = detect_board(root, state, false)
    {
        eprintln!("Hardware detection skipped: {error}");
    }
    Ok(())
}

fn choose(prompt: &str, choices: &[String]) -> Result<String> {
    let input = read_line(prompt)?;
    if let Ok(number) = input.parse::<usize>() {
        return choices
            .get(number.checked_sub(1).ok_or("selection starts at 1")?)
            .cloned()
            .ok_or_else(|| "selection is out of range".into());
    }
    Ok(input)
}

fn cable_choices(output: &str) -> Vec<String> {
    let mut choices = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        let Some(delimiter) = line
            .char_indices()
            .find(|(index, ch)| *index > 0 && (*ch == ')' || *ch == '.' || *ch == ':'))
            .map(|(index, _)| index)
        else {
            continue;
        };
        if !line[..delimiter].chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let name = line[delimiter + 1..].trim();
        if !name.is_empty() {
            choices.push(name.to_owned());
        }
    }
    choices
}

fn install_usb_blaster_rule() -> Result<()> {
    const RULE_PATH: &str = "/etc/udev/rules.d/92-usbblaster.rules";
    const RULE: &str = r#"# Intel/Altera USB-Blaster and FPGA Download Cables
SUBSYSTEMS=="usb", ATTRS{idVendor}=="09fb", ATTRS{idProduct}=="6001", MODE="0666"
SUBSYSTEMS=="usb", ATTRS{idVendor}=="09fb", ATTRS{idProduct}=="6002", MODE="0666"
SUBSYSTEMS=="usb", ATTRS{idVendor}=="09fb", ATTRS{idProduct}=="6003", MODE="0666"
SUBSYSTEMS=="usb", ATTRS{idVendor}=="09fb", ATTRS{idProduct}=="6010", MODE="0666"
SUBSYSTEMS=="usb", ATTRS{idVendor}=="09fb", ATTRS{idProduct}=="6810", MODE="0666"
SUBSYSTEMS=="usb", ATTRS{idVendor}=="09fb", ATTRS{idProduct}=="6020", MODE="0666"
SUBSYSTEMS=="usb", ATTRS{idVendor}=="09fb", ATTRS{idProduct}=="6022", MODE="0666"
SUBSYSTEMS=="usb", ATTRS{idVendor}=="09fb", ATTRS{idProduct}=="6024", MODE="0666"
SUBSYSTEMS=="usb", ATTRS{idVendor}=="09fb", ATTRS{idProduct}=="6025", MODE="0666"
SUBSYSTEMS=="usb", ATTRS{idVendor}=="09fb", ATTRS{idProduct}=="6026", MODE="0666"
SUBSYSTEMS=="usb", ATTRS{idVendor}=="09fb", ATTRS{idProduct}=="602C", MODE="0666"
SUBSYSTEMS=="usb", ATTRS{idVendor}=="09fb", ATTRS{idProduct}=="602D", MODE="0666"
SUBSYSTEMS=="usb", ATTRS{idVendor}=="09fb", ATTRS{idProduct}=="602E", MODE="0666"
"#;

    let auth = Command::new("sudo")
        .arg("-v")
        .status()
        .map_err(|error| format!("could not run sudo: {error}"))?;
    if !auth.success() {
        return Err("sudo authorization failed; USB-Blaster rule was not changed".into());
    }
    let mut tee = Command::new("sudo")
        .args(["tee", RULE_PATH])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("could not install {RULE_PATH}: {error}"))?;
    tee.stdin
        .take()
        .ok_or("could not open sudo input")?
        .write_all(RULE.as_bytes())?;
    let installed = tee.wait()?;
    if !installed.success() {
        return Err(format!("sudo could not install {RULE_PATH}").into());
    }
    for args in [
        &["udevadm", "control", "--reload-rules"][..],
        &["udevadm", "trigger"][..],
    ] {
        let status = Command::new("sudo")
            .args(args)
            .status()
            .map_err(|error| format!("could not run sudo {}: {error}", args.join(" ")))?;
        if !status.success() {
            return Err(format!("sudo {} failed", args.join(" ")).into());
        }
    }
    println!(
        "Installed {RULE_PATH} and reloaded udev. Unplug and reconnect the USB-Blaster before retrying qlm device detect."
    );
    Ok(())
}

fn cable(args: &[String]) -> Result<()> {
    if matches!(args, [action] if action == "permission" || action == "permissions" || action == "setup" || action == "p")
    {
        return install_usb_blaster_rule();
    }
    if matches!(args, [action] if action == "help" || action == "h") {
        println!(
            "CABLE COMMANDS\n  qlm cable show\n  qlm cable list (l)\n  qlm cable select (s)\n  qlm cable set <name-or-number>\n  qlm cable permissions (p)"
        );
        return Ok(());
    }
    let (root, mut state) = discover()?;
    match args {
        [action] if action == "help" || action == "h" => println!(
            "CABLE COMMANDS\n  qlm cable show\n  qlm cable list (l)\n  qlm cable select (s)\n  qlm cable set <name-or-number>\n  qlm cable permissions (p)"
        ),
        [] => println!(
            "CABLE COMMANDS\n  qlm cable show\n  qlm cable list (l)\n  qlm cable select (s)\n  qlm cable set <name-or-number>\n  qlm cable permissions (p)"
        ),
        [action] if action == "show" => match state.programmer {
            Some(settings) => println!("{} (JTAG position {})", settings.cable, settings.index),
            None => println!("No cable selected."),
        },
        [action] if action == "list" || action == "l" => run_tool(&root, "quartus_pgm", &["-l"])?,
        [action] if action == "select" || action == "s" => {
            let output = Command::new(executable("quartus_pgm"))
                .current_dir(&root)
                .arg("-l")
                .output()
                .map_err(|error| format!("could not run quartus_pgm: {error}"))?;
            if !output.status.success() {
                return Err(format!("quartus_pgm failed ({})", output.status).into());
            }
            let listing = String::from_utf8_lossy(&output.stdout);
            let choices = cable_choices(&listing);
            let name = if choices.is_empty() {
                print!("{listing}");
                read_line("Cable name")?
            } else if choices.len() == 1 {
                println!("Detected one cable: {}", choices[0]);
                choices[0].clone()
            } else {
                for (index, cable) in choices.iter().enumerate() {
                    println!("{:>3}: {cable}", index + 1);
                }
                choose("Cable number or name", &choices)?
            };
            let index = load_settings()?.jtag_index;
            state.programmer = Some(Programmer {
                cable: name.clone(),
                index,
            });
            save(&root, &state)?;
            println!("Saved cable {name}, JTAG position {index}");
        }
        [action, name, options @ ..] if action == "set" => {
            if name.trim().is_empty() {
                return Err("cable cannot be empty".into());
            }
            let index = match options {
                [] => 1,
                [flag, value] if flag == "--index" => value.parse::<u32>()?,
                _ => {
                    return Err("usage: qlm cable set <name-or-number> [--index <position>]".into());
                }
            };
            if index == 0 {
                return Err("JTAG position starts at 1".into());
            }
            let cable = if is_de10_lite(name) {
                DE10_LITE_CABLE.to_owned()
            } else {
                name.clone()
            };
            state.programmer = Some(Programmer {
                cable: cable.clone(),
                index,
            });
            save(&root, &state)?;
            println!("Saved cable {cable}, JTAG position {index}");
        }
        [action] if action == "set" => {
            let settings = load_settings()?;
            let cable = settings
                .cable
                .or_else(|| {
                    settings
                        .board
                        .filter(|board| is_de10_lite(board))
                        .map(|_| DE10_LITE_CABLE.to_owned())
                })
                .ok_or("no default cable configured; run qlm config set board de10-lite")?;
            let index = settings.jtag_index;
            if index == 0 {
                return Err("JTAG position starts at 1".into());
            }
            state.programmer = Some(Programmer {
                cable: cable.clone(),
                index,
            });
            save(&root, &state)?;
            println!("Saved cable {cable}, JTAG position {index}");
        }
        [name] => {
            let cable = if is_de10_lite(name) {
                DE10_LITE_CABLE.to_owned()
            } else {
                name.clone()
            };
            let index = load_settings()?.jtag_index;
            state.programmer = Some(Programmer {
                cable: cable.clone(),
                index,
            });
            save(&root, &state)?;
            println!("Saved cable {cable}, JTAG position {index}");
        }
        _ => {
            return Err(
                "usage: qlm cable [list | select | set <name-or-number-or-board> [--index <position>] | permissions]"
                    .into(),
            );
        }
    }
    Ok(())
}

fn build() -> Result<PathBuf> {
    let (root, mut state) = discover()?;
    reconcile_sources(&root, &mut state)?;
    build_project(&root, &state)
}
fn build_project(root: &Path, state: &Manifest) -> Result<PathBuf> {
    let part = project_device(&state.project)
        .ok_or("select a specific FPGA before building: qlm device set <part>")?;
    let (_, family) = selected_part(root, &part)?;
    if family != state.project.family {
        return Err("saved family does not match device; run qlm device set <part> again".into());
    }
    sync_project(root, state)?;
    let output = root.to_owned();
    let sof = output
        .join("output_files")
        .join(format!("{}.sof", state.project.name));
    // An unsuccessful build must never leave an old bitstream looking current.
    if sof.exists() {
        fs::remove_file(&sof)?;
    }
    println!("Compiling {} for {part}...", state.project.name);
    run_tool(
        &output,
        "quartus_sh",
        &["--flow", "compile", &state.project.name],
    )?;
    println!("Build completed: {}", output.join("output_files").display());
    Ok(sof)
}
fn program(args: &[String]) -> Result<()> {
    let rebuild = match args {
        [] => false,
        [action] if action == "build" || action == "b" => true,
        _ => return Err("usage: qlm program [build]".into()),
    };
    let (root, mut state) = discover()?;
    if rebuild {
        reconcile_sources(&root, &mut state)?;
    }
    let programmer = state.programmer.as_ref().ok_or(
        "select a cable first: qlm cable set de10-lite (or qlm cable set <name-or-number>)",
    )?;
    let sof = if rebuild {
        build_project(&root, &state)?
    } else {
        root.join("output_files")
            .join(format!("{}.sof", state.project.name))
    };
    if !sof.is_file() {
        return Err("missing .sof bitstream; run qlm build or qlm program build first".into());
    }
    // Run from the artifact directory so delimiters in parent paths cannot be
    // interpreted as part of quartus_pgm's operation mini-language.
    let filename = sof
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("invalid bitstream path")?;
    let operation = format!("p;{filename}@{}", programmer.index);
    println!(
        "Programming {} on {} (JTAG position {})...",
        state.project.name, programmer.cable, programmer.index
    );
    run_tool(
        sof.parent().ok_or("missing artifact directory")?,
        "quartus_pgm",
        &["-c", &programmer.cable, "-m", "jtag", "-o", &operation],
    )
}

#[cfg(feature = "tui")]
mod tui_app {
    use super::*;
    use crossterm::{
        event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    };
    use ratatui::{
        Terminal,
        backend::CrosstermBackend,
        layout::{Constraint, Direction, Layout},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs},
    };
    use std::{io::stdout, time::Duration};

    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Board,
        Cable,
        Settings,
    }

    impl Mode {
        fn title(self) -> &'static str {
            match self {
                Self::Board => "Boards / Devices",
                Self::Cable => "Cables",
                Self::Settings => "Settings",
            }
        }
        fn index(self) -> usize {
            match self {
                Self::Board => 0,
                Self::Cable => 1,
                Self::Settings => 2,
            }
        }
        fn from_index(index: usize) -> Self {
            match index {
                0 => Self::Board,
                1 => Self::Cable,
                _ => Self::Settings,
            }
        }
    }

    struct App {
        root: PathBuf,
        state: Option<Manifest>,
        settings: UserSettings,
        mode: Mode,
        catalog: Vec<String>,
        families: Vec<String>,
        family_selected: usize,
        items: Vec<String>,
        query: String,
        selected: usize,
        searching: bool,
        sort_by_family: bool,
        message: String,
    }

    impl App {
        fn new() -> Result<Self> {
            let (root, state) = match discover() {
                Ok((root, state)) => (root, Some(state)),
                Err(_) => (env::current_dir()?, None),
            };
            let settings = load_settings()?;
            let mut app = Self {
                root,
                state,
                settings,
                mode: Mode::Board,
                catalog: Vec::new(),
                families: Vec::new(),
                family_selected: 0,
                items: Vec::new(),
                query: String::new(),
                selected: 0,
                searching: false,
                sort_by_family: true,
                message: "Type / to search, Enter to select, Tab to switch sections".to_owned(),
            };
            app.refresh()?;
            if let Some(saved) = app
                .state
                .as_ref()
                .and_then(|state| state.project.device.as_ref())
                .or(app.settings.board.as_ref())
                && let Some(index) = app.items.iter().position(|item| item.ends_with(saved))
            {
                app.selected = index;
            }
            Ok(app)
        }

        fn reload_catalog(&mut self) -> Result<()> {
            let all = match self.mode {
                Mode::Board => {
                    let mut devices = quartus(
                        &self.root,
                        "package require ::quartus::device\nforeach part [get_part_list] {puts \"QLM:[lindex [get_part_info -family $part] 0]\\t$part\"}\n",
                    )?;
                    devices.sort_by_key(|device| {
                        let (family, part) = device
                            .split_once('\t')
                            .map_or(("", device.as_str()), |(family, part)| (family, part));
                        if self.sort_by_family {
                            (family.to_ascii_lowercase(), part.to_ascii_lowercase())
                        } else {
                            (part.to_ascii_lowercase(), family.to_ascii_lowercase())
                        }
                    });
                    devices
                        .into_iter()
                        .map(|device| {
                            device
                                .split_once('\t')
                                .map_or(device.clone(), |(family, part)| {
                                    format!("{family}  |  {part}")
                                })
                        })
                        .collect()
                }
                Mode::Cable => {
                    let output = Command::new(executable("quartus_pgm"))
                        .current_dir(&self.root)
                        .arg("-l")
                        .output()?;
                    if !output.status.success() {
                        return Err(format!("quartus_pgm failed ({})", output.status).into());
                    }
                    cable_choices(&String::from_utf8_lossy(&output.stdout))
                }
                Mode::Settings => vec![
                    format!(
                        "board = {}",
                        self.settings.board.as_deref().unwrap_or("(none)")
                    ),
                    format!(
                        "cable = {}",
                        self.settings.cable.as_deref().unwrap_or("(none)")
                    ),
                    format!("jtag-index = {}", self.settings.jtag_index),
                    format!("language = {}", self.settings.language),
                ],
            };
            self.catalog = all;
            self.families = if self.mode == Mode::Board {
                let mut families: Vec<_> = self
                    .catalog
                    .iter()
                    .filter_map(|item| {
                        item.split_once("  |  ")
                            .map(|(family, _)| family.to_owned())
                    })
                    .collect();
                families.sort();
                families.dedup();
                families
            } else {
                Vec::new()
            };
            self.family_selected = self.family_selected.min(self.families.len());
            self.apply_filter();
            Ok(())
        }

        fn apply_filter(&mut self) {
            let query = self.query.to_ascii_lowercase();
            self.items = self
                .catalog
                .iter()
                .filter(|item| {
                    let family_matches = self.mode != Mode::Board
                        || self.family_selected == 0
                        || item.split_once("  |  ").is_some_and(|(family, _)| {
                            self.families
                                .get(self.family_selected - 1)
                                .is_some_and(|selected| selected == family)
                        });
                    family_matches && item.to_ascii_lowercase().contains(&query)
                })
                .cloned()
                .collect();
            self.selected = self.selected.min(self.items.len().saturating_sub(1));
        }

        fn refresh(&mut self) -> Result<()> {
            self.reload_catalog()
        }

        fn select(&mut self) -> Result<()> {
            let Some(value) = self.items.get(self.selected).cloned() else {
                return Ok(());
            };
            match self.mode {
                Mode::Board => {
                    let requested_part = value
                        .split_once("  |  ")
                        .map_or(value.as_str(), |(_, part)| part);
                    let requested = de10_lite_part(requested_part).unwrap_or(requested_part);
                    let (part, family) = selected_part(&self.root, requested)?;
                    self.settings.board = Some(part.clone());
                    if let Some(state) = &mut self.state {
                        state.project.device = Some(part.clone());
                        state.project.family = family.clone();
                        save(&self.root, state)?;
                    }
                    save_settings(&self.settings)?;
                    self.message = format!("Saved device {part} ({family})");
                }
                Mode::Cable => {
                    if let Some(state) = &mut self.state {
                        state.programmer = Some(Programmer {
                            cable: value.clone(),
                            index: self.settings.jtag_index,
                        });
                        save(&self.root, state)?;
                    }
                    self.settings.cable = Some(value.clone());
                    save_settings(&self.settings)?;
                    self.message = format!("Saved cable {value}");
                }
                Mode::Settings => {
                    let key = value.split('=').next().map(str::trim).unwrap_or_default();
                    match key {
                        "board" => {
                            self.settings.board = Some("de10-lite".to_owned());
                            if self.settings.cable.is_none() {
                                self.settings.cable = Some(DE10_LITE_CABLE.to_owned());
                            }
                        }
                        "cable" => self.settings.cable = Some(DE10_LITE_CABLE.to_owned()),
                        "jtag-index" => {
                            self.settings.jtag_index = (self.settings.jtag_index % 8) + 1
                        }
                        "language" => {
                            self.settings.language = match self.settings.language.as_str() {
                                "systemverilog" => "verilog",
                                "verilog" => "vhdl",
                                _ => "systemverilog",
                            }
                            .to_owned();
                        }
                        _ => {}
                    }
                    save_settings(&self.settings)?;
                    self.message = "Saved user settings".to_owned();
                }
            }
            self.refresh()
        }
    }

    pub fn run() -> Result<()> {
        let mut app = App::new()?;
        enable_raw_mode()?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen)?;
        let result = run_loop(&mut app);
        disable_raw_mode()?;
        execute!(out, LeaveAlternateScreen)?;
        result
    }

    fn run_loop(app: &mut App) -> Result<()> {
        let backend = CrosstermBackend::new(stdout());
        let mut terminal = Terminal::new(backend)?;
        loop {
            terminal.draw(|frame| {
                let layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Length(3),
                        Constraint::Min(1),
                        Constraint::Length(2),
                    ])
                    .split(frame.area());
                let tabs = Tabs::new(["Boards / Devices", "Cables", "Settings"])
                    .select(app.mode.index())
                    .block(Block::default().borders(Borders::ALL).title("qlm"))
                    .highlight_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    );
                frame.render_widget(tabs, layout[0]);
                let family_labels = std::iter::once("All".to_owned())
                    .chain(app.families.iter().cloned())
                    .collect::<Vec<_>>();
                let family_tabs = Tabs::new(family_labels)
                    .select(if app.mode == Mode::Board {
                        app.family_selected
                    } else {
                        0
                    })
                    .block(Block::default().borders(Borders::ALL).title("Board family"))
                    .highlight_style(
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    );
                frame.render_widget(family_tabs, layout[1]);
                let title = if app.searching {
                    format!("{}  search: {}", app.mode.title(), app.query)
                } else {
                    app.mode.title().to_owned()
                };
                let list = List::new(app.items.iter().map(|item| ListItem::new(item.clone())))
                    .block(Block::default().borders(Borders::ALL).title(title))
                    .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
                    .highlight_symbol("▶ ");
                let mut state = ListState::default();
                state.select((!app.items.is_empty()).then_some(app.selected));
                frame.render_stateful_widget(list, layout[2], &mut state);
                let footer = Paragraph::new(Line::from(vec![
                    Span::styled("↑/↓", Style::default().fg(Color::Yellow)),
                    Span::raw(" move  "),
                    Span::styled("/", Style::default().fg(Color::Yellow)),
                    Span::raw(" search  "),
                    Span::styled("Enter", Style::default().fg(Color::Yellow)),
                    Span::raw(" select  ←/→ family  f sort  Tab section  q quit  "),
                    Span::raw(if app.sort_by_family {
                        "sorted by family  |  "
                    } else {
                        "sorted by part  |  "
                    }),
                    Span::raw(&app.message),
                ]))
                .block(Block::default().borders(Borders::ALL));
                frame.render_widget(footer, layout[3]);
            })?;
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
            else {
                continue;
            };
            match code {
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => break,
                KeyCode::Char('q') if !app.searching => break,
                KeyCode::Char('/') if !app.searching => {
                    app.searching = true;
                    app.query.clear();
                }
                KeyCode::Char(ch) if !app.searching && ch.is_ascii_alphanumeric() => {
                    app.searching = true;
                    app.query.clear();
                    app.query.push(ch);
                    app.apply_filter();
                }
                KeyCode::Esc => {
                    app.searching = false;
                    app.query.clear();
                    app.apply_filter();
                }
                KeyCode::Char(ch) if app.searching => {
                    app.query.push(ch);
                    app.apply_filter();
                }
                KeyCode::Backspace if app.searching => {
                    app.query.pop();
                    app.apply_filter();
                }
                KeyCode::Tab => {
                    app.mode = Mode::from_index((app.mode.index() + 1) % 3);
                    app.query.clear();
                    app.selected = 0;
                    app.family_selected = 0;
                    app.refresh()?;
                }
                KeyCode::Char('1') => {
                    app.mode = Mode::Board;
                    app.selected = 0;
                    app.family_selected = 0;
                    app.query.clear();
                    app.refresh()?;
                }
                KeyCode::Char('2') => {
                    app.mode = Mode::Cable;
                    app.selected = 0;
                    app.family_selected = 0;
                    app.query.clear();
                    app.refresh()?;
                }
                KeyCode::Char('3') => {
                    app.mode = Mode::Settings;
                    app.selected = 0;
                    app.family_selected = 0;
                    app.query.clear();
                    app.refresh()?;
                }
                KeyCode::Left if !app.searching && app.mode == Mode::Board => {
                    app.family_selected = app.family_selected.saturating_sub(1);
                    app.apply_filter();
                }
                KeyCode::Right if !app.searching && app.mode == Mode::Board => {
                    app.family_selected = (app.family_selected + 1).min(app.families.len());
                    app.apply_filter();
                }
                KeyCode::Up => app.selected = app.selected.saturating_sub(1),
                KeyCode::Down => {
                    if !app.items.is_empty() {
                        app.selected = (app.selected + 1).min(app.items.len() - 1);
                    }
                }
                KeyCode::Enter => {
                    app.searching = false;
                    app.select()?;
                }
                KeyCode::Char('r') if !app.searching => app.refresh()?,
                KeyCode::Char('f') if !app.searching && app.mode == Mode::Board => {
                    app.sort_by_family = !app.sort_by_family;
                    app.refresh()?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn source_kind(path: &Path) -> Result<&'static str> {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "v" => Ok("VERILOG_FILE"),
        "sv" => Ok("SYSTEMVERILOG_FILE"),
        "vhd" | "vhdl" => Ok("VHDL_FILE"),
        _ => Err(format!("unsupported HDL extension: {}", path.display()).into()),
    }
}

fn absolute(path: &Path) -> Result<PathBuf> {
    let full = env::current_dir()?.join(path);
    let mut normalized = PathBuf::new();
    for component in full.components() {
        match component {
            Component::CurDir => (),
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

fn relative(path: &Path, base: &Path) -> PathBuf {
    let a: Vec<_> = path.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    if common == 0 {
        return path.to_owned();
    }
    let mut result = PathBuf::new();
    for _ in common..b.len() {
        result.push("..");
    }
    for part in &a[common..] {
        result.push(part.as_os_str());
    }
    result
}

struct Script(PathBuf);
impl Drop for Script {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn quartus(directory: &Path, body: &str) -> Result<Vec<String>> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let script = Script(env::temp_dir().join(format!("qlm-{}-{stamp}.tcl", std::process::id())));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&script.0)?;
    write!(
        file,
        "package require ::quartus::project\nif {{[catch {{\n{body}\n}} problem]}} {{\ncatch {{project_close -dont_export_assignments}}\nputs stderr $problem\nexit 1\n}}\n"
    )?;
    drop(file);
    let executable = env::var_os("QUARTUS_SH").unwrap_or_else(|| "quartus_sh".into());
    let output = Command::new(executable).current_dir(directory).arg("-t").arg(&script.0).output()
        .map_err(|error| format!("could not run quartus_sh: {error}; set QUARTUS_SH or add Quartus's bin directory to PATH"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        return Err(format!(
            "Quartus failed ({})\n{stdout}{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let mut records = Vec::new();
    for line in stdout.lines() {
        if let Some(message) = line.strip_prefix("QLM:") {
            records.push(message.to_owned());
        } else if line.trim_start().starts_with("Warning")
            || line.trim_start().starts_with("Critical Warning")
            || line.trim_start().starts_with("Error")
        {
            eprintln!("{line}");
        }
    }
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tcl_words_escape_substitution() {
        assert_eq!(
            quote("a $b [exit];\"\\\nc"),
            "\"a \\$b \\[exit\\];\\\"\\\\\\nc\""
        );
    }
    #[test]
    fn portable_relative_paths() {
        assert_eq!(
            relative(Path::new("/work/demo/src/top.sv"), Path::new("/work/demo")),
            Path::new("src/top.sv")
        );
        assert_eq!(
            relative(Path::new("/work/shared/top.v"), Path::new("/work/demo")),
            Path::new("../shared/top.v")
        );
    }

    #[test]
    fn de10_lite_shortcuts_are_case_insensitive() {
        assert_eq!(de10_lite_part("de10-lite"), Some(DE10_LITE_PART));
        assert_eq!(de10_lite_part("DE10LITE"), Some(DE10_LITE_PART));
        assert!(is_de10_lite("de10_lite"));
        assert!(!is_de10_lite("de10"));
    }

    #[test]
    fn cable_listing_becomes_a_numbered_menu() {
        assert_eq!(
            cable_choices("Available cables:\n1) USB-Blaster [USB-0]\n2) USB-Blaster [USB-1]\n"),
            vec![
                "USB-Blaster [USB-0]".to_owned(),
                "USB-Blaster [USB-1]".to_owned()
            ]
        );
    }
}
