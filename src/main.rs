use color_eyre::Result;
use gadget_sdk as sdk;
use hyperlane_validator_blueprint as blueprint;
use sdk::runners::tangle::TangleConfig;
use sdk::runners::BlueprintRunner;
use std::path::{Path, PathBuf};

fn default_data_dir() -> PathBuf {
    const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");
    Path::new(MANIFEST_DIR).join("data")
}

#[sdk::main(env)]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let data_dir = match env.data_dir.clone() {
        Some(dir) => dir,
        None => {
            tracing::warn!("Data dir not specified, using default");
            default_data_dir()
        }
    };

    if !data_dir.exists() {
        tracing::warn!("Data dir does not exist, creating");
        std::fs::create_dir_all(&data_dir)?;
    }

    let ctx = blueprint::HyperlaneContext::new(env.clone(), data_dir).await?;

    let set_config = blueprint::SetConfigEventHandler::new(&env, ctx).await?;

    tracing::info!("Starting the event watcher ...");
    let tangle_config = TangleConfig::default();
    BlueprintRunner::new(tangle_config, env)
        .job(set_config)
        .run()
        .await?;

    tracing::info!("Exiting...");
    Ok(())
}
