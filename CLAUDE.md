# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

- `cargo build` — build the project
- `cargo run` — build and run
- `cargo check` — type-check without producing a binary
- `cargo clippy` — lint

There are no tests in this project currently.

## What This Project Does

presence-switch is a Discord Rich Presence IPC proxy. It sits between Discord RPC client applications and real Discord instances, acting as a multiplexing switch. Client apps connect to the switch (which binds `discord-ipc-0`), and the switch relays their RPC messages to all real Discord IPC servers (`discord-ipc-1` through `discord-ipc-9`).

## Architecture

Two main modules under `src/`:

- **`switch/`** — The IPC server that listens for incoming RPC client connections. Binds to available Discord IPC socket names. Each connected client gets a `Client` struct that processes the Discord IPC binary protocol (handshake, frames, ping/pong, close) and relays frames to Discord via broadcast channels.

- **`discord/`** — The IPC client that connects to real Discord instances. `ipc/` manages connections to Discord's IPC servers and relays messages bidirectionally using broadcast channels. `api.rs` fetches application metadata from Discord's REST API with response caching.

Both modules have platform-specific IPC implementations via `unix.rs` (Unix domain sockets) and `windows.rs` (named pipes), selected at compile time with `#[cfg]`.

## IPC Binary Protocol

All IPC messages use: 4-byte LE u32 opcode + 4-byte LE u32 length + UTF-8 JSON payload. Opcodes: 0=Handshake, 1=Frame, 2=Close, 3=Ping, 4=Pong.

## Key Patterns

- Fully async with Tokio; tasks communicate via `mpsc` and `broadcast` channels
- Graceful shutdown via `CancellationToken` propagation from a Ctrl+C signal handler
- Platform abstraction through conditional compilation, not trait objects
