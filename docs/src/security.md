# Security

Security is a core principle of OpenCrust.

## Features

- **Encrypted Credential Vault**: AES-256-GCM encryption for API keys.
- **Authentication**: WebSocket gateway requires pairing codes.
- **Allowlists**: Control who can interact with the agent per channel.
- **Prompt Injection Detection**: Input validation and sanitization.
- **WASM Sandboxing**: Plugins run in a restricted environment.

## Documentation

- [Architecture](./security/architecture.md)
- [Attack Surfaces](./security/attack-surfaces.md)
- [Checklist](./security/checklist.md)
