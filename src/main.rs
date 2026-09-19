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
  qlm list (l)                  List current HDL sources
  qlm build (b)                 Auto-sync sources and project, then compile
  qlm program (p)               Program FPGA; build automatically if needed

Build detects unset board/cable selections. Source changes sync automatically.

HARDWARE
  qlm device (d) <command>      Device commands
  qlm cable (c) <command>       Cable commands
  qlm pin <command>             Pin commands

SETTINGS AND TOOLS
  qlm config <command>          Settings commands
  qlm completions <shell>       Install or remove shell completions
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
        "l" => "list",
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
        "list" if args.len() == 1 => {
            let (root, mut state) = discover()?;
            reconcile_sources(&root, &mut state)?;
            for source in state.sources {
                println!("{source}");
            }
        }
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
        "Created {} in {}\nEdit {source} and constraints.tcl, then run qlm build.\nBoard/cable: auto-detect when unset · Sources/project: auto-sync on build",
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
    }
    Ok(changed)
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
        "Auto-synced project: {}",
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

struct CompletionPaths {
    script: PathBuf,
    startup: Vec<PathBuf>,
}

fn completion_paths(
    shell: &str,
    home: &Path,
    config: &Path,
    zsh_config: &Path,
) -> Result<CompletionPaths> {
    match shell {
        "bash" => {
            let login = [".bash_profile", ".bash_login", ".profile"]
                .iter()
                .map(|file| home.join(file))
                .find(|file| file.exists())
                .unwrap_or_else(|| home.join(".bash_profile"));
            Ok(CompletionPaths {
                script: config.join("qlm/completions/qlm.bash"),
                startup: vec![home.join(".bashrc"), login],
            })
        }
        "zsh" => Ok(CompletionPaths {
            script: config.join("qlm/completions/qlm.zsh"),
            startup: vec![zsh_config.join(".zshrc")],
        }),
        "fish" => Ok(CompletionPaths {
            script: config.join("fish/completions/qlm.fish"),
            startup: Vec::new(),
        }),
        _ => Err("shell must be bash, zsh, or fish".into()),
    }
}

fn completion_startup(old: &str, shell: &str, script: &Path) -> Result<String> {
    let start = format!("# >>> qlm {shell} completions >>>");
    let end = format!("# <<< qlm {shell} completions <<<");
    // Single-quote the path so spaces, quotes and shell substitutions remain literal.
    let path = format!("'{}'", path_text(script)?.replace('\'', "'\\''"));
    let source = format!("[ ! -r {path} ] || . {path}");
    let body = if shell == "bash" {
        // .profile can be shared by other shells and is also read non-interactively.
        format!(
            "if [ -n \"${{BASH_VERSION-}}\" ]; then\n    case $- in\n        *i*) {source} ;;\n    esac\nfi"
        )
    } else {
        source
    };
    let block = format!("{start}\n{body}\n{end}\n");
    match completion_block_range(old, shell)? {
        None => Ok(format!(
            "{old}{}{block}",
            if old.is_empty() || old.ends_with('\n') {
                ""
            } else {
                "\n"
            }
        )),
        Some(range) => Ok(format!(
            "{}{block}{}",
            &old[..range.start],
            &old[range.end..]
        )),
    }
}

fn completion_block_range(old: &str, shell: &str) -> Result<Option<std::ops::Range<usize>>> {
    let start = format!("# >>> qlm {shell} completions >>>");
    let end = format!("# <<< qlm {shell} completions <<<");
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    let mut offset = 0;
    for line in old.split_inclusive('\n') {
        let marker = line.trim_end_matches(['\r', '\n']);
        if marker == start {
            starts.push(offset);
        }
        if marker == end {
            ends.push(offset + line.len());
        }
        offset += line.len();
    }
    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => Ok(None),
        ([from], [to]) if from < to => Ok(Some(*from..*to)),
        _ => Err("incomplete or duplicate qlm completion markers in shell startup file".into()),
    }
}

fn install_completions(shell: &str, paths: &CompletionPaths) -> Result<()> {
    let script = completion_script(shell)?;
    // Validate every startup edit before writing any files.
    let updates = paths
        .startup
        .iter()
        .map(|path| {
            let old = match fs::read_to_string(path) {
                Ok(text) => text,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(error) => return Err(error.into()),
            };
            let text = completion_startup(&old, shell, &paths.script)?;
            Ok((path, text))
        })
        .collect::<Result<Vec<_>>>()?;
    fs::create_dir_all(paths.script.parent().ok_or("invalid completion path")?)?;
    fs::write(&paths.script, script)?;
    for (path, text) in updates {
        fs::create_dir_all(path.parent().ok_or("invalid shell startup path")?)?;
        fs::write(path, text)?;
    }
    Ok(())
}

fn remove_completions(shell: &str, paths: &CompletionPaths) -> Result<bool> {
    let mut startup = paths.startup.clone();
    if shell == "bash" {
        // Login profile precedence may have changed since installation.
        if let Some(home) = paths.startup.first().and_then(|path| path.parent()) {
            for name in [".bash_profile", ".bash_login", ".profile"] {
                let path = home.join(name);
                if !startup.contains(&path) {
                    startup.push(path);
                }
            }
        }
    }
    let mut updates = Vec::new();
    for path in startup {
        let old = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if let Some(range) = completion_block_range(&old, shell)? {
            updates.push((
                path,
                format!("{}{}", &old[..range.start], &old[range.end..]),
            ));
        }
    }
    // Validate all managed blocks before deleting the script or changing startup files.
    let removed = match fs::remove_file(&paths.script) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    let changed = removed || !updates.is_empty();
    for (path, text) in updates {
        fs::write(path, text)?;
    }
    Ok(changed)
}

fn completions(args: &[String]) -> Result<()> {
    let (shell, operation) = match args {
        [shell] => (shell.as_str(), "install"),
        [shell, option] if option == "--print" => (shell.as_str(), "print"),
        [shell, option] if option == "--remove" => (shell.as_str(), "remove"),
        [action, shell] if action == "remove" => (shell.as_str(), "remove"),
        _ => return Err("usage: qlm completions <bash|zsh|fish> [--print|--remove]".into()),
    };
    let script = completion_script(shell)?;
    if operation == "print" {
        std::io::stdout().write_all(script.as_bytes())?;
        return Ok(());
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set")?;
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let zsh_config = env::var_os("ZDOTDIR")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| home.clone());
    let paths = completion_paths(shell, &home, &config, &zsh_config)?;
    if operation == "remove" {
        if remove_completions(shell, &paths)? {
            println!("Removed {shell} completions and automatic loading entries.");
            println!("Open a new {shell} session to clear previously loaded completions.");
        } else {
            println!("No installed {shell} completions found.");
        }
        return Ok(());
    }
    install_completions(shell, &paths)?;
    println!("Saved {shell} completions: {}", paths.script.display());
    println!("Automatic loading enabled · Open a new {shell} session to use completions.");
    Ok(())
}

fn completion_script(shell: &str) -> Result<&'static str> {
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
        program|p) ;;
        completions)
            if [ "$COMP_CWORD" -eq 2 ]; then
                COMPREPLY=( $(compgen -W "bash zsh fish remove" -- "$cur") )
            elif [ "$COMP_CWORD" -eq 3 ]; then
                if [ "${COMP_WORDS[2]}" = remove ]; then
                    COMPREPLY=( $(compgen -W "bash zsh fish" -- "$cur") )
                else
                    COMPREPLY=( $(compgen -W "--print --remove" -- "$cur") )
                fi
            fi ;;

        *) COMPREPLY=( $(compgen -W "new n init i list l device d cable c board build b program p config pin tui completions help h" -- "$cur") ) ;;
    esac
}
complete -F _qlm_complete qlm
"#;
            Ok(script)
        }
        "zsh" => {
            let script = r#"#compdef qlm
_qlm() {
  local -a commands
  commands=(
    'new:create a project' 'n:create a project' 'init:initialize current directory' 'i:initialize current directory'
    'list:list HDL files' 'l:list HDL files'
    'device:select an FPGA' 'd:select an FPGA'
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
    program|p) ;;
    completions)
      if (( CURRENT == 3 )); then
        _values 'shell or action' bash zsh fish remove
      elif [[ $words[3] == remove ]]; then
        _values 'shell' bash zsh fish
      else
        _arguments '--print[Print the script]' '--remove[Remove completions]'
      fi ;;
  esac
}
if ! (( $+functions[compdef] )); then
  autoload -Uz compinit && compinit
fi
compdef _qlm qlm
"#;
            Ok(script)
        }
        "fish" => {
            let script = r#"complete -c qlm -f -n '__fish_use_subcommand' -a 'new n init i list l device d cable c board build b program p config pin tui completions help h'
complete -c qlm -f -n '__fish_seen_subcommand_from device d' -a 'show families f list l select s detect d set de10-lite 10M50DAF484C7G'
complete -c qlm -f -n '__fish_seen_subcommand_from cable c' -a 'show list l select s set permission permissions setup p de10-lite "USB-Blaster [USB-0]"'
complete -c qlm -f -n '__fish_seen_subcommand_from config' -a 'show s path p edit e set board cable jtag-index language default-language'
complete -c qlm -f -n '__fish_seen_subcommand_from pin' -a 'list l add a import i detect d auto edit e'
complete -c qlm -f -n '__fish_seen_subcommand_from completions; and not __fish_seen_subcommand_from bash zsh fish' -a 'bash zsh fish'
complete -c qlm -f -n '__fish_seen_subcommand_from completions; and not __fish_seen_subcommand_from bash zsh fish remove' -a 'remove'
complete -c qlm -f -n '__fish_seen_subcommand_from completions; and __fish_seen_subcommand_from bash zsh fish; and not __fish_seen_subcommand_from remove' -l print -d 'Print the script'
complete -c qlm -f -n '__fish_seen_subcommand_from completions; and __fish_seen_subcommand_from bash zsh fish; and not __fish_seen_subcommand_from remove' -l remove -d 'Remove completions'
"#;
            Ok(script)
        }
        _ => Err("shell must be bash, zsh, or fish".into()),
    }
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

fn detect_cable(root: &Path, state: &Manifest, settings: &UserSettings) -> Result<(String, bool)> {
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
    Ok((cable, auto_selected))
}

fn ensure_hardware(root: &Path, state: &mut Manifest) -> Result<()> {
    if project_device(&state.project).is_none() {
        println!("Detecting board and cable...");
        detect_board(root, state, false)?;
    } else if state.programmer.is_none() {
        println!("Detecting cable...");
        let settings = load_settings()?;
        let (cable, _) = detect_cable(root, state, &settings)?;
        state.programmer = Some(Programmer {
            cable,
            index: settings.jtag_index,
        });
        save(root, state)?;
    }
    Ok(())
}

fn detect_board(root: &Path, state: &mut Manifest, set_default: bool) -> Result<()> {
    let settings = load_settings()?;
    let (cable, auto_selected) = detect_cable(root, state, &settings)?;
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
    build_project(&root, &mut state)
}
fn build_project(root: &Path, state: &mut Manifest) -> Result<PathBuf> {
    ensure_hardware(root, state)?;
    println!("Auto-syncing sources and project...");
    reconcile_sources(root, state)?;
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
    if let Err(error) = run_tool(
        &output,
        "quartus_sh",
        &["--flow", "compile", &state.project.name],
    ) {
        if sof.exists() {
            fs::remove_file(&sof)?;
        }
        return Err(error);
    }
    if !sof.is_file() {
        return Err("build produced no .sof bitstream".into());
    }
    println!("Build completed: {}", output.join("output_files").display());
    Ok(sof)
}
fn program(args: &[String]) -> Result<()> {
    if !args.is_empty() {
        return Err("usage: qlm program".into());
    }
    let (root, mut state) = discover()?;
    let sof = root
        .join("output_files")
        .join(format!("{}.sof", state.project.name));
    if !sof.is_file() {
        println!("No build found · Building automatically...");
        build_project(&root, &mut state)?;
    } else {
        ensure_hardware(&root, &mut state)?;
        println!("Using existing build: {}", sof.display());
    }
    if !sof.is_file() {
        return Err("build produced no .sof bitstream; programming stopped".into());
    }
    let programmer = state
        .programmer
        .as_ref()
        .ok_or("no programming cable selected")?;
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
        event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    };
    use ratatui::{
        Frame, Terminal,
        backend::CrosstermBackend,
        layout::{Constraint, Direction, Layout},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap},
    };
    use std::{
        io::stdout,
        sync::mpsc::{self, Receiver, TryRecvError},
        thread,
        time::Duration,
    };

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Mode {
        Actions,
        Board,
        Cable,
        Settings,
    }

    #[derive(Debug, PartialEq)]
    enum Action {
        None,
        Quit,
        Load,
        Refresh,
        Select,
    }

    impl Mode {
        fn title(self) -> &'static str {
            match self {
                Self::Actions => "Actions",
                Self::Board => "Boards / Devices",
                Self::Cable => "Cables",
                Self::Settings => "Settings",
            }
        }
        fn index(self) -> usize {
            match self {
                Self::Actions => 0,
                Self::Board => 1,
                Self::Cable => 2,
                Self::Settings => 3,
            }
        }
        fn from_index(index: usize) -> Self {
            match index {
                0 => Self::Actions,
                1 => Self::Board,
                2 => Self::Cable,
                _ => Self::Settings,
            }
        }
    }

    impl Mode {
        fn hardware_index(self) -> Option<usize> {
            match self {
                Self::Board => Some(0),
                Self::Cable => Some(1),
                _ => None,
            }
        }
    }

    struct Field {
        label: &'static str,
        flag: Option<&'static str>,
        required: bool,
        choices: &'static [&'static str],
    }

    const DIRECTORY: Field = Field {
        label: "New directory",
        flag: None,
        required: true,
        choices: &[],
    };
    const NAME: Field = Field {
        label: "Project name (optional)",
        flag: Some("--name"),
        required: false,
        choices: &[],
    };
    const LANGUAGE: Field = Field {
        label: "Language (optional; ←/→ choose)",
        flag: Some("--lang"),
        required: false,
        choices: &["systemverilog", "verilog", "vhdl"],
    };
    const TOP: Field = Field {
        label: "Top-level module (optional)",
        flag: Some("--top"),
        required: false,
        choices: &[],
    };

    struct CliAction {
        title: &'static str,
        description: &'static str,
        args: &'static [&'static str],
        fields: &'static [Field],
        needs_project: bool,
    }

    const CLI_ACTIONS: &[CliAction] = &[
        CliAction {
            title: "Build project",
            description: "Auto-detect unset hardware, auto-sync sources and project, then compile.",
            args: &["build"],
            fields: &[],
            needs_project: true,
        },
        CliAction {
            title: "Program FPGA",
            description: "Load the existing build onto the FPGA. Build automatically if none exists.",
            args: &["program"],
            fields: &[],
            needs_project: true,
        },
        CliAction {
            title: "Create a new project…",
            description: "Create a project in a new directory and open it here.",
            args: &["new"],
            fields: &[DIRECTORY, NAME, LANGUAGE, TOP],
            needs_project: false,
        },
        CliAction {
            title: "Initialize this directory…",
            description: "Create a project here. Existing project files are protected.",
            args: &["init"],
            fields: &[NAME, LANGUAGE, TOP],
            needs_project: false,
        },
        CliAction {
            title: "List source files",
            description: "Show current HDL files. The source list updates automatically.",
            args: &["list"],
            fields: &[],
            needs_project: true,
        },
        CliAction {
            title: "Detect board and cable",
            description: "Probe the connected hardware and save the selection in this project.",
            args: &["device", "detect"],
            fields: &[],
            needs_project: true,
        },
        CliAction {
            title: "Detect and save default board",
            description: "Detect hardware and save a recognized DE10-Lite as your default board.",
            args: &["device", "detect", "--default"],
            fields: &[],
            needs_project: true,
        },
        CliAction {
            title: "Show selected device",
            description: "Show the FPGA target saved in this project.",
            args: &["device", "show"],
            fields: &[],
            needs_project: true,
        },
        CliAction {
            title: "Set device by name…",
            description: "Enter a Quartus part number or board name, such as de10-lite.",
            args: &["device", "set"],
            fields: &[Field {
                label: "Part or board name",
                flag: None,
                required: true,
                choices: &[],
            }],
            needs_project: true,
        },
        CliAction {
            title: "Show selected cable",
            description: "Show the project's cable and JTAG position.",
            args: &["cable", "show"],
            fields: &[],
            needs_project: true,
        },
        CliAction {
            title: "Set cable and JTAG position…",
            description: "Save a cable name or number and an optional JTAG position.",
            args: &["cable", "set"],
            fields: &[
                Field {
                    label: "Cable name or number",
                    flag: None,
                    required: true,
                    choices: &[],
                },
                Field {
                    label: "JTAG position (optional; default 1)",
                    flag: Some("--index"),
                    required: false,
                    choices: &[],
                },
            ],
            needs_project: true,
        },
        CliAction {
            title: "Apply automatic board pins",
            description: "Detect the board if needed and apply supported starter pin assignments.",
            args: &["pin", "auto"],
            fields: &[],
            needs_project: true,
        },
        CliAction {
            title: "List pin assignments",
            description: "Show the signal-to-pin assignments in the constraints file.",
            args: &["pin", "list"],
            fields: &[],
            needs_project: true,
        },
        CliAction {
            title: "Add or update a pin…",
            description: "Assign a physical FPGA pin to a signal in your design.",
            args: &["pin", "add"],
            fields: &[
                Field {
                    label: "Signal name",
                    flag: None,
                    required: true,
                    choices: &[],
                },
                Field {
                    label: "Pin (for example P11)",
                    flag: None,
                    required: true,
                    choices: &[],
                },
                Field {
                    label: "I/O standard (optional)",
                    flag: Some("--iostandard"),
                    required: false,
                    choices: &[],
                },
            ],
            needs_project: true,
        },
        CliAction {
            title: "Import pin assignments…",
            description: "Import a CSV with signal,pin and optional iostandard columns.",
            args: &["pin", "import"],
            fields: &[Field {
                label: "CSV path (relative to project)",
                flag: None,
                required: true,
                choices: &[],
            }],
            needs_project: true,
        },
        CliAction {
            title: "Edit pin and timing constraints",
            description: "Open constraints in your configured text editor.",
            args: &["pin", "edit"],
            fields: &[],
            needs_project: true,
        },
        CliAction {
            title: "Set up USB-Blaster permissions",
            description: "Install the Linux USB-Blaster access rule. May request your sudo password.",
            args: &["cable", "permissions"],
            fields: &[],
            needs_project: false,
        },
        CliAction {
            title: "Show user defaults",
            description: "Show your saved board, cable, JTAG position and HDL language defaults.",
            args: &["config", "show"],
            fields: &[],
            needs_project: false,
        },
        CliAction {
            title: "Set a user default…",
            description: "Set a default for future projects.",
            args: &["config", "set"],
            fields: &[
                Field {
                    label: "Setting (←/→ choose)",
                    flag: None,
                    required: true,
                    choices: &["board", "cable", "jtag-index", "language"],
                },
                Field {
                    label: "Value",
                    flag: None,
                    required: true,
                    choices: &[],
                },
            ],
            needs_project: false,
        },
        CliAction {
            title: "Edit user defaults",
            description: "Open user settings in your configured text editor.",
            args: &["config", "edit"],
            fields: &[],
            needs_project: false,
        },
        CliAction {
            title: "Show settings file location",
            description: "Print the path to your user settings file.",
            args: &["config", "path"],
            fields: &[],
            needs_project: false,
        },
        CliAction {
            title: "Install shell completions…",
            description: "Save a completion file and enable automatic loading in Bash, Zsh or Fish.",
            args: &["completions"],
            fields: &[Field {
                label: "Shell (←/→ choose)",
                flag: None,
                required: true,
                choices: &["bash", "zsh", "fish"],
            }],
            needs_project: false,
        },
        CliAction {
            title: "Remove shell completions…",
            description: "Remove the selected shell's completion file and qlm startup entries.",
            args: &["completions", "remove"],
            fields: &[Field {
                label: "Shell (←/→ choose)",
                flag: None,
                required: true,
                choices: &["bash", "zsh", "fish"],
            }],
            needs_project: false,
        },
        CliAction {
            title: "CLI help",
            description: "Show available commands and automatic build behavior.",
            args: &["help"],
            fields: &[],
            needs_project: false,
        },
    ];

    struct CommandRequest {
        action: usize,
        args: Vec<String>,
    }

    struct CommandForm {
        action: usize,
        values: Vec<String>,
        selected: usize,
        error: String,
    }

    impl CommandForm {
        fn new(action: usize) -> Self {
            Self {
                action,
                values: vec![String::new(); CLI_ACTIONS[action].fields.len()],
                selected: 0,
                error: String::new(),
            }
        }

        fn request(&self) -> std::result::Result<CommandRequest, String> {
            let action = &CLI_ACTIONS[self.action];
            let mut args: Vec<_> = action.args.iter().map(|arg| (*arg).to_owned()).collect();
            for (field, value) in action.fields.iter().zip(&self.values) {
                let value = value.trim();
                if value.is_empty() {
                    if field.required {
                        return Err(format!("{} is required", field.label));
                    }
                    continue;
                }
                if !field.choices.is_empty() && !field.choices.contains(&value) {
                    return Err(format!("Choose {}", field.choices.join(", ")));
                }
                if field.flag.is_none() && value.starts_with('-') {
                    return Err(
                        "Values cannot start with '-'; use ./ for paths beginning with a dash."
                            .into(),
                    );
                }
                if let Some(flag) = field.flag {
                    args.push(flag.to_owned());
                }
                args.push(value.to_owned());
            }
            Ok(CommandRequest {
                action: self.action,
                args,
            })
        }
    }

    type CatalogResult = std::result::Result<Vec<String>, String>;

    fn load_hardware_catalog(root: &Path, mode: Mode) -> Result<Vec<String>> {
        match mode {
            Mode::Board => {
                let devices = quartus(
                    root,
                    "package require ::quartus::device\nforeach part [get_part_list] {puts \"QLM:[lindex [get_part_info -family $part] 0]\\t$part\"}\n",
                )?;
                Ok(devices
                    .into_iter()
                    .map(|device| {
                        device.split_once('\t').map_or_else(
                            || device.clone(),
                            |(family, part)| format!("{family}  |  {part}"),
                        )
                    })
                    .collect())
            }
            Mode::Cable => {
                let output = Command::new(executable("quartus_pgm"))
                    .current_dir(root)
                    .arg("-l")
                    .output()?;
                if !output.status.success() {
                    return Err(format!("quartus_pgm failed ({})", output.status).into());
                }
                Ok(cable_choices(&String::from_utf8_lossy(&output.stdout)))
            }
            Mode::Actions | Mode::Settings => {
                unreachable!("this section does not require a hardware scan")
            }
        }
    }

    struct App {
        root: PathBuf,
        state: Option<Manifest>,
        settings: UserSettings,
        mode: Mode,
        catalog: Vec<String>,
        cached_catalogs: [Option<Vec<String>>; 2],
        pending_catalogs: [Option<Receiver<CatalogResult>>; 2],
        families: Vec<String>,
        family_selected: usize,
        items: Vec<String>,
        query: String,
        selected: usize,
        searching: bool,
        sort_by_family: bool,
        message: String,
        form: Option<CommandForm>,
        pending_command: Option<CommandRequest>,
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
                mode: Mode::Actions,
                catalog: Vec::new(),
                cached_catalogs: [None, None],
                pending_catalogs: [None, None],
                families: Vec::new(),
                family_selected: 0,
                items: Vec::new(),
                query: String::new(),
                selected: 0,
                searching: false,
                sort_by_family: true,
                message: "Build: auto-detect unset hardware · Auto-sync sources/project".to_owned(),
                form: None,
                pending_command: None,
            };
            app.load(false)?;
            Ok(app)
        }

        // Only initial loads and explicit refreshes start external processes.
        fn load(&mut self, force: bool) -> Result<()> {
            self.show_catalog();
            let Some(index) = self.mode.hardware_index() else {
                return Ok(());
            };
            if self.pending_catalogs[index].is_none()
                && (force || self.cached_catalogs[index].is_none())
            {
                let (sender, receiver) = mpsc::channel();
                let root = self.root.clone();
                let mode = self.mode;
                thread::Builder::new()
                    .name("qlm-catalog".into())
                    .spawn(move || {
                        let result =
                            load_hardware_catalog(&root, mode).map_err(|error| error.to_string());
                        let _ = sender.send(result);
                    })?;
                self.pending_catalogs[index] = Some(receiver);
            }
            Ok(())
        }

        fn loading(&self) -> bool {
            self.mode
                .hardware_index()
                .is_some_and(|index| self.pending_catalogs[index].is_some())
        }

        fn poll_catalogs(&mut self) {
            for index in 0..self.pending_catalogs.len() {
                let Some(receiver) = &self.pending_catalogs[index] else {
                    continue;
                };
                let result = match receiver.try_recv() {
                    Ok(result) => result,
                    Err(TryRecvError::Empty) => continue,
                    Err(TryRecvError::Disconnected) => {
                        Err("hardware scan stopped unexpectedly".into())
                    }
                };
                self.pending_catalogs[index] = None;
                match result {
                    Ok(catalog) => {
                        self.cached_catalogs[index] = Some(catalog);
                        if self.mode.hardware_index() == Some(index) {
                            self.show_catalog();
                            if self.message.starts_with("Error:") {
                                self.message = format!("Refreshed {}", self.mode.title());
                            }
                        }
                    }
                    Err(error) if self.mode.hardware_index() == Some(index) => {
                        self.message = format!("Error: {error}")
                    }
                    Err(_) => {}
                }
            }
        }

        fn show_catalog(&mut self) {
            let selected_item = self.items.get(self.selected).cloned();
            self.catalog = match self.mode {
                Mode::Actions => {
                    let mut actions: Vec<_> = CLI_ACTIONS.iter().collect();
                    if self.state.is_none() {
                        actions.sort_by_key(|action| action.needs_project);
                    }
                    actions
                        .into_iter()
                        .map(|action| action.title.to_owned())
                        .collect()
                }
                Mode::Board | Mode::Cable => self.cached_catalogs
                    [self.mode.hardware_index().unwrap()]
                .clone()
                .unwrap_or_default(),
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
            self.sort_catalog();
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
            if let Some(selected) = selected_item.or_else(|| {
                (self.mode == Mode::Board)
                    .then(|| {
                        self.state
                            .as_ref()
                            .and_then(|state| state.project.device.clone())
                            .or_else(|| self.settings.board.clone())
                    })
                    .flatten()
            }) && let Some(index) = self
                .items
                .iter()
                .position(|item| item == &selected || item.ends_with(&selected))
            {
                self.selected = index;
            }
        }

        fn sort_catalog(&mut self) {
            if self.mode == Mode::Board {
                self.catalog.sort_by_cached_key(|item| {
                    let (family, part) = item.split_once("  |  ").unwrap_or(("", item));
                    if self.sort_by_family {
                        (family.to_ascii_lowercase(), part.to_ascii_lowercase())
                    } else {
                        (part.to_ascii_lowercase(), family.to_ascii_lowercase())
                    }
                });
            }
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

        fn switch_mode(&mut self, mode: Mode) -> Action {
            self.mode = mode;
            self.searching = false;
            self.query.clear();
            self.selected = 0;
            self.family_selected = 0;
            self.items.clear();
            Action::Load
        }

        fn handle_form_key(&mut self, key: KeyEvent) -> Action {
            if key.code == KeyCode::Esc
                || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
            {
                self.form = None;
                return Action::None;
            }
            if key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                return Action::None;
            }
            let form = self.form.as_mut().unwrap();
            let fields = CLI_ACTIONS[form.action].fields;
            match key.code {
                KeyCode::Up => form.selected = form.selected.saturating_sub(1),
                KeyCode::Down => form.selected = (form.selected + 1).min(fields.len()),
                KeyCode::Enter if form.selected < fields.len() => form.selected += 1,
                KeyCode::Enter => match form.request() {
                    Ok(request) => {
                        self.pending_command = Some(request);
                        self.form = None;
                    }
                    Err(error) => form.error = error,
                },
                KeyCode::Char(ch) if form.selected < fields.len() => {
                    form.values[form.selected].push(ch);
                    form.error.clear();
                }
                KeyCode::Backspace if form.selected < fields.len() => {
                    form.values[form.selected].pop();
                    form.error.clear();
                }
                KeyCode::Left | KeyCode::Right if form.selected < fields.len() => {
                    let choices = fields[form.selected].choices;
                    if !choices.is_empty() {
                        let index = choices
                            .iter()
                            .position(|choice| *choice == form.values[form.selected]);
                        let next = match (index, key.code) {
                            (Some(index), KeyCode::Left) => {
                                (index + choices.len() - 1) % choices.len()
                            }
                            (Some(index), _) => (index + 1) % choices.len(),
                            (None, _) => 0,
                        };
                        form.values[form.selected] = choices[next].into();
                        form.error.clear();
                    }
                }
                _ => {}
            }
            Action::None
        }

        fn reload_context(&mut self) -> Result<()> {
            self.settings = load_settings()?;
            let manifest = self.root.join(MANIFEST);
            self.state = if manifest.is_file() {
                let state = toml::from_str(&fs::read_to_string(manifest)?)?;
                validate(&state)?;
                Some(state)
            } else {
                None
            };
            self.show_catalog();
            Ok(())
        }

        fn cycle_family(&mut self, previous: bool) {
            if self.mode != Mode::Board || self.searching {
                return;
            }
            let step = if previous { self.families.len() } else { 1 };
            self.family_selected = (self.family_selected + step) % (self.families.len() + 1);
            self.selected = 0;
            self.apply_filter();
        }

        fn handle_key(&mut self, key: KeyEvent) -> Action {
            // Some terminals report both press and release for a single key.
            if key.kind == KeyEventKind::Release {
                return Action::None;
            }
            if self.form.is_some() {
                return self.handle_form_key(key);
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('c') => return Action::Quit,
                    KeyCode::Left => {
                        self.cycle_family(true);
                        return Action::None;
                    }
                    KeyCode::Right => {
                        self.cycle_family(false);
                        return Action::None;
                    }
                    _ => {}
                }
            }
            if key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                return Action::None;
            }
            let code = match key.code {
                KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
                KeyCode::Char('j') if !self.searching => KeyCode::Down,
                KeyCode::Char('k') if !self.searching => KeyCode::Up,
                code => code,
            };
            match code {
                KeyCode::Left | KeyCode::Char('h') if !self.searching => {
                    return self.switch_mode(Mode::from_index((self.mode.index() + 3) % 4));
                }
                KeyCode::Right | KeyCode::Char('l') if !self.searching => {
                    return self.switch_mode(Mode::from_index((self.mode.index() + 1) % 4));
                }
                KeyCode::Char(number @ '1'..='4') if !self.searching => {
                    return self.switch_mode(Mode::from_index(number as usize - '1' as usize));
                }
                KeyCode::Char('q') if !self.searching => return Action::Quit,
                KeyCode::Char('r') if !self.searching => return Action::Refresh,
                KeyCode::Char('f') if !self.searching && self.mode == Mode::Board => {
                    self.sort_by_family = !self.sort_by_family;
                    self.sort_catalog();
                    self.apply_filter();
                }
                KeyCode::Char('/') if !self.searching => {
                    self.searching = true;
                    self.query.clear();
                    self.selected = 0;
                    self.apply_filter();
                }
                KeyCode::Esc => {
                    self.searching = false;
                    self.query.clear();
                    self.apply_filter();
                }
                KeyCode::Char(ch) if self.searching => {
                    self.query.push(ch);
                    self.selected = 0;
                    self.apply_filter();
                }
                KeyCode::Backspace if self.searching => {
                    self.query.pop();
                    self.selected = 0;
                    self.apply_filter();
                }
                KeyCode::BackTab => self.cycle_family(true),
                KeyCode::Tab => self.cycle_family(false),
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1));
                }
                KeyCode::Enter => {
                    self.searching = false;
                    return Action::Select;
                }
                _ => {}
            }
            Action::None
        }

        fn select(&mut self) -> Result<()> {
            let Some(value) = self.items.get(self.selected).cloned() else {
                return Ok(());
            };
            match self.mode {
                Mode::Actions => {
                    let Some(index) = CLI_ACTIONS.iter().position(|action| action.title == value)
                    else {
                        return Ok(());
                    };
                    if CLI_ACTIONS[index].needs_project && self.state.is_none() {
                        self.message = "Create or initialize a project first.".into();
                    } else if CLI_ACTIONS[index].fields.is_empty() {
                        self.pending_command = Some(CommandRequest {
                            action: index,
                            args: CLI_ACTIONS[index]
                                .args
                                .iter()
                                .map(|arg| (*arg).to_owned())
                                .collect(),
                        });
                    } else {
                        self.form = Some(CommandForm::new(index));
                    }
                }
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
            if self.mode == Mode::Settings {
                self.show_catalog();
            }
            Ok(())
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

    fn control_hint(key: &'static str, label: &'static str) -> Vec<Span<'static>> {
        vec![
            Span::styled(
                key,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(label),
        ]
    }

    fn draw_form(frame: &mut Frame, form: &CommandForm) {
        let action = &CLI_ACTIONS[form.action];
        let mut lines = vec![Line::from(action.description), Line::default()];
        for (index, field) in action.fields.iter().enumerate() {
            lines.push(Line::from(Span::styled(
                field.label,
                Style::default().fg(Color::Gray),
            )));
            let focused = form.selected == index;
            let value = if focused {
                format!("> {}▏", form.values[index])
            } else {
                format!("  {}", form.values[index])
            };
            lines.push(Line::from(Span::styled(
                value,
                if focused {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                },
            )));
            lines.push(Line::default());
        }
        lines.push(Line::from(Span::styled(
            if form.selected == action.fields.len() {
                "▶ Run action"
            } else {
                "  Run action"
            },
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::default());
        lines.push(Line::from(
            "↑/↓ fields · Enter next/run · ←/→ choices · Esc cancel",
        ));
        lines.push(Line::from(Span::styled(
            form.error.as_str(),
            Style::default().fg(Color::Red),
        )));
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().borders(Borders::ALL).title(action.title)),
            frame.area(),
        );
    }

    fn draw(frame: &mut Frame, app: &App) {
        if let Some(form) = &app.form {
            draw_form(frame, form);
            return;
        }
        let mut navigation = control_hint(
            if app.searching {
                "↑/↓"
            } else {
                "↑/↓ j/k"
            },
            " move  ",
        );
        navigation.extend(control_hint(
            "↵ Enter",
            if app.mode == Mode::Actions {
                " open/run  "
            } else {
                " save  "
            },
        ));
        navigation.extend(control_hint("Esc", " clear search"));
        let sections = if app.searching {
            control_hint("Esc", " return to navigation")
        } else {
            control_hint("←/→ h/l  1/2/3/4", " sections")
        };
        let mut commands = if app.searching {
            let mut hints = control_hint("⌫ Backspace", " erase  ");
            hints.extend(control_hint("Ctrl+C", " quit"));
            hints
        } else {
            let mut hints = control_hint("/", " search  ");
            hints.extend(control_hint("r", " ↻ refresh  "));
            hints.extend(control_hint("q", " quit"));
            hints
        };
        if app.mode == Mode::Board && !app.searching {
            commands.extend(control_hint("  f", " ⇅ sort"));
        }
        let footer = vec![
            Line::from(navigation),
            Line::from(sections),
            Line::from(commands),
            Line::from(app.message.as_str()),
        ];
        let inner_width = usize::from(frame.area().width.saturating_sub(2)).max(1);
        let footer_height = footer
            .iter()
            .map(|line| line.width().div_ceil(inner_width).max(1))
            .sum::<usize>()
            + 2;
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(if matches!(app.mode, Mode::Board | Mode::Actions) {
                    3
                } else {
                    0
                }),
                Constraint::Min(3),
                Constraint::Length(footer_height as u16),
            ])
            .split(frame.area());
        let tabs = Tabs::new(["[1] Actions", "[2] Boards", "[3] Cables", "[4] Settings"])
            .select(app.mode.index())
            .block(Block::default().borders(Borders::ALL).title("qlm"))
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );
        frame.render_widget(tabs, layout[0]);
        if app.mode == Mode::Board {
            let family = app
                .family_selected
                .checked_sub(1)
                .and_then(|index| app.families.get(index))
                .map_or("All", String::as_str);
            let sort = if app.sort_by_family { "family" } else { "part" };
            let family_selector = Paragraph::new(format!(
                "⇤  {family}  ⇥   ({}/{})   Sorted by {sort}",
                app.family_selected + 1,
                app.families.len() + 1
            ))
            .style(Style::default().fg(Color::Yellow))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(if app.searching {
                        "Board family · Esc to navigate"
                    } else {
                        "Board family · Tab/Shift+Tab · Ctrl+←/→"
                    }),
            );
            frame.render_widget(family_selector, layout[1]);
        }
        if app.mode == Mode::Actions {
            let context = match &app.state {
                Some(state) => format!("Project: {} · {}", state.project.name, app.root.display()),
                None => format!("No project here · {}", app.root.display()),
            };
            frame.render_widget(
                Paragraph::new(context).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Working directory"),
                ),
                layout[1],
            );
        }
        let list_area = if app.mode == Mode::Actions {
            let areas =
                Layout::vertical([Constraint::Min(3), Constraint::Length(4)]).split(layout[2]);
            if let Some(action) = app
                .items
                .get(app.selected)
                .and_then(|title| CLI_ACTIONS.iter().find(|action| action.title == title))
            {
                let description = if action.needs_project && app.state.is_none() {
                    format!(
                        "Create or initialize a project first. {}",
                        action.description
                    )
                } else {
                    action.description.to_owned()
                };
                frame.render_widget(
                    Paragraph::new(description).wrap(Wrap { trim: true }).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("Selected action"),
                    ),
                    areas[1],
                );
            }
            areas[0]
        } else {
            layout[2]
        };
        let mut title = if app.searching {
            format!("{}  / Search: {}▏", app.mode.title(), app.query)
        } else if !app.query.is_empty() {
            format!("{}  / Filter: {}", app.mode.title(), app.query)
        } else {
            app.mode.title().to_owned()
        };
        if app.loading() {
            title.push_str(" · Loading…");
        }
        let block = Block::default().borders(Borders::ALL).title(title);
        if app.items.is_empty() {
            frame.render_widget(
                Paragraph::new(if app.loading() {
                    "Loading hardware… You can keep navigating."
                } else {
                    "No matches. Esc clears the search; r refreshes the list."
                })
                .wrap(Wrap { trim: true })
                .block(block),
                list_area,
            );
        } else {
            let list = List::new(app.items.iter().map(|item| {
                let disabled = app.mode == Mode::Actions
                    && app.state.is_none()
                    && CLI_ACTIONS
                        .iter()
                        .any(|action| action.title == item && action.needs_project);
                ListItem::new(item.as_str()).style(if disabled {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default()
                })
            }))
            .block(block)
            .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
            .highlight_symbol("▶ ");
            let mut state = ListState::default();
            state.select(Some(app.selected));
            frame.render_stateful_widget(list, list_area, &mut state);
        }
        frame.render_widget(
            Paragraph::new(footer)
                .wrap(Wrap { trim: true })
                .block(Block::default().borders(Borders::ALL).title("Controls")),
            layout[3],
        );
    }

    fn execute_command(
        app: &mut App,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
        request: CommandRequest,
    ) -> Result<()> {
        disable_raw_mode()?;
        terminal.show_cursor()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        let result = (|| -> Result<()> {
            let action = &CLI_ACTIONS[request.action];
            println!(
                "\n{}\nWorking directory: {}\n",
                action.title,
                app.root.display()
            );
            stdout().flush()?;
            let status = Command::new(env::current_exe()?)
                .args(&request.args)
                .current_dir(&app.root)
                .status();
            let success = match status {
                Ok(status) => {
                    if status.success() {
                        println!("\nCompleted: {}", action.title);
                    } else {
                        println!("\nFailed: {} ({status})", action.title);
                    }
                    status.success()
                }
                Err(error) => {
                    println!("\nCould not run action: {error}");
                    false
                }
            };
            if success && request.args.first().is_some_and(|arg| arg == "new") {
                app.root = app.root.join(&request.args[1]);
                // A newly opened project may use a different Quartus installation/context.
                app.cached_catalogs = [None, None];
                app.pending_catalogs = [None, None];
            }
            app.message = format!(
                "{}: {}",
                if success { "Completed" } else { "Failed" },
                action.title
            );
            println!("\nPress Enter to return to Actions.");
            stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            app.reload_context()?;
            Ok(())
        })();
        // Restore the UI even when launching a command or reloading its state fails.
        enable_raw_mode()?;
        execute!(terminal.backend_mut(), EnterAlternateScreen)?;
        terminal.clear()?;
        result
    }

    fn run_loop(app: &mut App) -> Result<()> {
        let backend = CrosstermBackend::new(stdout());
        let mut terminal = Terminal::new(backend)?;
        loop {
            app.poll_catalogs();
            terminal.draw(|frame| draw(frame, app))?;
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            let result = match app.handle_key(key) {
                Action::Quit => break,
                Action::Load => app.load(false),
                Action::Refresh => app.load(true),
                Action::Select => app.select(),
                Action::None => Ok(()),
            };
            if let Err(error) = result {
                app.message = format!("Error: {error}");
            }
            if let Some(request) = app.pending_command.take()
                && let Err(error) = execute_command(app, &mut terminal, request)
            {
                app.message = format!("Error: {error}");
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use ratatui::backend::TestBackend;

        fn app() -> App {
            let mut app = App {
                root: PathBuf::new(),
                state: None,
                settings: UserSettings::default(),
                mode: Mode::Board,
                catalog: vec!["Cyclone V  |  5CSE".into(), "MAX 10  |  10M08".into()],
                cached_catalogs: [
                    Some(vec!["Cyclone V  |  5CSE".into(), "MAX 10  |  10M08".into()]),
                    Some(vec!["USB-Blaster".into()]),
                ],
                pending_catalogs: [None, None],
                families: vec!["Cyclone V".into(), "MAX 10".into()],
                family_selected: 0,
                items: Vec::new(),
                query: String::new(),
                selected: 0,
                searching: false,
                sort_by_family: true,
                message: "Ready".into(),
                form: None,
                pending_command: None,
            };
            app.apply_filter();
            app
        }

        fn press(app: &mut App, code: KeyCode) -> Action {
            app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
        }

        fn action_index(title: &str) -> usize {
            CLI_ACTIONS
                .iter()
                .position(|action| action.title == title)
                .unwrap()
        }

        #[test]
        fn actions_are_first_and_do_not_need_quartus_to_load() {
            let mut app = app();
            assert_eq!(Mode::from_index(0), Mode::Actions);
            app.switch_mode(Mode::Actions);
            app.load(false).unwrap();
            assert_eq!(app.items[0], "Create a new project…");
            assert!(app.pending_catalogs.iter().all(Option::is_none));
            app.selected = app
                .items
                .iter()
                .position(|item| item == "Build project")
                .unwrap();
            app.select().unwrap();
            assert!(app.pending_command.is_none());
            assert!(app.message.contains("Create or initialize"));

            app.state = Some(
                toml::from_str(
                    "version = 1\nsources = []\n[project]\nname = 'demo'\ntop = 'demo'\n",
                )
                .unwrap(),
            );
            app.show_catalog();
            press(&mut app, KeyCode::Char('/'));
            for ch in "List source".chars() {
                press(&mut app, KeyCode::Char(ch));
            }
            assert_eq!(app.items, ["List source files"]);
            app.select().unwrap();
            assert_eq!(app.pending_command.as_ref().unwrap().args, ["list"]);
        }

        #[test]
        fn command_forms_validate_values_and_preserve_literal_arguments() {
            let mut form = CommandForm::new(action_index("Create a new project…"));
            assert!(form.request().err().unwrap().contains("required"));
            form.values = vec![
                "project with spaces;$name".into(),
                "demo".into(),
                "vhdl".into(),
                "top".into(),
            ];
            let request = form.request().unwrap();
            assert_eq!(
                request.args,
                [
                    "new",
                    "project with spaces;$name",
                    "--name",
                    "demo",
                    "--lang",
                    "vhdl",
                    "--top",
                    "top"
                ]
            );
            form.values[2] = "invalid".into();
            assert!(form.request().is_err());
            form.values[2].clear();
            assert!(!form.request().unwrap().args.contains(&"--lang".into()));

            let mut form = CommandForm::new(action_index("Add or update a pin…"));
            form.values = vec!["led[0]".into(), "A8".into(), "3.3-V LVTTL".into()];
            assert_eq!(
                form.request().unwrap().args,
                ["pin", "add", "led[0]", "A8", "--iostandard", "3.3-V LVTTL"]
            );
        }

        #[test]
        fn form_keys_edit_fields_choose_values_run_and_cancel() {
            let mut app = app();
            app.mode = Mode::Actions;
            app.form = Some(CommandForm::new(action_index("Install shell completions…")));
            press(&mut app, KeyCode::Right);
            press(&mut app, KeyCode::Right);
            assert_eq!(app.form.as_ref().unwrap().values, ["zsh"]);
            assert_eq!(app.mode, Mode::Actions);
            press(&mut app, KeyCode::Enter);
            assert!(app.pending_command.is_none());
            press(&mut app, KeyCode::Enter);
            assert!(app.form.is_none());
            assert_eq!(
                app.pending_command.take().unwrap().args,
                ["completions", "zsh"]
            );

            app.form = Some(CommandForm::new(action_index("Create a new project…")));
            for ch in "hjklq/1234".chars() {
                press(&mut app, KeyCode::Char(ch));
            }
            assert_eq!(app.form.as_ref().unwrap().values[0], "hjklq/1234");
            assert!(!app.searching);
            assert_eq!(app.mode, Mode::Actions);
            press(&mut app, KeyCode::Esc);
            assert!(app.form.is_none());
            assert!(app.pending_command.is_none());
        }

        #[test]
        fn action_descriptions_and_form_controls_fit_the_terminal() {
            let mut app = app();
            app.switch_mode(Mode::Actions);
            app.load(false).unwrap();
            for width in [60, 80] {
                let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
                terminal.draw(|frame| draw(frame, &app)).unwrap();
                let screen = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                for text in [
                    "[1] Actions",
                    "No project here",
                    "Create a new project",
                    "Selected action",
                    "open/run",
                ] {
                    assert!(screen.contains(text), "Missing {text} at width {width}");
                }
                app.form = Some(CommandForm::new(action_index("Create a new project…")));
                terminal.draw(|frame| draw(frame, &app)).unwrap();
                let screen = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                for text in [
                    "New directory",
                    "Project name",
                    "Language",
                    "Top-level module",
                    "Run action",
                    "Esc cancel",
                ] {
                    assert!(screen.contains(text), "Missing {text} at width {width}");
                }
                app.form = None;
            }
        }

        #[test]
        fn shortcuts_work_outside_search() {
            let mut app = app();
            for (key, mode) in [
                ('1', Mode::Actions),
                ('3', Mode::Cable),
                ('4', Mode::Settings),
                ('2', Mode::Board),
            ] {
                assert_eq!(press(&mut app, KeyCode::Char(key)), Action::Load);
                assert_eq!(app.mode, mode);
                assert!(!app.searching);
            }
            assert_eq!(press(&mut app, KeyCode::Char('f')), Action::None);
            assert!(!app.sort_by_family);
            assert_eq!(press(&mut app, KeyCode::Char('r')), Action::Refresh);
            assert_eq!(press(&mut app, KeyCode::Char('q')), Action::Quit);
            assert!(app.query.is_empty());
        }

        #[test]
        fn search_starts_only_with_slash() {
            let mut app = app();
            for mode in [Mode::Board, Mode::Cable, Mode::Settings] {
                app.mode = mode;
                for ch in "abcxyz056789 .".chars() {
                    assert_eq!(press(&mut app, KeyCode::Char(ch)), Action::None);
                    assert!(!app.searching);
                    assert!(app.query.is_empty());
                }
            }
            press(&mut app, KeyCode::Char('/'));
            assert!(app.searching);
            press(&mut app, KeyCode::Char('m'));
            assert_eq!(app.query, "m");
            press(&mut app, KeyCode::Esc);
            press(&mut app, KeyCode::Char('m'));
            assert!(!app.searching);
            assert!(app.query.is_empty());
        }

        #[test]
        fn search_accepts_shortcut_characters_and_resets_cleanly() {
            let mut app = app();
            press(&mut app, KeyCode::Char('/'));
            for ch in "123frq".chars() {
                assert_eq!(press(&mut app, KeyCode::Char(ch)), Action::None);
            }
            assert_eq!(app.query, "123frq");
            assert_eq!(app.mode, Mode::Board);
            assert!(app.items.is_empty());
            press(&mut app, KeyCode::Down);
            assert_eq!(app.selected, 0);
            press(&mut app, KeyCode::Backspace);
            assert_eq!(app.query, "123fr");
            press(&mut app, KeyCode::Esc);
            assert!(!app.searching);
            assert_eq!(app.items.len(), 2);
            press(&mut app, KeyCode::Char('/'));
            press(&mut app, KeyCode::Char('m'));
            assert_eq!(app.items, ["MAX 10  |  10M08"]);
            assert_eq!(press(&mut app, KeyCode::Enter), Action::Select);
            assert!(!app.searching);
            // Starting a fresh search must immediately remove the old filter.
            press(&mut app, KeyCode::Char('/'));
            assert_eq!(app.items.len(), 2);
        }

        #[test]
        fn control_arrows_cycle_families_without_switching_sections() {
            let mut app = app();
            let left = KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL);
            let right = KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL);
            assert_eq!(app.handle_key(right), Action::None);
            assert_eq!(app.items, ["Cyclone V  |  5CSE"]);
            app.handle_key(left);
            assert_eq!(app.family_selected, 0);
            app.handle_key(left);
            assert_eq!(app.items, ["MAX 10  |  10M08"]);
            app.handle_key(right);
            assert_eq!(app.family_selected, 0);
            assert_eq!(app.mode, Mode::Board);
            assert!(app.pending_catalogs.iter().all(Option::is_none));

            press(&mut app, KeyCode::Char('/'));
            press(&mut app, KeyCode::Char('m'));
            app.handle_key(left);
            app.handle_key(right);
            assert_eq!(app.mode, Mode::Board);
            assert!(app.searching);
            assert_eq!(app.query, "m");
            assert_eq!(app.family_selected, 0);
            press(&mut app, KeyCode::Esc);
            for mode in [Mode::Cable, Mode::Settings] {
                app.switch_mode(mode);
                app.load(false).unwrap();
                let items = app.items.clone();
                app.handle_key(left);
                app.handle_key(right);
                assert_eq!(app.mode, mode);
                assert_eq!(app.items, items);
            }
        }

        #[test]
        fn cached_sections_and_sorting_do_not_start_hardware_scans() {
            let mut app = app();
            for _ in 0..3 {
                for mode in [Mode::Cable, Mode::Settings, Mode::Actions, Mode::Board] {
                    assert_eq!(app.switch_mode(mode), Action::Load);
                    app.load(false).unwrap();
                    assert!(!app.items.is_empty());
                    assert!(app.pending_catalogs.iter().all(Option::is_none));
                }
            }
            press(&mut app, KeyCode::Char('f'));
            assert_eq!(app.items[0], "MAX 10  |  10M08");
            assert!(app.pending_catalogs.iter().all(Option::is_none));
            app.switch_mode(Mode::Settings);
            app.settings.language = "vhdl".into();
            app.load(false).unwrap();
            assert!(app.items.contains(&"language = vhdl".into()));
        }

        #[test]
        fn pending_scan_does_not_block_navigation_or_replace_another_section() {
            let mut app = app();
            let (sender, receiver) = mpsc::channel();
            app.cached_catalogs[1] = None;
            app.pending_catalogs[1] = Some(receiver);
            app.switch_mode(Mode::Cable);
            app.load(false).unwrap();
            assert!(app.loading());
            assert!(app.items.is_empty());
            app.poll_catalogs();
            assert!(app.loading());
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let screen = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(screen.contains("Loading hardware"));

            app.switch_mode(Mode::Settings);
            app.load(false).unwrap();
            let settings = app.items.clone();
            sender.send(Ok(vec!["New cable".into()])).unwrap();
            app.poll_catalogs();
            assert_eq!(app.items, settings);
            assert!(!app.loading());
            app.switch_mode(Mode::Cable);
            app.load(false).unwrap();
            assert_eq!(app.items, ["New cable"]);
            assert!(!app.loading());
        }

        #[test]
        fn refresh_keeps_cached_entries_and_caches_empty_results() {
            let mut app = app();
            app.switch_mode(Mode::Cable);
            app.load(false).unwrap();
            let (sender, receiver) = mpsc::channel();
            app.pending_catalogs[1] = Some(receiver);
            // Repeated refresh presses reuse the scan already in progress.
            app.load(true).unwrap();
            assert_eq!(app.items, ["USB-Blaster"]);
            sender.send(Err("Cable scan failed".into())).unwrap();
            app.poll_catalogs();
            assert_eq!(app.items, ["USB-Blaster"]);
            assert!(app.message.contains("Cable scan failed"));
            assert!(!app.loading());

            let (sender, receiver) = mpsc::channel();
            app.pending_catalogs[1] = Some(receiver);
            sender.send(Ok(Vec::new())).unwrap();
            app.poll_catalogs();
            assert!(!app.message.contains("Cable scan failed"));
            app.switch_mode(Mode::Settings);
            app.load(false).unwrap();
            app.switch_mode(Mode::Cable);
            app.load(false).unwrap();
            assert!(app.items.is_empty());
            assert!(!app.loading());
        }

        #[test]
        fn background_results_respect_search_started_during_loading() {
            let mut app = app();
            let (sender, receiver) = mpsc::channel();
            let catalog = app.cached_catalogs[0].take().unwrap();
            app.pending_catalogs[0] = Some(receiver);
            app.show_catalog();
            press(&mut app, KeyCode::Char('/'));
            press(&mut app, KeyCode::Char('m'));
            sender.send(Ok(catalog)).unwrap();
            app.poll_catalogs();
            assert_eq!(app.items, ["MAX 10  |  10M08"]);
            assert_eq!(app.query, "m");
            assert!(app.searching);
        }

        #[test]
        fn vertical_arrows_stay_in_bounds_and_tab_cycles_families() {
            let mut app = app();
            for _ in 0..3 {
                press(&mut app, KeyCode::Down);
            }
            assert_eq!(app.selected, 1);
            press(&mut app, KeyCode::Tab);
            assert_eq!(app.items, ["Cyclone V  |  5CSE"]);
            assert_eq!(app.selected, 0);
            press(&mut app, KeyCode::Tab);
            assert_eq!(app.items, ["MAX 10  |  10M08"]);
            press(&mut app, KeyCode::Tab);
            assert_eq!(app.family_selected, 0);
            press(&mut app, KeyCode::BackTab);
            assert_eq!(app.items, ["MAX 10  |  10M08"]);
            app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT));
            assert_eq!(app.items, ["Cyclone V  |  5CSE"]);
            press(&mut app, KeyCode::BackTab);
            for _ in 0..3 {
                press(&mut app, KeyCode::Up);
            }
            assert_eq!(app.family_selected, 0);
            assert_eq!(app.items.len(), 2);
            assert_eq!(app.selected, 0);
        }

        #[test]
        fn vim_keys_navigate_without_interfering_with_search() {
            let mut app = app();
            for mode in [Mode::Board, Mode::Cable, Mode::Settings] {
                app.mode = mode;
                for _ in 0..3 {
                    press(&mut app, KeyCode::Char('j'));
                }
                assert_eq!(app.selected, 1);
                for _ in 0..3 {
                    press(&mut app, KeyCode::Char('k'));
                }
                assert_eq!(app.selected, 0);
                assert!(!app.searching);
            }
            app.mode = Mode::Board;
            for (key, modes) in [
                (
                    KeyCode::Char('l'),
                    [Mode::Cable, Mode::Settings, Mode::Actions, Mode::Board],
                ),
                (
                    KeyCode::Char('h'),
                    [Mode::Actions, Mode::Settings, Mode::Cable, Mode::Board],
                ),
                (
                    KeyCode::Right,
                    [Mode::Cable, Mode::Settings, Mode::Actions, Mode::Board],
                ),
                (
                    KeyCode::Left,
                    [Mode::Actions, Mode::Settings, Mode::Cable, Mode::Board],
                ),
            ] {
                for mode in modes {
                    assert_eq!(press(&mut app, key), Action::Load);
                    app.load(false).unwrap();
                    assert_eq!(app.mode, mode);
                    assert_eq!(app.family_selected, 0);
                    assert!(!app.loading());
                }
            }
            assert_eq!(app.family_selected, 0);
            assert_eq!(app.items.len(), 2);
            press(&mut app, KeyCode::Char('/'));
            for ch in "hjkl".chars() {
                press(&mut app, KeyCode::Char(ch));
            }
            assert_eq!(app.query, "hjkl");
            assert_eq!(app.mode, Mode::Board);
            assert_eq!(app.family_selected, 0);
            assert!(app.items.is_empty());
            press(&mut app, KeyCode::Enter);
            press(&mut app, KeyCode::Char('j'));
            assert_eq!(app.selected, 0);
        }

        #[test]
        fn function_keys_and_release_events_do_not_navigate() {
            let mut app = app();
            for code in [KeyCode::F(1), KeyCode::F(2), KeyCode::F(3)] {
                assert_eq!(press(&mut app, code), Action::None);
            }
            let mut key = KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE);
            key.kind = KeyEventKind::Release;
            assert_eq!(app.handle_key(key), Action::None);
            assert_eq!(app.mode, Mode::Board);
            assert!(!app.searching);
            press(&mut app, KeyCode::Char('/'));
            assert_eq!(
                app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
                Action::Quit
            );
        }

        #[test]
        fn controls_are_visible_and_match_the_current_mode() {
            let mut app = app();
            for width in [60, 80, 120] {
                let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
                terminal.draw(|frame| draw(frame, &app)).unwrap();
                let screen = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                for hint in [
                    "[1] Actions",
                    "[2] Boards",
                    "[3] Cables",
                    "[4] Settings",
                    "↑/↓",
                    "j/k",
                    "h/l",
                    "↵ Enter",
                    "Ctrl+←/→",
                    "↻ refresh",
                    "⇅ sort",
                    "Ready",
                ] {
                    assert!(screen.contains(hint), "Missing {hint} at width {width}");
                }
                assert!(screen.contains("Tab/Shift+Tab"));
                assert!(!screen.contains("F1"));
            }
            press(&mut app, KeyCode::Char('/'));
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let screen = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(screen.contains("⌫ Backspace"));
            assert!(screen.contains("Ctrl+C"));
            assert!(!screen.contains("⇅ sort"));
            assert!(!screen.contains("1/2/3/4 sections"));
            assert!(!screen.contains("j/k"));
            assert!(!screen.contains("h/l"));
        }
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

    struct CompletionWorkspace(PathBuf);

    impl CompletionWorkspace {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root =
                env::temp_dir().join(format!("qlm-completion-{}-{stamp}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for CompletionWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn completions_install_files_and_preserve_startup_configuration() {
        let workspace = CompletionWorkspace::new();
        let home = workspace.0.join("home");
        let config = workspace.0.join("config with 'quotes' and $variables");
        let zsh = workspace.0.join("zsh");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&zsh).unwrap();
        fs::write(home.join(".bashrc"), "# existing bash config").unwrap();
        fs::write(home.join(".profile"), "# existing login config\n").unwrap();
        fs::write(zsh.join(".zshrc"), "# existing zsh config\n").unwrap();
        for shell in ["bash", "zsh", "fish"] {
            let paths = completion_paths(shell, &home, &config, &zsh).unwrap();
            install_completions(shell, &paths).unwrap();
            let first: Vec<_> = paths
                .startup
                .iter()
                .map(|path| fs::read_to_string(path).unwrap())
                .collect();
            fs::write(&paths.script, "outdated completion").unwrap();
            install_completions(shell, &paths).unwrap();
            assert_eq!(
                fs::read_to_string(&paths.script).unwrap(),
                completion_script(shell).unwrap()
            );
            for (path, before) in paths.startup.iter().zip(first) {
                let after = fs::read_to_string(path).unwrap();
                assert_eq!(before, after);
                assert!(after.starts_with("# existing"));
                assert_eq!(
                    after
                        .matches(&format!("# >>> qlm {shell} completions >>>"))
                        .count(),
                    1
                );
            }
        }
        assert!(
            !home.join(".bash_profile").exists(),
            "existing login profile must remain active"
        );
        assert!(!home.join(".zshrc").exists(), "ZDOTDIR must be respected");
        assert!(config.join("fish/completions/qlm.fish").exists());
        assert!(!config.join("fish/config.fish").exists());
    }

    #[test]
    fn completion_startup_updates_only_its_managed_block() {
        let old = completion_startup("# before\n", "zsh", Path::new("/old/qlm.zsh")).unwrap()
            + "# after\n";
        let updated = completion_startup(&old, "zsh", Path::new("/new/qlm.zsh")).unwrap();
        assert!(updated.starts_with("# before\n"));
        assert!(updated.ends_with("# after\n"));
        assert!(!updated.contains("/old/"));
        assert!(updated.contains("/new/"));
        assert!(
            completion_startup("# >>> qlm zsh completions >>>\n", "zsh", Path::new("/new"))
                .is_err()
        );
    }

    #[test]
    fn removing_completions_preserves_other_shells_and_user_settings() {
        let workspace = CompletionWorkspace::new();
        let config = workspace.0.join("config");
        let zsh = completion_paths("zsh", &workspace.0, &config, &workspace.0).unwrap();
        let fish = completion_paths("fish", &workspace.0, &config, &workspace.0).unwrap();
        fs::write(&zsh.startup[0], "# keep before\n").unwrap();
        install_completions("zsh", &zsh).unwrap();
        install_completions("fish", &fish).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&zsh.startup[0])
            .unwrap()
            .write_all(b"# keep after\n")
            .unwrap();
        assert!(remove_completions("zsh", &zsh).unwrap());
        assert!(!zsh.script.exists());
        assert!(fish.script.exists());
        assert_eq!(
            fs::read_to_string(&zsh.startup[0]).unwrap(),
            "# keep before\n# keep after\n"
        );
        assert!(!remove_completions("zsh", &zsh).unwrap());
        assert!(remove_completions("fish", &fish).unwrap());
        assert!(!fish.script.exists());
        assert!(!remove_completions("fish", &fish).unwrap());
    }

    #[test]
    fn removing_bash_completions_checks_old_login_profiles() {
        let workspace = CompletionWorkspace::new();
        fs::write(workspace.0.join(".profile"), "# old login profile\n").unwrap();
        let paths = completion_paths("bash", &workspace.0, &workspace.0, &workspace.0).unwrap();
        install_completions("bash", &paths).unwrap();
        fs::write(workspace.0.join(".bash_profile"), "# new login profile\n").unwrap();
        let paths = completion_paths("bash", &workspace.0, &workspace.0, &workspace.0).unwrap();
        assert!(remove_completions("bash", &paths).unwrap());
        assert_eq!(
            fs::read_to_string(workspace.0.join(".profile")).unwrap(),
            "# old login profile\n"
        );
        assert_eq!(
            fs::read_to_string(workspace.0.join(".bash_profile")).unwrap(),
            "# new login profile\n"
        );
        assert!(
            fs::read_to_string(workspace.0.join(".bashrc"))
                .unwrap()
                .is_empty()
        );
        assert!(!workspace.0.join(".bash_login").exists());
    }

    #[test]
    fn removal_validates_startup_blocks_before_deleting_files() {
        let workspace = CompletionWorkspace::new();
        let paths = completion_paths("zsh", &workspace.0, &workspace.0, &workspace.0).unwrap();
        install_completions("zsh", &paths).unwrap();
        let incomplete = "# >>> qlm zsh completions >>>\n# preserve this\n";
        fs::write(&paths.startup[0], incomplete).unwrap();
        assert!(remove_completions("zsh", &paths).is_err());
        assert!(paths.script.exists());
        assert_eq!(fs::read_to_string(&paths.startup[0]).unwrap(), incomplete);
    }

    #[test]
    fn invalid_startup_blocks_leave_all_completion_files_unchanged() {
        let workspace = CompletionWorkspace::new();
        let paths = completion_paths("bash", &workspace.0, &workspace.0, &workspace.0).unwrap();
        fs::write(&paths.startup[0], "# keep this\n").unwrap();
        fs::write(&paths.startup[1], "# >>> qlm bash completions >>>\n").unwrap();
        assert!(install_completions("bash", &paths).is_err());
        assert_eq!(
            fs::read_to_string(&paths.startup[0]).unwrap(),
            "# keep this\n"
        );
        assert!(!paths.script.exists());
    }

    #[test]
    #[cfg(unix)]
    fn installed_completions_load_in_available_shells() {
        let workspace = CompletionWorkspace::new();
        let config = workspace.0.join("config 'quoted' $(literal)");
        for shell in ["bash", "zsh", "fish"] {
            if Command::new(shell).arg("--version").output().is_err() {
                continue;
            }
            let paths = completion_paths(shell, &workspace.0, &config, &workspace.0).unwrap();
            install_completions(shell, &paths).unwrap();
            let mut command = Command::new(shell);
            match shell {
                "bash" => {
                    command
                        .arg("--noprofile")
                        .arg("--rcfile")
                        .arg(&paths.startup[0])
                        .args(["-ic", "complete -p qlm"]);
                }
                "zsh" => {
                    command.env("ZDOTDIR", &workspace.0).args([
                        "-d",
                        "-ic",
                        "print -r -- $_comps[qlm]",
                    ]);
                }
                "fish" => {
                    command
                        .env("XDG_CONFIG_HOME", &config)
                        .args(["-c", "complete -C 'qlm b'"]);
                }
                _ => unreachable!(),
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{shell}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                stdout.contains(if shell == "fish" { "build" } else { "_qlm" }),
                "{shell}: {stdout}"
            );
        }
    }

    #[test]
    #[ignore = "requires installed Quartus Lite; run cargo test -- --ignored"]
    fn real_quartus_automatic_project_sync() {
        struct Workspace(PathBuf);
        impl Drop for Workspace {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let workspace =
            Workspace(env::temp_dir().join(format!("qlm-sync-{}-{stamp}", std::process::id())));
        let root = &workspace.0;
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/demo.sv"), "module demo; endmodule\n").unwrap();
        let source = "src/extra $value [literal].v";
        fs::write(root.join(source), "module extra; endmodule\n").unwrap();
        fs::write(
            root.join("constraints.tcl"),
            "set_global_assignment -name SEED 7\n",
        )
        .unwrap();
        let mut state = Manifest {
            version: 1,
            project: Project {
                name: "demo".into(),
                family: "Cyclone V".into(),
                device: None,
                board: None,
                top: "demo".into(),
            },
            sources: Vec::new(),
            settings: Some("constraints.tcl".into()),
            programmer: None,
        };
        reconcile_sources(root, &mut state).unwrap();
        sync_project(root, &state).unwrap();
        let text = fs::read_to_string(root.join("demo.qsf")).unwrap();
        assert!(root.join("demo.qpf").exists());
        assert!(text.contains("SYSTEMVERILOG_FILE"));
        assert!(text.contains("SEED 7"));
        assert!(text.contains("VERILOG_FILE"));
        assert!(text.contains("literal"));
        fs::remove_file(root.join(source)).unwrap();
        reconcile_sources(root, &mut state).unwrap();
        sync_project(root, &state).unwrap();
        let text = fs::read_to_string(root.join("demo.qsf")).unwrap();
        assert!(!text.contains("literal"), "stale source was not removed");
    }

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
