//! Telegram channel plugin.
//!
//! Factory for creating the Telegram channel. Implements
//! [`ChannelPlugin`](oxios_gateway::plugin::ChannelPlugin) so the
//! main binary can activate the Telegram channel from configuration.

use anyhow::Result;
use async_trait::async_trait;
use oxios_gateway::plugin::{ChannelBundle, ChannelContext, ChannelPlugin};

use crate::channels::telegram::{TelegramChannel, TelegramSessionSettings, TokenValidation};

/// Telegram channel plugin — creates a Telegram Bot channel.
#[derive(Default)]
pub struct TelegramPlugin;

/// Store key for the bot token in the credential store. Mirrors the Web UI
/// Secrets section and `/api/secrets`; canonical constant lives in
/// `oxios_kernel::credential::TELEGRAM_TOKEN_STORE_KEY`.
pub(crate) use oxios_kernel::credential::TELEGRAM_TOKEN_STORE_KEY as TOKEN_STORE_KEY;

impl TelegramPlugin {
    /// Create a new Telegram plugin instance.
    pub fn new() -> Self {
        Self
    }
}

/// Resolve the bot token: env var named by `bot_token_env` first, then the
/// credential stores (`~/.oxios` via `OXICODE_HOME`, then shared
/// `~/.oxicode/auth.json`). The same resolution backs the Web UI's
/// Secrets status display, so what the UI shows is what the plugin uses.
pub(crate) fn resolve_bot_token(bot_token_env: &str) -> Result<String> {
    oxios_kernel::credential::CredentialStore::resolve_secret(TOKEN_STORE_KEY, bot_token_env)
        .map(|(token, _source)| token)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Telegram bot token not found. Store it in the Web UI \
                 (Settings → Secrets → Telegram Bot Token) or set the \
                 {bot_token_env} environment variable."
            )
        })
}

#[async_trait]
impl ChannelPlugin for TelegramPlugin {
    fn name(&self) -> &str {
        "telegram"
    }

    async fn setup(&self, ctx: ChannelContext) -> Result<ChannelBundle> {
        let config = ctx.config.read().clone();
        let telegram = &config.channels.telegram;

        let token = resolve_bot_token(&telegram.bot_token_env)?;
        let allowed = telegram.allowed_users.clone();
        let api_base = telegram.api_base.clone();

        let rotation_hours = telegram.session.rotation_hours;
        let max_messages = telegram.session.max_messages;
        let channel = TelegramChannel::new(token, allowed)
            .with_api_base(api_base)
            .with_session_settings(TelegramSessionSettings {
                rotation_hours,
                max_messages_per_session: max_messages,
            });

        // Fail fast on a definitively rejected token (401/404) so the
        // connect button and boot logs surface the real problem. Transient
        // network failures do NOT block setup — the polling loop retries.
        let channel = match channel.validate_token().await {
            TokenValidation::Valid(username) => {
                tracing::info!(bot_username = %username, "Telegram bot token validated");
                channel.with_bot_username(Some(username))
            }
            TokenValidation::Unreachable => {
                tracing::warn!("Telegram getMe unreachable; proceeding — polling will retry");
                channel
            }
            TokenValidation::Rejected(reason) => {
                return Err(anyhow::anyhow!("Telegram rejected the bot token: {reason}"));
            }
        };

        tracing::info!(
            rotation_hours = rotation_hours,
            max_messages = max_messages,
            "Telegram channel created with session management"
        );

        Ok(ChannelBundle {
            channel: Box::new(channel),
            tasks: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::telegram::fake_server::{self, FakeResponse};
    use oxios_kernel::OxiosConfig;

    /// Unique env-var name so parallel tests never collide.
    const TEST_ENV: &str = "OXIOS_TEST_TELEGRAM_TOKEN_X7Q";

    fn ctx_with_api_base(base: String) -> ChannelContext {
        let mut config = OxiosConfig::default();
        config.channels.telegram.bot_token_env = TEST_ENV.to_string();
        config.channels.telegram.api_base = base;
        ChannelContext {
            config: std::sync::Arc::new(parking_lot::RwLock::new(config)),
            config_path: std::path::PathBuf::new(),
        }
    }

    #[tokio::test]
    async fn setup_succeeds_with_valid_token_from_env() {
        unsafe { std::env::set_var(TEST_ENV, "tok-1") };
        let server = fake_server::spawn(FakeResponse::Ok {
            username: "oxios_bot".into(),
        })
        .await;
        let bundle = TelegramPlugin::new()
            .setup(ctx_with_api_base(server.base_url))
            .await;
        unsafe { std::env::remove_var(TEST_ENV) };
        let bundle = bundle.expect("setup with valid env token + reachable API");
        assert_eq!(bundle.channel.name(), "telegram");
        assert!(bundle.tasks.is_empty());
    }

    #[tokio::test]
    async fn setup_fails_fast_on_rejected_token() {
        unsafe { std::env::set_var(TEST_ENV, "tok-bad") };
        let server = fake_server::spawn(FakeResponse::Unauthorized).await;
        let err = TelegramPlugin::new()
            .setup(ctx_with_api_base(server.base_url))
            .await
            .err()
            .expect("setup must fail on 401");
        unsafe { std::env::remove_var(TEST_ENV) };
        let msg = err.to_string();
        assert!(msg.contains("rejected"), "unexpected error: {msg}");
        assert!(msg.contains("Unauthorized"), "unexpected error: {msg}");
    }

    #[tokio::test]
    async fn setup_proceeds_when_unreachable() {
        // Bind then drop a listener so the port is closed → getMe fails to
        // connect → Unreachable → setup must still succeed (boot resilience).
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        unsafe { std::env::set_var(TEST_ENV, "tok-2") };
        let bundle = TelegramPlugin::new()
            .setup(ctx_with_api_base(base))
            .await
            .expect("setup must proceed when Telegram is unreachable");
        unsafe { std::env::remove_var(TEST_ENV) };
        assert_eq!(bundle.channel.name(), "telegram");
        assert!(bundle.channel.status().is_null());
    }

    #[test]
    fn resolve_bot_token_missing_message_names_both_fixes() {
        // Note: if the machine's shared store (~/.oxicode/auth.json) happens
        // to contain `telegram_bot_token`, resolution succeeds and this test
        // would be machine-dependent. Use an env name that never exists and
        // skip the assertion in that (unlikely) case.
        let resolved = oxios_kernel::credential::CredentialStore::resolve_secret(
            TOKEN_STORE_KEY,
            "OXIOS_NO_SUCH_ENV_VAR_QQ",
        );
        if resolved.is_none() {
            let err = resolve_bot_token("OXIOS_NO_SUCH_ENV_VAR_QQ")
                .err()
                .expect("must error when no token anywhere")
                .to_string();
            assert!(
                err.contains("Web UI"),
                "error should mention the Web UI: {err}"
            );
            assert!(
                err.contains("OXIOS_NO_SUCH_ENV_VAR_QQ"),
                "error should name the env var: {err}"
            );
        }
    }

    #[test]
    fn resolve_bot_token_env_wins() {
        unsafe { std::env::set_var(TEST_ENV, "env-token") };
        let token = resolve_bot_token(TEST_ENV).expect("env token must resolve");
        unsafe { std::env::remove_var(TEST_ENV) };
        assert_eq!(token, "env-token");
    }
}
