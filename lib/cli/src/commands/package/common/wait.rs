use futures_util::StreamExt;
use wasmer_api::WasmerClient;

/// Different conditions that can be "awaited" when publishing a package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum PublishWait {
    None,
    Container,
    NativeExecutables,
    Bindings,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitPackageState {
    pub container: bool,
    pub native_executables: bool,
    pub bindings: bool,
}

impl WaitPackageState {
    pub fn is_any(self) -> bool {
        self.container || self.native_executables || self.bindings
    }

    pub fn new_none() -> Self {
        Self {
            container: false,
            native_executables: false,
            bindings: false,
        }
    }

    pub fn new_all() -> Self {
        Self {
            container: true,
            native_executables: true,
            bindings: true,
        }
    }

    pub fn new_container() -> Self {
        Self {
            container: true,
            native_executables: false,
            bindings: false,
        }
    }

    pub fn new_exe() -> Self {
        Self {
            container: true,
            native_executables: true,
            bindings: false,
        }
    }

    pub fn new_bindings() -> Self {
        Self {
            container: true,
            native_executables: false,
            bindings: true,
        }
    }
}

impl From<PublishWait> for WaitPackageState {
    fn from(value: PublishWait) -> Self {
        match value {
            PublishWait::None => Self::new_none(),
            PublishWait::Container => Self::new_container(),
            PublishWait::NativeExecutables => Self::new_exe(),
            PublishWait::Bindings => Self::new_bindings(),
            PublishWait::All => Self::new_all(),
        }
    }
}

pub async fn wait_package(
    client: &WasmerClient,
    to_wait: PublishWait,
    package_version_id: wasmer_api::types::Id,
    timeout: humantime::Duration,
) -> anyhow::Result<()> {
    if let PublishWait::None = to_wait {
        return Ok(());
    }

    let registry_url = client.graphql_endpoint().to_string();
    let login_token = client.auth_token().unwrap_or_default().to_string();
    let package_version_id = package_version_id.into_inner();

    let (mut stream, _) = wasmer_registry::subscriptions::subscribe_package_version_ready(
        &registry_url,
        &login_token,
        &package_version_id,
    )
    .await?;

    let mut state: WaitPackageState = to_wait.into();

    let deadline: std::time::Instant =
        std::time::Instant::now() + std::time::Duration::from_secs(timeout.as_secs());

    loop {
        if !state.is_any() {
            break;
        }

        if std::time::Instant::now() > deadline {
            return Err(anyhow::anyhow!(
                "Timed out waiting for package version to become ready"
            ));
        }

        let data = match tokio::time::timeout_at(deadline.into(), stream.next()).await {
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "Timed out waiting for package version to become ready"
                ));
            }
            Ok(None) => {
                break;
            }
            Ok(Some(data)) => data,
        };

        if let Some(data) = data.unwrap().data {
            match data.package_version_ready.state {
                wasmer_registry::subscriptions::PackageVersionState::WEBC_GENERATED => {
                    state.container = false
                }
                wasmer_registry::subscriptions::PackageVersionState::BINDINGS_GENERATED => {
                    state.bindings = false
                }
                wasmer_registry::subscriptions::PackageVersionState::NATIVE_EXES_GENERATED => {
                    state.native_executables = false
                }
                wasmer_registry::subscriptions::PackageVersionState::Other(_) => {}
            }
        }
    }

    Ok(())
}
