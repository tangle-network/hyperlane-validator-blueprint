use api::services::events::JobCalled;
use color_eyre::Result;
use gadget_sdk as sdk;
use sdk::config::StdGadgetConfiguration;
use sdk::contexts::{ServicesContext, TangleClientContext};
use sdk::docker::bollard::Docker;
use sdk::docker::connect_to_docker;
use sdk::event_listener::tangle::{
    jobs::{services_post_processor, services_pre_processor},
    TangleEventListener,
};
use sdk::tangle_subxt::tangle_testnet_runtime::api;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(TangleClientContext, ServicesContext)]
pub struct HyperlaneContext {
    #[config]
    pub env: StdGadgetConfiguration,
    data_dir: PathBuf,
    connection: Arc<Docker>,
    container: Mutex<Option<String>>,
}

const IMAGE: &str = "gcr.io/abacus-labs-dev/hyperlane-agent:main";
impl HyperlaneContext {
    pub async fn new(env: StdGadgetConfiguration, data_dir: PathBuf) -> Result<Self> {
        let connection = connect_to_docker(None).await?;
        Ok(Self {
            env,
            data_dir,
            connection,
            container: Mutex::new(None),
        })
    }
}

#[sdk::job(
    id = 0,
    params(config_urls, origin_chain_name),
    result(_),
    event_listener(
        listener = TangleEventListener<Arc<HyperlaneContext>, JobCalled>,
        pre_processor = services_pre_processor,
        post_processor = services_post_processor,
    ),
)]
pub async fn set_config(
    ctx: Arc<HyperlaneContext>,
    config_urls: Option<Vec<String>>,
    origin_chain_name: String,
) -> Result<u64> {
    Ok(0)
}
