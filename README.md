# presence-switch

A Discord Rich Presence IPC proxy that multiplexes RPC messages across multiple Discord instances.

## What it does

presence-switch sits between Discord RPC client applications (games, media players, etc.) and running Discord instances. It binds the first available `discord-ipc-{0..9}` socket (preferring `discord-ipc-0`) and relays all incoming RPC messages to every other existing Discord IPC socket.

This means a single RPC client can broadcast its presence to multiple Discord clients simultaneously.

## How it works

```mermaid
graph LR
    A["RPC Client<br/>(e.g. game)"] --> B["presence-switch<br/>(discord-ipc-0)"]
    B --> C["Discord #1"]
    B --> D["Discord #2"]
    B --> E["Discord #N"]
```

1. The switch claims an available `discord-ipc-*` socket name
2. RPC clients connect to the switch thinking it's Discord
3. The switch relays messages to all real Discord instances on other sockets

The IPC binary protocol uses a simple format: 4-byte LE opcode + 4-byte LE length + UTF-8 JSON payload. The switch processes handshake, ping, and close opcodes directly, and forwards all other opcodes (frame, pong) to Discord.

## Requirements

- Rust (edition 2024)
- One or more running Discord instances

## Building

```sh
cargo build --release
```

## Installing

Tagged releases publish `.rpm` and `.msi` builds to the [Releases](https://github.com/kramerc/presence-switch/releases) page. For unreleased changes, the [`Package`](.github/workflows/package.yml) workflow also produces dev artifacts on every push to `main` and every PR — download them from the workflow run's Artifacts section.

To build packages locally on Linux:

```sh
scripts/package.sh rpm   # → target/generate-rpm/presence-switch-*.rpm
scripts/package.sh msi   # → target/wix/presence-switch-*.msi (cross-compiled from Linux)
scripts/package.sh all
```

See `scripts/package.sh --help` for the toolchain requirements.

To build the MSI natively on Windows (MSVC toolchain, no cross-compile):

```powershell
pwsh scripts/package.ps1   # → target/wix/presence-switch-*.msi
```

It links with the WiX v3 toolset, downloading it to `%LOCALAPPDATA%` on first
run if neither `$env:WIX` nor `candle.exe` is found.

### Linux (any RPM-based distro with systemd)

```sh
sudo dnf install ./presence-switch-*.rpm   # Fedora, RHEL, CentOS, Rocky, Alma
sudo zypper install ./presence-switch-*.rpm   # openSUSE
systemctl --user daemon-reload
systemctl --user enable --now presence-switch
```

The package installs a per-user systemd unit at `/usr/lib/systemd/user/presence-switch.service`. View logs with `journalctl --user -u presence-switch`. CI builds the RPM against Ubuntu 24.04's glibc (2.39+), so the target distro needs glibc ≥ 2.39 — that covers Fedora 41+, RHEL 10+, recent openSUSE Tumbleweed, and similar.

### Windows

Double-click the `.msi` to install per-user (no admin prompt). The installer adds an entry under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` so presence-switch launches at every logon — inspect or disable it via *Task Manager → Startup apps*. Uninstall via *Settings → Apps & features*.

Release builds run with a system tray icon and no console window. Debug builds retain the console for development. The tray icon is embedded in the executable; no separate image file is needed at runtime.

## Usage

1. Close Discord or ensure `discord-ipc-0` is not taken
2. Run presence-switch:
   ```sh
   cargo run --release
   ```
3. Start your Discord instances — they will claim `discord-ipc-1`, `discord-ipc-2`, etc.
4. Launch your RPC-enabled application — it connects to presence-switch on `discord-ipc-0`, which relays to all Discord instances

For best results, start presence-switch before any Discord instances so it can claim `discord-ipc-0`, which is what most RPC clients connect to by default.

### Windows tray controls

Open the tray icon's menu (check the notification area's hidden icons if needed):

- **Open Log** opens the log file in Notepad.
- **Quit** stops the IPC server and exits the application.

### Linux service controls

Linux runs headlessly, without a tray icon or graphical-session requirement in the application. Manage the installed user service with:

```sh
systemctl --user status presence-switch
systemctl --user stop presence-switch
systemctl --user restart presence-switch
journalctl --user -u presence-switch -f
```

When running in a terminal, press `Ctrl+C` to request shutdown.

### Logs and troubleshooting

On Windows and macOS, logs are appended to `presence-switch.log` in the system temporary directory (normally `%TEMP%\presence-switch.log` on Windows). **Open Log** opens this file; on macOS it uses the default viewer. If startup fails before the tray appears, open the file manually to check for initialization errors. Failures opening the log itself cannot be recorded there and currently have no fallback dialog.

Desktop logging currently includes TRACE-level RPC payloads and has no rotation or retention limit. The logging policy is tracked in [#29](https://github.com/kramerc/presence-switch/issues/29). Linux writes to stdout, which the systemd user service captures in the journal.

## Platform support

| Platform | IPC mechanism | Application interface |
|----------|---------------|-----------------------|
| Linux | Unix domain sockets | Headless process / systemd user service |
| Windows | Named pipes | System tray with Open Log and Quit |
| macOS | Unix domain sockets | Menu-bar tray implementation; native validation pending |

Platform-specific implementations are selected at compile time via `#[cfg]`. Tray code and its `tray-icon`, `winit`, and `image` dependencies are enabled only on Windows and macOS.

Windows compilation, strict Clippy checks, and the 17-test suite have passed locally, including server failure and shutdown tests. Interactive tray and MSI login-startup smoke testing remain outstanding. macOS build and runtime validation are tracked in [#30](https://github.com/kramerc/presence-switch/issues/30); macOS packaging is tracked separately in [#17](https://github.com/kramerc/presence-switch/issues/17).

## Project structure

```
src/
├── main.rs         # Selects the desktop or headless entry point
├── gui.rs          # Windows/macOS tray, file logging, and shutdown coordination
├── switch/         # IPC server — accepts RPC client connections
│   └── ipc/
│       ├── mod.rs      # Server and Client logic
│       ├── unix.rs     # Unix domain socket listener
│       └── windows.rs  # Named pipe listener
└── discord/        # IPC client — connects to real Discord instances
    ├── api.rs          # Discord REST API for app metadata (cached)
    └── ipc/
        ├── mod.rs      # Client, protocol types, socket discovery
        ├── unix.rs     # Unix domain socket connection
        └── windows.rs  # Named pipe connection
```

## Releasing

To cut a new release:

1. Bump `version` in `Cargo.toml` (and run `cargo update -w` so `Cargo.lock` matches).
2. Commit the bump and merge to `main`.
3. Tag the commit on `main` matching the new version, e.g.:
   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```
4. The [`Package`](.github/workflows/package.yml) workflow runs on the tag, validates that the tag matches `Cargo.toml`, builds the `.rpm` and `.msi`, and publishes a GitHub Release with both attached and auto-generated notes from the commits since the previous release.

If the tag version doesn't match `Cargo.toml`'s `version` field, both build jobs fail loudly before doing any work.

## License

[MIT](LICENSE) © Kramer Campbell
