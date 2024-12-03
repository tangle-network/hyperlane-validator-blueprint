#[cfg(test)]
mod e2e;

use api::services::events::JobCalled;
use color_eyre::Result;
use gadget_sdk as sdk;
use sdk::config::StdGadgetConfiguration;
use sdk::contexts::{ServicesContext, TangleClientContext};
use sdk::docker::bollard::network::ConnectNetworkOptions;
use sdk::docker::bollard::Docker;
use sdk::docker::connect_to_docker;
use sdk::docker::Container;
use sdk::event_listener::tangle::{
    jobs::{services_post_processor, services_pre_processor},
    TangleEventListener,
};
use sdk::keystore::BackendExt;
use sdk::tangle_subxt::tangle_testnet_runtime::api;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use color_eyre::eyre::eyre;
use tokio::sync::Mutex;

#[derive(Clone, TangleClientContext, ServicesContext)]
pub struct HyperlaneContext {
    #[config]
    pub env: StdGadgetConfiguration,
    data_dir: PathBuf,
    connection: Arc<Docker>,
    container: Arc<Mutex<Option<String>>>,
    #[call_id]
    call_id: Option<u64>,
}

const IMAGE: &str = "gcr.io/abacus-labs-dev/hyperlane-agent:main";
impl HyperlaneContext {
    pub async fn new(env: StdGadgetConfiguration, data_dir: PathBuf) -> Result<Self> {
        let connection = connect_to_docker(None).await?;
        Ok(Self {
            env,
            data_dir,
            connection,
            container: Arc::new(Mutex::new(None)),
            call_id: None,
        })
    }

    #[tracing::instrument(skip_all)]
    async fn spinup_container(&self) -> Result<()> {
        let mut container_guard = self.container.lock().await;
        if container_guard.is_some() {
            return Ok(());
        }

        tracing::info!("Spinning up new container");

        // TODO: Bollard isn't pulling the image for some reason?
        let output = Command::new("docker").args(["pull", IMAGE]).output()?;
        if !output.status.success() {
            return Err(eyre!("Docker pull failed"));
        }

        let mut container = Container::new(&self.connection, IMAGE);

        let keystore = self.env.keystore()?;
        let ecdsa = keystore.ecdsa_key()?.alloy_key()?;
        let secret = hex::encode(ecdsa.to_bytes());

        let hyperlane_db_path = self.hyperlane_db_path();
        if !hyperlane_db_path.exists() {
            tracing::warn!("Hyperlane DB does not exist, creating...");
            std::fs::create_dir_all(&hyperlane_db_path)?;
            tracing::info!("Hyperlane DB created at `{}`", hyperlane_db_path.display());
        }

        let mut binds = vec![format!("{}:/hyperlane_db", hyperlane_db_path.display())];

        let agent_configs_path = self.agent_configs_path();
        let agent_configs_path_exists = agent_configs_path.exists();
        if agent_configs_path_exists {
            binds.push(format!(
                "{}:/config:ro",
                agent_configs_path.to_string_lossy()
            ));
        }

        let mut env = Vec::new();

        if agent_configs_path_exists {
            let mut config_files = Vec::new();

            let files = std::fs::read_dir(agent_configs_path)?;
            for config in files {
                let path = config?.path();
                if path.is_file() {
                    config_files.push(format!(
                        "/config/{}",
                        path.file_name().unwrap().to_string_lossy()
                    ));
                }
            }

            env.push(format!("CONFIG_FILES={}", config_files.join(",")));
        }

        let origin_chain_name_path = self.origin_chain_name_path();
        if origin_chain_name_path.exists() {
            let origin_chain_name = std::fs::read_to_string(origin_chain_name_path)?;
            env.push(format!("HYP_ORIGINCHAINNAME={origin_chain_name}"));
        }

        container
            .env(env)
            .binds(binds)
            .cmd([
                "./validator",
                "--db /hyperlane_db",
                "--validator.key",
                &format!("0x{secret}"),
            ])
            .create()
            .await?;

        if self.env.test_mode {
            let id = container.id().unwrap();
            self.connection
                .connect_network(
                    "hyperlane_validator_test_net",
                    ConnectNetworkOptions {
                        container: id,
                        ..Default::default()
                    },
                )
                .await?;
        }

        container.start(false).await?;
        *container_guard = container.id().map(ToString::to_string);

        // Allow time to spin up
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;

        let status = container.status().await?;

        // Container is down, something's wrong.
        if !status.unwrap().is_active() {
            return Err(eyre!("Failed to start container, config error?"));
        }

        Ok(())
    }

    async fn revert_configs(&self) -> Result<()> {
        tracing::error!("Container failed to start with new configs, reverting");

        self.remove_existing_container().await?;

        let original_configs_path = self.original_agent_configs_path();
        if !original_configs_path.exists() {
            // There is no config to revert
            return Err(eyre!("Configs failed to apply, with no fallback"));
        }

        let configs_path = self.agent_configs_path();

        tracing::debug!(
            "Moving `{}` to `{}`",
            original_configs_path.display(),
            configs_path.display()
        );
        std::fs::remove_dir_all(&configs_path)?;
        std::fs::rename(original_configs_path, configs_path)?;

        let original_origin_chain_name_path = self.original_origin_chain_name_path();
        if original_origin_chain_name_path.exists() {
            let origin_chain_name_path = self.origin_chain_name_path();
            tracing::debug!(
                "Moving `{}` to `{}`",
                original_origin_chain_name_path.display(),
                origin_chain_name_path.display(),
            );
            std::fs::rename(original_origin_chain_name_path, origin_chain_name_path)?;
        }

        self.spinup_container().await?;
        Ok(())
    }

    pub async fn remove_existing_container(&self) -> Result<()> {
        let mut container_id = self.container.lock().await;
        if let Some(container_id) = container_id.take() {
            tracing::warn!("Removing existing container...");
            let mut c = Container::from_id(&self.connection, container_id).await?;
            c.stop().await?;
            c.remove(None).await?;
        }

        Ok(())
    }

    fn hyperlane_db_path(&self) -> PathBuf {
        self.data_dir.join("hyperlane_db")
    }

    fn agent_configs_path(&self) -> PathBuf {
        self.data_dir.join("agent_configs")
    }

    fn original_agent_configs_path(&self) -> PathBuf {
        self.data_dir.join("agent_configs.orig")
    }

    fn origin_chain_name_path(&self) -> PathBuf {
        self.data_dir.join("origin_chain_name.txt")
    }

    fn original_origin_chain_name_path(&self) -> PathBuf {
        self.data_dir.join("origin_chain_name.txt.orig")
    }
}

#[sdk::job(
    id = 0,
    params(config_urls, origin_chain_name),
    result(_),
    event_listener(
        listener = TangleEventListener<HyperlaneContext, JobCalled>,
        pre_processor = services_pre_processor,
        post_processor = services_post_processor,
    ),
)]
pub async fn set_config(
    ctx: HyperlaneContext,
    config_urls: Option<Vec<String>>,
    origin_chain_name: String,
) -> Result<u64> {
    let mut configs = Vec::new();
    if let Some(config_urls) = config_urls {
        for config_url in config_urls {
            // https://github.com/seanmonstar/reqwest/issues/178
            let url = reqwest::Url::parse(&config_url)?;
            if url.scheme() == "file" && ctx.env.test_mode {
                let config = std::fs::read_to_string(url.to_file_path().unwrap())?;
                configs.push(config);
                continue;
            }
            configs.push(reqwest::get(config_url).await?.text().await?);
        }
    }

    // TODO: First step, verify the config is valid. Is there an easy way to do so?
    if origin_chain_name.is_empty() {
        return Err(eyre!(
            "`origin_chain_name` is invalid, ensure it contains a name"
        ));
    }

    ctx.remove_existing_container().await?;

    let configs_path = ctx.agent_configs_path();
    if configs_path.exists() {
        let orig_configs_path = ctx.original_agent_configs_path();
        tracing::info!("Configs path exists, backing up.");
        if orig_configs_path.exists() {
            tracing::warn!("Removing old backup at {}", orig_configs_path.display());
            std::fs::remove_dir_all(&orig_configs_path)?;
        }

        std::fs::rename(&configs_path, orig_configs_path)?;
        std::fs::create_dir_all(&configs_path)?;
    }

    let origin_chain_name_path = ctx.origin_chain_name_path();
    if origin_chain_name_path.exists() {
        let orig_origin_chain_name_path = ctx.original_origin_chain_name_path();
        if orig_origin_chain_name_path.exists() {
            tracing::warn!("Removing old backup at {}", orig_origin_chain_name_path.display());
            std::fs::remove_file(&orig_origin_chain_name_path)?;
        }

        tracing::info!("Origin chain exists, backing up.");
        std::fs::rename(&origin_chain_name_path, orig_origin_chain_name_path)?;
    }

    std::fs::create_dir_all(&configs_path)?;
    if configs.is_empty() {
        tracing::info!("No configs provided, using defaults");
    } else {
        // TODO: Limit number of configs?
        for (index, config) in configs.iter().enumerate() {
            std::fs::write(configs_path.join(format!("{index}.json")), config)?;
        }
        tracing::info!("New configs written to: {}", configs_path.display());
    }

    std::fs::write(&origin_chain_name_path, origin_chain_name)?;
    tracing::info!(
        "Origin chain written to: {}",
        origin_chain_name_path.display()
    );

    if let Err(e) = ctx.spinup_container().await {
        // Something went wrong spinning up the container, possibly bad config. Try to revert.
        tracing::error!("{e}");
        ctx.revert_configs().await?;
    }

    Ok(0)
}
