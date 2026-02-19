use std::io::IsTerminal;
use std::path::Path;

use anyhow::{Context, Result};
use dialoguer::{Input, Password, Select};
use opencrust_config::{AgentConfig, AppConfig, GatewayConfig, LlmProviderConfig};
use tracing::info;

/// Run the interactive onboarding wizard. Writes config.yml and optionally
/// stores the API key in the credential vault.
pub fn run_wizard(config_dir: &Path) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        println!("Non-interactive environment detected.");
        println!(
            "To configure OpenCrust, edit: {}/config.yml",
            config_dir.display()
        );
        println!();
        println!("Minimal config.yml example:");
        println!("---");
        println!("llm:");
        println!("  main:");
        println!("    provider: anthropic");
        println!("    api_key: sk-ant-...");
        println!("agent:");
        println!("  system_prompt: \"You are a helpful assistant.\"");
        return Ok(());
    }

    println!();
    println!("  OpenCrust Setup Wizard");
    println!("  ----------------------");
    println!();

    // --- Provider selection ---
    let providers = &["anthropic", "openai", "sansa"];
    let selection = Select::new()
        .with_prompt("Select your LLM provider")
        .items(providers)
        .default(0)
        .interact()
        .context("provider selection cancelled")?;
    let provider = providers[selection];

    // --- API key ---
    let env_hint = match provider {
        "anthropic" => "ANTHROPIC_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "sansa" => "SANSA_API_KEY",
        _ => "API_KEY",
    };

    let api_key: String = Password::new()
        .with_prompt(format!(
            "Enter your {provider} API key (or set {env_hint} env var later)"
        ))
        .allow_empty_password(true)
        .interact()
        .context("API key input cancelled")?;

    let api_key = api_key.trim().to_string();

    // --- Vault storage ---
    let store_in_vault = if !api_key.is_empty() {
        let choices = &[
            "Store in encrypted vault (recommended)",
            "Store as plaintext in config.yml",
            "Skip storing (use env var)",
        ];
        Select::new()
            .with_prompt("How should the API key be stored?")
            .items(choices)
            .default(0)
            .interact()
            .context("storage choice cancelled")?
    } else {
        2 // skip
    };

    // --- System prompt ---
    let system_prompt: String = Input::new()
        .with_prompt("System prompt (optional)")
        .default("You are a helpful personal AI assistant.".to_string())
        .allow_empty(true)
        .interact_text()
        .context("system prompt input cancelled")?;

    // --- Build config ---
    let mut llm_config = LlmProviderConfig {
        provider: provider.to_string(),
        model: None,
        api_key: None,
        base_url: None,
        extra: Default::default(),
    };

    match store_in_vault {
        0 => {
            // Encrypted vault
            let vault_path = config_dir.join("credentials").join("vault.json");
            let passphrase: String = Password::new()
                .with_prompt("Set a vault passphrase")
                .with_confirmation("Confirm passphrase", "Passphrases don't match")
                .interact()
                .context("passphrase input cancelled")?;

            match opencrust_security::CredentialVault::create(&vault_path, &passphrase) {
                Ok(mut vault) => {
                    vault.set(env_hint, &api_key);
                    vault.save().context("failed to save vault")?;
                    println!("  API key encrypted in vault.");
                    println!("  Set OPENCRUST_VAULT_PASSPHRASE env var for server mode.");
                }
                Err(e) => {
                    println!("  Warning: vault creation failed ({e}), storing in config instead.");
                    llm_config.api_key = Some(api_key.clone());
                }
            }
        }
        1 => {
            // Plaintext in config
            llm_config.api_key = Some(api_key.clone());
        }
        _ => {
            // Skip — user will use env var
            println!("  Set {env_hint} environment variable before starting the server.");
        }
    }

    let config = AppConfig {
        gateway: GatewayConfig::default(),
        llm: [("main".to_string(), llm_config)].into_iter().collect(),
        agent: AgentConfig {
            system_prompt: if system_prompt.is_empty() {
                None
            } else {
                Some(system_prompt)
            },
            ..Default::default()
        },
        ..Default::default()
    };

    let config_path = config_dir.join("config.yml");
    let yaml = serde_yaml::to_string(&config).context("failed to serialize config")?;
    std::fs::write(&config_path, &yaml)
        .context(format!("failed to write {}", config_path.display()))?;

    info!("config written to {}", config_path.display());
    println!();
    println!("  Config written to {}", config_path.display());
    println!("  Run `opencrust start` to launch the gateway.");
    println!();

    Ok(())
}
