use anyhow::{bail, Context};
use colored::Colorize;
use dialoguer::Confirm;
use wasmer_api::{
    global_id::{GlobalId, NodeKind},
    types::DeployApp,
    WasmerClient,
};
use wasmer_config::app::AppConfigV1;

use crate::{
    commands::Login,
    opts::{ApiOpts, WasmerEnv},
};

/// App identifier.
///
/// Can be either a namespace/name a plain name or an app id.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum AppIdent {
    /// Backend app id like "da_xxysw34234"
    AppId(String),
    /// Backend app VERSION id like "dav_xxysw34234"
    AppVersionId(String),
    NamespacedName(String, String),
    Alias(String),
}

impl AppIdent {
    /// Resolve an app identifier through the API.
    pub async fn resolve(&self, client: &WasmerClient) -> Result<DeployApp, anyhow::Error> {
        match self {
            AppIdent::AppId(app_id) => wasmer_api::query::get_app_by_id(client, app_id.clone())
                .await
                .with_context(|| format!("Could not find app with id '{}'", app_id)),
            AppIdent::AppVersionId(id) => {
                let (app, _version) =
                    wasmer_api::query::get_app_version_by_id_with_app(client, id.clone())
                        .await
                        .with_context(|| format!("Could not query for app version id '{}'", id))?;
                Ok(app)
            }
            AppIdent::Alias(name) => wasmer_api::query::get_app_by_alias(client, name.clone())
                .await?
                .with_context(|| format!("Could not find app with name '{name}'")),
            AppIdent::NamespacedName(owner, name) => {
                wasmer_api::query::get_app(client, owner.clone(), name.clone())
                    .await?
                    .with_context(|| format!("Could not find app '{owner}/{name}'"))
            }
        }
    }
}

impl std::str::FromStr for AppIdent {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((namespace, name)) = s.split_once('/') {
            if namespace.is_empty() {
                bail!("invalid app identifier '{s}': namespace can not be empty");
            }
            if name.is_empty() {
                bail!("invalid app identifier '{s}': name can not be empty");
            }

            Ok(Self::NamespacedName(
                namespace.to_string(),
                name.to_string(),
            ))
        } else if let Ok(id) = GlobalId::parse_prefixed(s) {
            match id.kind() {
                NodeKind::DeployApp => Ok(Self::AppId(s.to_string())),
                NodeKind::DeployAppVersion => Ok(Self::AppVersionId(s.to_string())),
                _ => {
                    bail!(
                        "invalid app identifier '{s}': expected an app id, but id is of type {kind}",
                        kind = id.kind(),
                    );
                }
            }
        } else {
            Ok(Self::Alias(s.to_string()))
        }
    }
}

pub fn get_app_config_from_current_dir() -> Result<(AppConfigV1, std::path::PathBuf), anyhow::Error>
{
    // read the information from local `app.yaml
    let current_dir = std::env::current_dir()?;
    let app_config_path = current_dir.join(AppConfigV1::CANONICAL_FILE_NAME);

    if !app_config_path.exists() || !app_config_path.is_file() {
        bail!(
            "Could not find app.yaml at path: '{}'.\nPlease specify an app like 'wasmer app get <namespace>/<name>' or 'wasmer app get <name>`'",
            app_config_path.display()
        );
    }
    // read the app.yaml
    let raw_app_config = std::fs::read_to_string(&app_config_path)
        .with_context(|| format!("Could not read file '{}'", app_config_path.display()))?;

    // parse the app.yaml
    let config = AppConfigV1::parse_yaml(&raw_app_config)
        .map_err(|err| anyhow::anyhow!("Could not parse app.yaml: {err:?}"))?;

    Ok((config, app_config_path))
}

/// Options for identifying an app.
///
/// Provides convenience methods for resolving an app identifier or loading it
/// from a local app.yaml.
#[derive(clap::Parser, Debug)]
pub struct AppIdentOpts {
    /// Identifier of the application.
    ///
    /// NOTE: If not specified, the command will look for an app config file in
    /// the current directory.
    ///
    /// Valid input:
    /// - namespace/app-name
    /// - app-alias
    /// - App ID
    pub app_ident: Option<AppIdent>,
}

// Allowing because this is not performance-critical at all.
#[allow(clippy::large_enum_variant)]
pub enum ResolvedAppIdent {
    Ident(AppIdent),
    Config {
        ident: AppIdent,
        config: AppConfigV1,
        path: std::path::PathBuf,
    },
}

impl ResolvedAppIdent {
    pub fn ident(&self) -> &AppIdent {
        match self {
            Self::Ident(ident) => ident,
            Self::Config { ident, .. } => ident,
        }
    }
}

impl AppIdentOpts {
    pub fn resolve_static(&self) -> Result<ResolvedAppIdent, anyhow::Error> {
        if let Some(id) = &self.app_ident {
            return Ok(ResolvedAppIdent::Ident(id.clone()));
        }

        // Try to load from local.
        let (config, path) = get_app_config_from_current_dir()?;

        let ident = if let Some(id) = &config.app_id {
            AppIdent::AppId(id.clone())
        } else {
            AppIdent::Alias(config.name.clone())
        };

        Ok(ResolvedAppIdent::Config {
            ident,
            config,
            path,
        })
    }

    /// Load the specified app from the API.
    pub async fn load_app(
        &self,
        client: &WasmerClient,
    ) -> Result<(ResolvedAppIdent, DeployApp), anyhow::Error> {
        let id = self.resolve_static()?;
        let app = id.ident().resolve(client).await?;

        Ok((id, app))
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_app_ident() {
        assert_eq!(
            AppIdent::from_str("da_MRrWI0t5U582").unwrap(),
            AppIdent::AppId("da_MRrWI0t5U582".to_string()),
        );
        assert_eq!(
            AppIdent::from_str("lala").unwrap(),
            AppIdent::Alias("lala".to_string()),
        );

        assert_eq!(
            AppIdent::from_str("alpha/beta").unwrap(),
            AppIdent::NamespacedName("alpha".to_string(), "beta".to_string()),
        );
    }
}

pub(super) async fn login_user(
    api: &ApiOpts,
    env: &WasmerEnv,
    interactive: bool,
    msg: &str,
) -> anyhow::Result<WasmerClient> {
    if let Ok(client) = api.client() {
        return Ok(client);
    }

    let theme = dialoguer::theme::ColorfulTheme::default();

    if api.token.is_none() {
        if interactive {
            eprintln!(
                "{}: You need to be logged in to {msg}.",
                "WARN".yellow().bold()
            );

            if Confirm::with_theme(&theme)
                .with_prompt("Do you want to login now?")
                .interact()?
            {
                Login {
                    no_browser: false,
                    wasmer_dir: env.wasmer_dir.clone(),
                    registry: api
                        .registry
                        .clone()
                        .map(|l| wasmer_registry::wasmer_env::Registry::from(l.to_string())),
                    token: api.token.clone(),
                    cache_dir: Some(env.cache_dir.clone()),
                }
                .run_async()
                .await?;
                // self.api = ApiOpts::default();
            } else {
                anyhow::bail!("Stopping the flow as the user is not logged in.")
            }
        } else {
            let bin_name = match std::env::args().next() {
                Some(n) => n,
                None => String::from("wasmer"),
            };
            eprintln!("You are not logged in. Use the `--token` flag or log in (use `{bin_name} login`) to {msg}.");

            anyhow::bail!("Stopping execution as the user is not logged in.")
        }
    }

    api.client()
}
