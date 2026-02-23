# Architecture

OpenCrust is a Rust-based AI agent framework designed for performance and security.

## Structure

```
crates/
  opencrust-cli/        # CLI, init wizard, daemon management
  opencrust-gateway/    # WebSocket gateway, HTTP API, sessions
  opencrust-config/     # YAML/TOML loading, hot-reload, MCP config
  opencrust-channels/   # Discord, Telegram, Slack, WhatsApp, iMessage
  opencrust-agents/     # LLM providers, tools, MCP client, agent runtime
  opencrust-db/         # SQLite memory, vector search (sqlite-vec)
  opencrust-plugins/    # WASM plugin sandbox (wasmtime)
  opencrust-media/      # Media processing
  opencrust-security/   # Credential vault, allowlists, pairing, validation
  opencrust-skills/     # SKILL.md parser, scanner, installer
  opencrust-common/     # Shared types, errors, utilities
```

## Tools

The agent runtime includes 6 built-in tools that the LLM can invoke during a conversation. The tool loop runs for up to 10 iterations per message.

| Tool | Description |
|------|-------------|
| `bash` | Execute shell commands (30s timeout, 32 KB max output) |
| `file_read` | Read file contents (1 MB max, path traversal prevention) |
| `file_write` | Write file contents (1 MB max, path traversal prevention) |
| `web_fetch` | Fetch web pages (30s timeout, 1 MB max response) |
| `web_search` | Search via Brave Search API (requires `BRAVE_API_KEY`) |
| `schedule_heartbeat` | Schedule future agent wake-ups (30-day max, 5 pending limit) |

See [Tools](./tools.md) for the full reference.

## Personality (DNA)

The agent's personality and identity is stored in `~/.opencrust/dna.md`. On first message with no DNA file, a bootstrap instruction is injected into the system prompt that directs the agent to ask the user for their preferences and write the file using the `file_write` tool.

Once created, DNA content is prepended to the system prompt on every message. The file is watched via the `notify` crate with 500ms debounce for hot-reload - edits take effect immediately.

System prompt layering order: `dna_content` + `system_prompt` (from config) + `memory_context` + `session_summary`.

## MCP (Model Context Protocol)

OpenCrust can connect to external MCP servers to extend the agent's capabilities. MCP tools are discovered at startup and appear as native agent tools with namespaced names (`server.tool_name`).

Configuration lives in `config.yml` under the `mcp:` section or in `~/.opencrust/mcp.json` (Claude Desktop compatible format). Both sources are merged at startup.

The `opencrust-agents` crate contains the MCP client (using the `rmcp` crate) with a tool bridge that converts MCP tool definitions into the internal tool format.

See [MCP](./mcp.md) for the full reference.

## Architectural Decision Records

See [Decision Records](./adr/README.md).
