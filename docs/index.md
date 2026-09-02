# ClotoCore

A platform for building AI agents that run on your own machine.

ClotoCore is a Rust kernel plus a desktop dashboard. Everything an agent can
actually *do* — reason, remember, see, speak, reach the network — arrives as a
sandboxed plugin the kernel loads and mediates. The kernel itself stays
deliberately small: it routes events, enforces permissions, and hands plugins
the capabilities they are allowed to have, rather than being the intelligence.

## What you compose

An agent is a set of plugins, a personality definition, and a capability set.
The same kernel runs a research assistant and a streaming VTuber character;
what differs is which plugins are loaded and what they are permitted to reach.

Plugins speak [MCP](https://modelcontextprotocol.io/) — so a plugin can be
written in any language that can speak the protocol — and first-party servers
additionally speak MGP, the event protocol that lets a plugin react to what
other plugins do rather than only answer calls. The protocol itself lives in
[its own repository](https://github.com/Cloto-dev/mgp-spec).

## Where to go next

| If you want to | Read |
| --- | --- |
| Understand how the kernel is put together | [Architecture](ARCHITECTURE.md) |
| Write a plugin | [Build an MCP/MGP server](QUICKSTART_MCP_SERVER.md) |
| Work on ClotoCore itself | [Development](DEVELOPMENT.md) |
| See what changed | [Changelog](CHANGELOG.md) |
| Know where this is going | [Project vision](PROJECT_VISION.md) |

## Running it

Pre-built installers are published on
[GitHub Releases](https://github.com/Cloto-dev/ClotoCore/releases). They are
still marked experimental: the setup wizard downloads plugin servers and
configures Python on first launch, and it has been exercised on far fewer
machine configurations than the build-from-source path.

To build from source you need Rust and Node. The dashboard is built first
because the kernel embeds it:

```bash
git clone https://github.com/Cloto-dev/ClotoCore.git
cd ClotoCore
npm --prefix dashboard ci && npm --prefix dashboard run build
cargo build --release
cargo run --package cloto_core
```

The dashboard is then served at `http://localhost:8081`. For the Tauri desktop
shell instead, run `npx tauri dev` from `dashboard/`.

Configuration is environment variables with defaults for all of them; copy
`.env.example` to `.env` to change any. [Development](DEVELOPMENT.md) covers the
ones that matter while working on the code.

## Licence

Business Source License 1.1, converting to MIT in 2028.
