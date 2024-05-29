//! Create a new Edge app.

use crate::{
    commands::AsyncCliCommand,
    opts::{ApiOpts, ItemFormatOpts, WasmerEnv},
    utils::{load_package_manifest, prompts::PackageCheckMode},
};
use anyhow::Context;
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Confirm, Select};
use is_terminal::IsTerminal;
use std::{collections::HashMap, env, io::Cursor, path::PathBuf, str::FromStr};
use wasmer_api::{types::AppTemplate, WasmerClient};
use wasmer_config::{app::AppConfigV1, package::PackageSource};

use super::{deploy::CmdAppDeploy, util::login_user};

async fn write_app_config(app_config: &AppConfigV1, dir: Option<PathBuf>) -> anyhow::Result<()> {
    let raw_app_config = app_config.clone().to_yaml()?;

    let app_dir = match dir {
        Some(dir) => dir,
        None => std::env::current_dir()?,
    };

    let app_config_path = app_dir.join(AppConfigV1::CANONICAL_FILE_NAME);
    std::fs::write(&app_config_path, raw_app_config).with_context(|| {
        format!(
            "could not write app config to '{}'",
            app_config_path.display()
        )
    })
}

/// Create a new Edge app.
#[derive(clap::Parser, Debug)]
pub struct CmdAppCreate {
    /// A reference to the template to use.
    ///
    /// It can be either an URL to a github repository - like
    /// `https://github.com/wasmer-examples/php-wasmer-starter` -  or the name of a template that
    /// will be searched for in the selected registry, like `astro-starter`.
    #[clap(
        long,
        conflicts_with = "package",
        conflicts_with = "use_local_manifest"
    )]
    pub template: Option<String>,

    /// Name of the package to use.
    #[clap(
        long,
        conflicts_with = "template",
        conflicts_with = "use_local_manifest"
    )]
    pub package: Option<String>,

    /// Whether or not to search (and use) a local manifest.
    #[clap(long, conflicts_with = "template", conflicts_with = "package")]
    pub use_local_manifest: bool,

    /// Whether or not to deploy the application once it is created.
    ///
    /// If selected, this might entail the step of publishing the package related to the
    /// application. By default, the application is not deployed and the package is not published.
    #[clap(long = "deploy")]
    pub deploy_app: bool,

    /// Skip local schema validation.
    #[clap(long)]
    pub no_validate: bool,

    /// Do not prompt for user input.
    #[clap(long, default_value_t = !std::io::stdin().is_terminal())]
    pub non_interactive: bool,

    /// Do not interact with any APIs.
    #[clap(long)]
    pub offline: bool,

    /// The owner of the app.
    #[clap(long)]
    pub owner: Option<String>,

    /// The name of the app (can be changed later)
    #[clap(long = "name")]
    pub app_name: Option<String>,

    /// The path to the directory where the config file for the application will be written to.
    #[clap(long = "dir")]
    pub app_dir_path: Option<PathBuf>,

    /// Do not wait for the app to become reachable if deployed.
    #[clap(long)]
    pub no_wait: bool,

    // Common args.
    #[clap(flatten)]
    #[allow(missing_docs)]
    pub api: ApiOpts,

    #[clap(flatten)]
    pub env: WasmerEnv,

    #[clap(flatten)]
    #[allow(missing_docs)]
    pub fmt: ItemFormatOpts,

    /// Name to use when creating a new package from a template.
    #[clap(long)]
    pub new_package_name: Option<String>,

    /// Don't print any message.
    #[clap(long)]
    pub quiet: bool,
}

impl CmdAppCreate {
    #[inline]
    fn get_app_config(&self, owner: &str, name: &str, package: &str) -> AppConfigV1 {
        AppConfigV1 {
            name: String::from(name),
            owner: Some(String::from(owner)),
            package: PackageSource::from_str(package).unwrap(),
            app_id: None,
            domains: None,
            env: HashMap::new(),
            cli_args: None,
            capabilities: None,
            scheduled_tasks: None,
            volumes: None,
            health_checks: None,
            debug: None,
            scaling: None,
            extra: HashMap::new(),
        }
    }

    async fn get_app_name(&self) -> anyhow::Result<String> {
        if let Some(name) = &self.app_name {
            return Ok(name.clone());
        }

        if self.non_interactive {
            // if not interactive we can't prompt the user to choose the owner of the app.
            anyhow::bail!("No app name specified: use --name <app_name>");
        }

        let default_name = match &self.app_dir_path {
            Some(path) => path
                .file_name()
                .and_then(|f| f.to_str())
                .map(|s| s.to_owned()),
            None => env::current_dir().ok().and_then(|dir| {
                dir.file_name()
                    .and_then(|f| f.to_str())
                    .map(|s| s.to_owned())
            }),
        };

        crate::utils::prompts::prompt_for_ident(
            "What should be the name of the app?",
            default_name.as_deref(),
        )
    }

    async fn get_owner(&self, client: Option<&WasmerClient>) -> anyhow::Result<String> {
        if let Some(owner) = &self.owner {
            return Ok(owner.clone());
        }

        if self.non_interactive {
            // if not interactive we can't prompt the user to choose the owner of the app.
            anyhow::bail!("No owner specified: use --owner <owner>");
        }

        let user = if let Some(client) = client {
            Some(wasmer_api::query::current_user_with_namespaces(client, None).await?)
        } else {
            None
        };
        crate::utils::prompts::prompt_for_namespace("Who should own this app?", None, user.as_ref())
    }

    async fn create_from_local_manifest(
        &self,
        owner: &str,
        app_name: &str,
    ) -> anyhow::Result<bool> {
        if (!self.use_local_manifest && self.non_interactive)
            || self.template.is_some()
            || self.package.is_some()
        {
            return Ok(false);
        }

        let app_dir = match &self.app_dir_path {
            Some(dir) => PathBuf::from(dir),
            None => std::env::current_dir()?,
        };

        let (manifest_path, _) = if let Some(res) = load_package_manifest(&app_dir)? {
            res
        } else if self.use_local_manifest {
            anyhow::bail!("The --use_local_manifest flag was passed, but path {} does not contain a valid package manifest.", app_dir.display())
        } else {
            return Ok(false);
        };

        let ask_confirmation = || {
            eprintln!(
                "A package manifest was found in path {}.",
                &manifest_path.display()
            );
            let theme = dialoguer::theme::ColorfulTheme::default();
            Confirm::with_theme(&theme)
                .with_prompt("Use it for the app?")
                .interact()
        };

        if self.use_local_manifest || ask_confirmation()? {
            let app_config = self.get_app_config(owner, app_name, ".");
            write_app_config(&app_config, self.app_dir_path.clone()).await?;
            self.try_deploy(owner, app_name).await?;
            return Ok(true);
        }

        Ok(false)
    }

    async fn create_from_package(
        &self,
        client: Option<&WasmerClient>,
        owner: &str,
        app_name: &str,
    ) -> anyhow::Result<bool> {
        if self.template.is_some() {
            return Ok(false);
        }

        if let Some(pkg) = &self.package {
            let app_config = self.get_app_config(owner, app_name, pkg);
            write_app_config(&app_config, self.app_dir_path.clone()).await?;
            self.try_deploy(owner, app_name).await?;
            return Ok(true);
        } else if !self.non_interactive {
            let (package_id, _) = crate::utils::prompts::prompt_for_package(
                "Enter the name of the package",
                Some("wasmer/hello"),
                if client.is_some() {
                    Some(PackageCheckMode::MustExist)
                } else {
                    None
                },
                client,
            )
            .await?;

            let app_config = self.get_app_config(owner, app_name, &package_id.to_string());
            write_app_config(&app_config, self.app_dir_path.clone()).await?;
            self.try_deploy(owner, app_name).await?;
            return Ok(true);
        } else {
            eprintln!(
                "{}: the app creation process did not produce any local change.",
                "Warning".bold().yellow()
            );
        }

        Ok(false)
    }

    // A utility function used to fetch the URL of the template to use.
    async fn get_template_url(&self, client: &WasmerClient) -> anyhow::Result<url::Url> {
        let mut url = if let Some(template) = &self.template {
            if let Ok(url) = url::Url::parse(template) {
                url
            } else if let Some(template) =
                wasmer_api::query::fetch_app_template_from_slug(client, template.clone()).await?
            {
                url::Url::parse(&template.repo_url)?
            } else {
                anyhow::bail!("Template '{}' not found in the registry", template)
            }
        } else {
            if self.non_interactive {
                anyhow::bail!("No template selected")
            }

            let templates: Vec<AppTemplate> =
                wasmer_api::query::fetch_app_templates(client, String::new(), 10)
                    .await?
                    .ok_or(anyhow::anyhow!("No template received from the backend"))?
                    .edges
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.node)
                    .collect();

            let theme = ColorfulTheme::default();
            let items = templates
                .iter()
                .map(|t| {
                    format!(
                        "{}{}\n  {} {}",
                        t.name.bold(),
                        if t.language.is_empty() {
                            String::new()
                        } else {
                            format!(" {}", t.language.dimmed())
                        },
                        "demo:".bold().dimmed(),
                        t.demo_url.dimmed()
                    )
                })
                .collect::<Vec<_>>();

            let dialog = dialoguer::Select::with_theme(&theme)
                .with_prompt(format!("Select a template ({} available)", items.len()))
                .items(&items)
                .max_length(6)
                .clear(true)
                .report(false)
                .default(0);

            let selection = dialog.interact()?;

            let selected_template = templates
                .get(selection)
                .ok_or(anyhow::anyhow!("Invalid selection!"))?;

            if !self.quiet {
                eprintln!(
                    "{} {} {} {} ({} {})",
                    "✔".green().bold(),
                    "Selected template".bold(),
                    "·".dimmed(),
                    selected_template.name.green().bold(),
                    "demo url".dimmed().bold(),
                    selected_template.demo_url.dimmed()
                )
            }

            url::Url::parse(&selected_template.repo_url)?
        };

        let url = if url.path().contains("archive/refs/heads") || url.path().contains("/zipball/") {
            url
        } else {
            let old_path = url.path();
            url.set_path(&format!("{old_path}/zipball/main"));
            url
        };

        Ok(url)
    }

    async fn create_from_template(
        &self,
        client: Option<&WasmerClient>,
        owner: &str,
        app_name: &str,
    ) -> anyhow::Result<bool> {
        let client = match client {
            Some(client) => client,
            None => anyhow::bail!("Cannot"),
        };

        let url = self.get_template_url(client).await?;

        tracing::info!("Downloading template from url {url}");

        let output_path = if let Some(path) = &self.app_dir_path {
            path.clone()
        } else {
            PathBuf::from(".").canonicalize()?
        };

        if output_path.is_dir() && output_path.read_dir()?.next().is_some() {
            if !self.quiet {
                eprintln!("The current directory is not empty.");
                eprintln!("Use the `--dir` flag to specify another directory, or remove files from the currently selected one.")
            }
            anyhow::bail!("Stopping as the directory is not empty")
        }

        let pb = indicatif::ProgressBar::new_spinner();

        pb.enable_steady_tick(std::time::Duration::from_millis(500));
        pb.set_style(
            indicatif::ProgressStyle::with_template("{spinner:.magenta} {msg}")
                .unwrap()
                .tick_strings(&["✶", "✸", "✹", "✺", "✹", "✷"]),
        );

        pb.set_message("Downloading package..");

        let response = reqwest::get(url).await?;
        let bytes = response.bytes().await?;
        pb.set_message("Unpacking the template..");

        let cursor = Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(cursor)?;

        // Extract the files to the output path
        for entry in 0..archive.len() {
            let mut entry = archive
                .by_index(entry)
                .context(format!("Getting the archive entry #{entry}"))?;

            let path = entry.mangled_name();

            let path: PathBuf = {
                let mut components = path.components();
                components.next();
                components.collect()
            };

            if path.to_str().unwrap_or_default().contains(".github") {
                continue;
            }

            let path = output_path.join(path);

            if let Some(parent) = path.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent)?;
                }
            }

            if !path.exists() {
                // AsyncRead not implemented for entry..
                if entry.is_file() {
                    let mut outfile = std::fs::OpenOptions::new()
                        .create(true)
                        .truncate(true)
                        .write(true)
                        .open(&path)?;
                    std::io::copy(&mut entry, &mut outfile)?;
                } else {
                    std::fs::create_dir(path)?;
                }
            }
        }
        pb.set_style(
            indicatif::ProgressStyle::with_template(&format!("{} {{msg}}", "✔".green().bold()))
                .unwrap(),
        );
        pb.finish_with_message(format!("{}", "Unpacked template".bold()));

        pb.finish();

        let app_yaml_path = output_path.join(AppConfigV1::CANONICAL_FILE_NAME);

        if app_yaml_path.exists() && app_yaml_path.is_file() {
            let contents = tokio::fs::read_to_string(&app_yaml_path).await?;
            let mut raw_yaml: serde_yaml::Value = serde_yaml::from_str(&contents)?;

            if let serde_yaml::Value::Mapping(m) = &mut raw_yaml {
                m.insert("name".into(), app_name.into());
                m.insert("owner".into(), owner.into());
                m.shift_remove("domains");
                m.shift_remove("app_id");
            };

            let raw_app = serde_yaml::to_string(&raw_yaml)?;

            // Validate..
            AppConfigV1::parse_yaml(&raw_app)?;

            tokio::fs::write(&app_yaml_path, raw_app).await?;
        }

        let build_md_path = output_path.join("BUILD.md");
        if build_md_path.exists() {
            let contents = tokio::fs::read_to_string(build_md_path).await?;
            eprintln!(
                "{}: {} 
{}",
                "NOTE".bold(),
                "The selected template has a `BUILD.md` file.
This means there are likely additional build 
steps that you need to perform before deploying
the app:\n"
                    .bold(),
                contents
            );
            let bin_name = match std::env::args().nth(0) {
                Some(n) => n,
                None => String::from("wasmer"),
            };
            eprintln!(
                "After taking the necessary steps to build your application, re-run `{}`",
                format!("{bin_name} deploy").bold()
            )
        } else {
            self.try_deploy(owner, app_name).await?;
        }

        Ok(true)
    }

    async fn try_deploy(&self, owner: &str, app_name: &str) -> anyhow::Result<()> {
        let interactive = !self.non_interactive;
        let theme = dialoguer::theme::ColorfulTheme::default();

        if self.deploy_app
            || (interactive
                && Confirm::with_theme(&theme)
                    .with_prompt("Do you want to deploy the app now?")
                    .interact()?)
        {
            let cmd_deploy = CmdAppDeploy {
                quiet: false,
                api: self.api.clone(),
                env: self.env.clone(),
                fmt: ItemFormatOpts {
                    format: self.fmt.format,
                },
                no_validate: false,
                non_interactive: self.non_interactive,
                publish_package: true,
                dir: self.app_dir_path.clone(),
                no_wait: self.no_wait,
                no_default: false,
                no_persist_id: false,
                owner: Some(String::from(owner)),
                app_name: Some(app_name.into()),
                bump: false,
                template: None,
                package: None,
                use_local_manifest: self.use_local_manifest,
            };
            cmd_deploy.run_async().await?;
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl AsyncCliCommand for CmdAppCreate {
    type Output = ();

    async fn run_async(self) -> Result<Self::Output, anyhow::Error> {
        let client = if self.offline {
            None
        } else {
            Some(
                login_user(
                    &self.api,
                    &self.env,
                    !self.non_interactive,
                    "retrieve informations about the owner of the app",
                )
                .await?,
            )
        };

        // Get the future owner of the app.
        let owner = self.get_owner(client.as_ref()).await?;

        // Get the name of the app.
        let app_name = self.get_app_name().await?;

        if !self.create_from_local_manifest(&owner, &app_name).await? {
            if self.template.is_some() {
                self.create_from_template(client.as_ref(), &owner, &app_name)
                    .await?;
            } else if self.package.is_some() {
                self.create_from_package(client.as_ref(), &owner, &app_name)
                    .await?;
            } else if !self.non_interactive {
                if self.offline {
                    eprintln!("Creating app from a package name running in offline mode");
                    self.create_from_package(client.as_ref(), &owner, &app_name)
                        .await?;
                } else {
                    let theme = ColorfulTheme::default();
                    let choice = Select::with_theme(&theme)
                        .with_prompt("What would you like to deploy?")
                        .items(&["Start with a template", "Choose an existing package"])
                        .default(0)
                        .interact()?;
                    match choice {
                        0 => {
                            self.create_from_template(client.as_ref(), &owner, &app_name)
                                .await?
                        }
                        1 => {
                            self.create_from_package(client.as_ref(), &owner, &app_name)
                                .await?
                        }
                        x => panic!("unhandled selection {x}"),
                    };
                }
            } else {
                eprintln!("Warning: the creation process did not produce any result.");
            }
        }

        Ok(())
    }
}
