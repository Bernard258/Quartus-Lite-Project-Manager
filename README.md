# Quartus Manager (`qlm`)

A Cargo-style Rust CLI for Quartus Lite. Run commands from your project directory
or any subdirectory: `qlm` finds the nearest `Quartus.toml` automatically. There is
no environment to activate. The manifest saves the target FPGA, HDL sources,
constraints script, and programming cable; generated Quartus files and build
output live in the project directory.

## Install

Install the latest stable release (Linux x86_64, macOS Intel or Apple Silicon):

```sh
wget -qO- https://raw.githubusercontent.com/Bernard258/Quartus-Lite-Project-Manager/main/scripts/install.sh | sh
```

Or use curl:

```sh
curl -fsSL https://raw.githubusercontent.com/Bernard258/Quartus-Lite-Project-Manager/main/scripts/install.sh | sh
```

The installer downloads the matching `qlm-*` release asset to `~/.local/bin/qlm`.
It requires curl or wget, but does not require Rust or sudo. Run it again to
update. A published, non-prerelease GitHub release with the matching binary is
required; if one is unavailable, build from source below.

Add the install directory to your shell's startup file (for example `~/.bashrc`
or `~/.zshrc`) and reload the shell:

```sh
export PATH="$HOME/.local/bin:$PATH"
qlm --version
```

For a custom directory, set `QLM_INSTALL_DIR` on the `sh` side of the pipe:

```sh
wget -qO- https://raw.githubusercontent.com/Bernard258/Quartus-Lite-Project-Manager/main/scripts/install.sh | QLM_INSTALL_DIR="$HOME/bin" sh
```

To install a specific release, pass its tag (including prerelease tags) to `sh`:

```sh
wget -qO- https://raw.githubusercontent.com/Bernard258/Quartus-Lite-Project-Manager/main/scripts/install.sh | sh -s -- v0.1.0
```

The default is `latest`. Set `QLM_REPOSITORY="owner/repository"` on the `sh`
side of the pipe to download release assets from a fork instead. Its release
assets must use the same `qlm-*` names.

### Uninstall

```sh
wget -qO- https://raw.githubusercontent.com/Bernard258/Quartus-Lite-Project-Manager/main/scripts/uninstall.sh | sh
```

Or use curl:

```sh
curl -fsSL https://raw.githubusercontent.com/Bernard258/Quartus-Lite-Project-Manager/main/scripts/uninstall.sh | sh
```

If you installed into a custom directory, use the same override:

```sh
wget -qO- https://raw.githubusercontent.com/Bernard258/Quartus-Lite-Project-Manager/main/scripts/uninstall.sh | QLM_INSTALL_DIR="$HOME/bin" sh
```

The uninstaller removes only `qlm` from the selected directory. It keeps your
projects, user settings, and shell configuration. For a Cargo installation,
use `cargo uninstall quartus-manager` instead.

### Build from source

To build from source, install a current stable Rust toolchain, then run:

```sh
git clone https://github.com/Bernard258/Quartus-Lite-Project-Manager.git
cd Quartus-Lite-Project-Manager
cargo install --path . --locked
```

Cargo installs `qlm` into `~/.cargo/bin` by default; add that directory to `PATH`.
Windows binaries are not currently provided. Installing `qlm` does not install
Quartus, device support, or USB-Blaster drivers. The macOS CLI binary alone does
not provide a Quartus toolchain; hardware commands require working local Quartus
executables.

Quartus tools must be on `PATH`. Alternatively, set `QUARTUS_SH` to the absolute
path of `quartus_sh`. Programming then uses `quartus_pgm` from the same directory,
unless `QUARTUS_PGM` is explicitly set.

## Project workflow

```sh
qlm new blinky
cd blinky
qlm device families
qlm device list 10M08
qlm device set 10M08DAF256C8G  # Example only: select the part on your board
qlm device show

# Edit src/blinky.sv and constraints.tcl for your board.
qlm build

qlm cable list
qlm cable set "USB-Blaster [USB-0]" --index 1
qlm program
```

If Linux has not configured USB-Blaster access yet, install the udev rule from
the program and reconnect the cable:

```sh
qlm cable permissions
```

For a DE10-Lite, use the board name and skip the part and cable details:
`qlm device set de10-lite` selects `10M50DAF484C7G`, and `qlm cable set de10-lite`
selects the usual `USB-Blaster [USB-0]` cable. The board name also accepts
`de10lite`.
`qlm device detect` identifies the FPGA reported by the JTAG chain and validates it
against the installed Quartus device database, so it works across Quartus
families and board types. Because a USB-Blaster reports the FPGA rather than a
physical board model, `10M50DAF484C7G` is recognized as a DE10-Lite.

User preferences are stored in `~/.config/qlm/settings.toml` (or under
`$XDG_CONFIG_HOME`). Set the board once, then project setup becomes:

```sh
qlm config set board de10-lite
qlm device set
qlm cable set
```

`qlm config set cable <name>`, `qlm config set jtag-index <number>`, and
`qlm config set language <systemverilog|verilog|vhdl>` set other defaults.
`language` may also be written as `default-language`; it controls the starter
file created by future `qlm new` or `qlm init` commands.
Use `qlm config edit` to open the file in `$VISUAL`, `$EDITOR`, or `vi`; use
`qlm config show` or `qlm config path` to inspect it without opening an editor.

For a searchable terminal interface, launch the built-in TUI from any directory
(it updates the current project too when `Quartus.toml` is found):

```sh
qlm tui
```

Use `Tab` or `1`/`2`/`3` to switch between boards/devices, cables, and settings.
The board view renders family tabs; use `←`/`→` to switch groups and `All` to
see every part. Press `/` or start typing to search the current list. Press
`f` to switch board sorting between family and part, `Enter` to save a selection,
`r` to refresh hardware lists, and `q` to quit. The settings view lets you
set the DE10-Lite board/cable defaults and cycle the JTAG index and HDL language.

`qlm init` or `qlm new` without a directory scaffolds the current directory.
Both `new` and `init` accept
`--name`, `--top`, `--family`, `--device`, and `--lang` (`systemverilog`, `verilog`,
or `vhdl`). `--device` validates the part through Quartus and derives its family.
You can omit target options entirely and select a device later. Scaffolding
without `--device` does not require Quartus.

When run interactively without an explicit device, project creation checks for
connected programming cables. It asks before saving a detected cable and asks
whether to probe the board and use its detected device. Non-interactive runs
skip this hardware prompt.

The starter HDL is a minimal pass-through between `clk` and `led`, not a timed
LED blinker. Set pin locations, I/O standards, and timing constraints for your
actual board before programming it.

```text
blinky/
├── Quartus.toml          # Persistent project configuration
├── constraints.tcl      # Board-specific Quartus assignments
├── .gitignore
├── src/blinky.sv
├── blinky.qpf            # Generated by sync/build; ignored by Git
├── blinky.qsf            # Generated by sync/build; ignored by Git
└── output_files/blinky.sof # Generated bitstream; ignored by Git
```

## Commands

The following top-level commands accept one-letter aliases: `n` (new), `i` (init),
`a` (add), `r` (remove), `l` (list), `s` (sync), `d` (device), `c` (cable),
`b` (build), `p` (program), and `h` (help). Arguments and options are unchanged:
for example, `qlm n`, `qlm n blinky`, or `qlm d set 10M08DAF256C8G`.

Shell completion is generated without an extra package. Enable it for the
current shell with one of these commands:

```sh
# zsh
eval "$(qlm completions zsh)"

# bash
eval "$(qlm completions bash)"

# fish
qlm completions fish | source
```

The completion covers command aliases and context-specific subcommands such as
`qlm device ...`, `qlm cable ...`, `qlm config ...`, and `qlm pin ...`. Put the
matching `eval` or `source` command in your shell startup file (for example
`~/.zshrc`, `~/.bashrc`, or `~/.config/fish/config.fish`) to enable it in every
new terminal.

| Command | Behavior |
| --- | --- |
| `qlm new <directory>` | Scaffold a new directory |
| `qlm new` | Scaffold the current directory (same as `init`) |
| `qlm init` | Scaffold the current directory |
| `qlm device families` | Query installed Quartus family support |
| `qlm device list [filter]` | List installed FPGA parts, optionally filtering by substring |
| `qlm device select [filter]` | Show matching parts and save a numbered or typed selection |
| `qlm device detect [--default]` | Detect the connected FPGA over JTAG, save the board in the project, and optionally make it the user default |
| `qlm device set <part-or-board>` | Validate and save the part and family (`de10-lite` is a shortcut) |
| `qlm device <part>` | Direct shortcut for `device set <part>` |
| `qlm device show` | Display the saved target |
| `qlm add <source>...` | Register existing HDL files |
| `qlm add --auto` | Recursively find and register `.v`, `.sv`, `.vhd`, and `.vhdl` files |
| `qlm remove <source>...` | Unregister files without deleting them |
| `qlm list` | List registered sources |
| `qlm sync` | Generate Quartus project files from saved state |
| `qlm build` | Validate the target, sync, and compile with live Quartus output |
| `qlm cable list` | List connected programming cables |
| `qlm cable select` | List connected cables and prompt for the cable name |
| `qlm cable set <name-or-number-or-board> [--index N]` | Save a cable and JTAG position (default 1); `de10-lite` selects the standard USB-Blaster |
| `qlm cable <name>` | Direct shortcut for `cable set <name>` |
| `qlm cable` | Show saved programming settings |
| `qlm cable permissions` | Install the Linux USB-Blaster udev rule using sudo, then reload udev |
| `qlm program` | Load the existing `.sof` through JTAG |
| `qlm program build` | Build, then load the generated `.sof` through JTAG |
| `qlm pin list` | Show current pin assignments |
| `qlm pin add <signal> <pin> [--iostandard <standard>]` | Add or replace one pin assignment |
| `qlm pin import <csv>` | Add or replace assignments from `signal,pin[,iostandard]` rows |
| `qlm pin auto` | Detect the board when needed and apply the DE10-Lite starter pin profile |
| `qlm pin detect` | Detect the board when needed and apply the known pin profile |
| `qlm pin edit` | Open the constraints file in `$VISUAL`, `$EDITOR`, or `vi` |
| `qlm config show` | Show user defaults |
| `qlm config set <key> <value>` | Set a user default |
| `qlm config path` | Print the settings file path |
| `qlm config edit` | Open user settings in an editor |
| `qlm tui` | Open the terminal interface (enabled in default builds) |
| `qlm completions <shell>` | Generate shell completions |
| `qlm help` | Show top-level help |
| `qlm --version` | Show the CLI version |

`device set` queries the installed Quartus device database. A part unavailable in
that installation is rejected without changing the manifest. `build` requires an
explicit device; `AUTO` is only supported for project generation with `sync`.

`program` uses the selected cable and its one-based JTAG chain position. A stable
cable name is preferable to a number when multiple cables are attached. `qlm program`
loads the existing `.sof` without rebuilding or checking whether sources changed.
Use `qlm program build` to compile first; it stops on build failure or a missing
bitstream. Programming configures FPGA SRAM; persistent flash programming and `.pof` operations are not
implemented. Actual programming also requires a connected board and functioning
Quartus cable drivers/permissions.

If detection reports `Insufficient port permissions`, Quartus has found the
USB-Blaster but Linux is denying access to its JTAG device. Install the
USB-Blaster udev rule supplied with Quartus (or add the equivalent rule for
vendor `09fb`), reload udev, unplug and reconnect the cable, and make sure your
user belongs to the rule's group. Quartus includes the exact rule instructions
at `qprogrammer/drivers/linux_drivers_install_instruction.txt`; after creating
`/etc/udev/rules.d/92-usbblaster.rules`, apply it with:

```sh
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Unplug and reconnect the cable, then retry `qlm device detect`.

## Saved state and source management

Example `Quartus.toml` after configuration:

```toml
version = 1
sources = ["src/blinky.sv"]
settings = "constraints.tcl"

[project]
name = "blinky"
family = "MAX 10"
device = "10M50DAF484C7G"
board = "de10-lite" # Added by qlm device detect for recognized boards
top = "blinky"

[programmer]
cable = "USB-Blaster [USB-0]"
index = 1
```

Commit the manifest, constraints, and HDL. Generated Quartus files in the project
directory can be regenerated. There is no virtual environment or HDL package
dependency lockfile.

The manifest's detected `project.board` selects the board profile; for a
DE10-Lite it resolves to `10M50DAF484C7G` during sync and build. The
`programmer.cable` and `programmer.index` entries are used by `qlm program`, so
programming uses the cable saved in that project.

Source arguments are relative to the current directory. Stored source paths are
relative to the project root, and shared files can use `../` paths. Supported HDL
extensions are `.v`, `.sv`, `.vhd`, and `.vhdl`. VHDL files are emitted as
Quartus `VHDL_FILE` assignments and compile alongside Verilog/SystemVerilog files.
Use `qlm add --auto` to scan the project recursively; generated `.git`, `target`,
`.qlm`, and `output_files` directories are skipped. Adding a registered path is a
no-op; `qlm sync`, `qlm build`, and `qlm program build` also reconcile the manifest,
adding present HDL files and removing missing registrations. Files are referenced rather than copied. Batch changes are validated before
saving. Manifest updates use atomic replacement but do not preserve TOML comments
or custom formatting. Avoid simultaneous commands modifying the same project.

`new <directory>` refuses an existing target directory. `init` and `new` without
a directory refuse to overwrite the
manifest, starter HDL, or constraints script; it preserves existing `.gitignore`
entries. Existing unmanaged Quartus projects cannot yet be imported.

## Constraints and Quartus integration

`constraints.tcl` is sourced by Quartus from the project root. Add board-specific
assignments there. For file assignments, use absolute paths resolved by Tcl so
Quartus can find them from its generated project directory:

```tcl
# Replace these with the correct pins and standard for your board.
# set_location_assignment PIN_A1 -to clk
# set_instance_assignment -name IO_STANDARD "3.3-V LVTTL" -to clk
# set_global_assignment -name SDC_FILE [file normalize constraints.sdc]
```

The manifest is authoritative for generated settings. Sync/build regenerates the
`.qpf`/`.qsf`; put permanent customizations in `constraints.tcl`, not in generated
files or GUI-only changes. Keep device, top, output directory, and source settings
in the manifest rather than overriding them in the constraints script.

Project and device operations use the [Quartus Tcl project API](https://www.intel.com/content/www/us/en/docs/programmable/683432/23-2/tcl_pkg_project_ver_7-0.html).
Compilation invokes [`quartus_sh --flow compile`](https://www.intel.com/content/www/us/en/docs/programmable/683432/22-4/compilation-with-quartus-sh-flow.html).
Hardware operations invoke [`quartus_pgm`](https://www.intel.com/content/www/us/en/docs/programmable/683039/25-1/scripting-support-08828.html),
Quartus's programming executable. No shell command interpolation is used.

## Validation

```sh
cargo test
cargo clippy --all-targets -- -D warnings
sh -n scripts/install.sh
sh -n scripts/uninstall.sh
python3 tests/install.py
# Requires Quartus; generates real projects without programming hardware:
cargo test -- --ignored
```

Tests cover scaffolding, directory discovery, source changes, saved target and
cable settings, programming arguments, compilation/programmer failure handling,
and refusing missing programming output. Hardware programming is tested
with simulated executables. Real Quartus tests cover synchronization and custom
assignments. The generic scaffold needs board-specific pin and timing constraints
before it is ready for hardware use.
