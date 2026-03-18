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

The IPC binary protocol uses a simple format: 4-byte LE opcode + 4-byte LE length + UTF-8 JSON payload. The switch handles handshake, frame, close, ping, and pong opcodes.

## Requirements

- Rust (edition 2024)
- One or more running Discord instances

## Building

```sh
cargo build --release
```

## Usage

1. Close Discord or ensure `discord-ipc-0` is not taken
2. Run presence-switch:
   ```sh
   cargo run --release
   ```
3. Start your Discord instances — they will claim `discord-ipc-1`, `discord-ipc-2`, etc.
4. Launch your RPC-enabled application — it connects to presence-switch on `discord-ipc-0`, which relays to all Discord instances

For best results, start presence-switch before any Discord instances so it can claim `discord-ipc-0`, which is what most RPC clients connect to by default.

Press `Ctrl+C` to shut down gracefully.

## Platform support

| Platform | IPC mechanism       |
|----------|---------------------|
| Linux    | Unix domain sockets |
| macOS    | Unix domain sockets |
| Windows  | Named pipes         |

Platform-specific implementations are selected at compile time via `#[cfg]`.

## Project structure

```
src/
├── main.rs
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
