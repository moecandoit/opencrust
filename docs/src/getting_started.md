# Getting Started

## Quick Start

The fastest way to get started is using the install script:

```bash
# Install (Linux, macOS)
curl -fsSL https://raw.githubusercontent.com/opencrust-org/opencrust/main/install.sh | sh

# Interactive setup — pick your LLM provider, store API keys in encrypted vault
opencrust init

# Start
opencrust start
```

## Build from Source

You can also build from source if you have Rust installed (1.85+).

```bash
cargo build --release
./target/release/opencrust init
./target/release/opencrust start
```

## Configuration

OpenCrust looks for its configuration file at `~/.opencrust/config.yml`.

Example configuration:

```yaml
gateway:
  host: "127.0.0.1"
  port: 3888

llm:
  claude:
    provider: anthropic
    model: claude-sonnet-4-5-20250929
    # api_key resolved from: vault > config > ANTHROPIC_API_KEY env var

  ollama-local:
    provider: ollama
    model: llama3.1
    base_url: "http://localhost:11434"

channels:
  telegram:
    type: telegram
    enabled: true
    bot_token: "your-bot-token"  # or TELEGRAM_BOT_TOKEN env var

agent:
  system_prompt: "You are a helpful assistant."
  max_tokens: 4096
  max_context_tokens: 100000

memory:
  enabled: true

# MCP servers for external tools
mcp:
  filesystem:
    command: npx
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
```

## Migrating from OpenClaw

If you are migrating from OpenClaw, you can use the migration tool to import your skills, channel configs, and credentials.

```bash
opencrust migrate openclaw
```

Use `--dry-run` to preview changes before committing. Use `--source /path/to/openclaw` to specify a custom OpenClaw config directory.
