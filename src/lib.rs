use blueprint_sdk as sdk;
use bollard::network::ConnectNetworkOptions;
use color_eyre::Result;
use color_eyre::eyre::eyre;
use docktopus::DockerBuilder;
use docktopus::container::Container;
use sdk::crypto::sp_core::SpEcdsa;
use sdk::crypto::tangle_pair_signer::TanglePairSigner;
use sdk::extract::Context;
use sdk::keystore::backends::Backend;
use sdk::macros::context::{ServicesContext, TangleClientContext};
use sdk::runner::config::BlueprintEnvironment;
use sdk::tangle::extract::{List, Optional, TangleArgs2, TangleResult};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tokio::sync::Mutex;

pub fn default_data_dir() -> PathBuf {
    const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");
    Path::new(MANIFEST_DIR).join("data")
}

#[derive(Clone, TangleClientContext, ServicesContext)]
pub struct HyperlaneContext {
    #[config]
    pub env: BlueprintEnvironment,
    data_dir: PathBuf,
    connection: Arc<DockerBuilder>,
    container: Arc<Mutex<Option<String>>>,
}

const IMAGE: &str = "gcr.io/abacus-labs-dev/hyperlane-agent:agents-v1.2.0";
impl HyperlaneContext {
    pub async fn new(env: BlueprintEnvironment, data_dir: PathBuf) -> Result<Self> {
        let connection = DockerBuilder::new().await?;
        Ok(Self {
            env,
            data_dir,
            connection: Arc::new(connection),
            container: Arc::new(Mutex::new(None)),
        })
    }

    #[tracing::instrument(skip_all)]
    async fn spinup_container(&self) -> Result<()> {
        let mut container_guard = self.container.lock().await;
        if container_guard.is_some() {
            return Ok(());
        }

        blueprint_sdk::info!("Spinning up new container");

        // TODO: Bollard isn't pulling the image for some reason?
        let output = Command::new("docker").args(["pull", IMAGE]).output()?;
        if !output.status.success() {
            return Err(eyre!("Docker pull failed"));
        }

        let mut container = Container::new(self.connection.client(), IMAGE);

        let keystore = self.env.keystore();
        let ecdsa_pub = keystore.first_local::<SpEcdsa>()?;
        let ecdsa_pair = keystore.get_secret::<SpEcdsa>(&ecdsa_pub)?;
        let tangle_ecdsa_pair = TanglePairSigner::new(ecdsa_pair.0);

        let alloy_key = tangle_ecdsa_pair.alloy_key()?;
        let secret = hex::encode(alloy_key.to_bytes());

        let hyperlane_db_path = self.hyperlane_db_path();
        if !hyperlane_db_path.exists() {
            blueprint_sdk::warn!("Hyperlane DB does not exist, creating...");
            std::fs::create_dir_all(&hyperlane_db_path)?;
            blueprint_sdk::info!("Hyperlane DB created at `{}`", hyperlane_db_path.display());
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

            if !config_files.is_empty() {
                env.push(format!("CONFIG_FILES={}", config_files.join(",")));
            }
        }

        let origin_chain_name_path = self.origin_chain_name_path();
        if origin_chain_name_path.exists() {
            let origin_chain_name = std::fs::read_to_string(origin_chain_name_path)?;
            env.push(format!("HYP_ORIGINCHAINNAME={origin_chain_name}"));
        }

        container = container.env(env).binds(binds).cmd([
            "./validator",
            "--db /hyperlane_db",
            "--validator.key",
            &format!("0x{secret}"),
        ]);

        container.create().await?;

        if self.env.test_mode {
            let id = container.id().unwrap();
            self.connection
                .client()
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
        blueprint_sdk::error!("Container failed to start with new configs, reverting");

        self.remove_existing_container().await?;

        let original_configs_path = self.original_agent_configs_path();
        if !original_configs_path.exists() {
            // There is no config to revert
            return Err(eyre!("Configs failed to apply, with no fallback"));
        }

        let configs_path = self.agent_configs_path();

        blueprint_sdk::debug!(
            "Moving `{}` to `{}`",
            original_configs_path.display(),
            configs_path.display()
        );
        std::fs::remove_dir_all(&configs_path)?;
        std::fs::rename(original_configs_path, configs_path)?;

        let original_origin_chain_name_path = self.original_origin_chain_name_path();
        if original_origin_chain_name_path.exists() {
            let origin_chain_name_path = self.origin_chain_name_path();
            blueprint_sdk::debug!(
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
            blueprint_sdk::warn!("Removing existing container...");
            let mut c = Container::from_id(self.connection.client(), container_id).await?;
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

pub const SET_CONFIG_JOB_ID: u8 = 0;

pub async fn set_config(
    Context(ctx): Context<HyperlaneContext>,
    TangleArgs2(Optional(config_urls), origin_chain_name): TangleArgs2<
        Optional<List<String>>,
        String,
    >,
) -> Result<TangleResult<u64>> {
    let mut configs = Vec::new();
    if let Some(List(config_urls)) = config_urls {
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
        blueprint_sdk::info!("Configs path exists, backing up.");
        if orig_configs_path.exists() {
            blueprint_sdk::warn!("Removing old backup at {}", orig_configs_path.display());
            std::fs::remove_dir_all(&orig_configs_path)?;
        }

        std::fs::rename(&configs_path, orig_configs_path)?;
        std::fs::create_dir_all(&configs_path)?;
    }

    let origin_chain_name_path = ctx.origin_chain_name_path();
    if origin_chain_name_path.exists() {
        let orig_origin_chain_name_path = ctx.original_origin_chain_name_path();
        if orig_origin_chain_name_path.exists() {
            blueprint_sdk::warn!(
                "Removing old backup at {}",
                orig_origin_chain_name_path.display()
            );
            std::fs::remove_file(&orig_origin_chain_name_path)?;
        }

        blueprint_sdk::info!("Origin chain exists, backing up.");
        std::fs::rename(&origin_chain_name_path, orig_origin_chain_name_path)?;
    }

    std::fs::create_dir_all(&configs_path)?;
    if configs.is_empty() {
        blueprint_sdk::info!("No configs provided, using defaults");
    } else {
        // TODO: Limit number of configs?
        for (index, config) in configs.iter().enumerate() {
            std::fs::write(configs_path.join(format!("{index}.json")), config)?;
        }
        blueprint_sdk::info!("New configs written to: {}", configs_path.display());
    }

    std::fs::write(&origin_chain_name_path, origin_chain_name)?;
    blueprint_sdk::info!(
        "Origin chain written to: {}",
        origin_chain_name_path.display()
    );

    if let Err(e) = ctx.spinup_container().await {
        // Something went wrong spinning up the container, possibly bad config. Try to revert.
        blueprint_sdk::error!("{e}");
        ctx.revert_configs().await?;
    }

    Ok(TangleResult(0))
}
