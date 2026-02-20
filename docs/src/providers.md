# Providers

OpenCrust integrates with various Large Language Models (LLMs).

## Supported Providers

- **Anthropic Claude**: Streaming (SSE), tool use.
- **OpenAI**: GPT-4o, Azure, or any OpenAI-compatible endpoint.
- **Ollama**: Local models with streaming.
- **Sansa**: Regional LLM.

## Configuration

Providers are configured in `~/.opencrust/config.yml`. API keys are securely stored in the credential vault.

```yaml
llm:
  claude:
    provider: anthropic
    model: claude-sonnet-4-5-20250929
    # api_key resolved from: vault > config > ANTHROPIC_API_KEY env var
```
