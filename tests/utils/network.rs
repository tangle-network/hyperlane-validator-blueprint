use blueprint_sdk as sdk;
use blueprint_sdk::testing::chain_setup::anvil::AnvilTestnet;
use bollard::container::RemoveContainerOptions;
use bollard::models::EndpointSettings;
use bollard::network::{ConnectNetworkOptions, CreateNetworkOptions, InspectNetworkOptions};
use color_eyre::Result;
use dockworker::DockerBuilder;
use sdk::crypto::sp_core::SpEcdsa;
use sdk::extract::Context;
use sdk::keystore::backends::Backend;
use sdk::runner::config::BlueprintEnvironment;
use sdk::tangle::extract::TangleArgs2;
use sdk::testing::chain_setup::anvil::start_anvil_container;
use sdk::testing::tempfile::{self, TempDir};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use testcontainers::ContainerAsync;
use testcontainers::GenericImage;

/// Network name for validator tests
pub const VALIDATOR_NETWORK_NAME: &str = "hyperlane_validator_test_net";
/// Network name for relayer tests
pub const RELAYER_NETWORK_NAME: &str = "hyperlane_relayer_test_net";
pub const TESTNET1_STATE_PATH: &str = "./test_assets/testnet1-state.json";
pub const TESTNET2_STATE_PATH: &str = "./test_assets/testnet2-state.json";
pub const AGENT_CONFIG_TEMPLATE_PATH: &str = "./test_assets/agent-config.json.template";
pub const TEST_ASSETS_PATH: &str = "./test_assets";

/// Represents a testnet running in a Docker container
pub struct Testnet {
    pub inner: AnvilTestnet,
    pub validator_network_ip: String,
    pub relayer_network_ip: String,
}

/// Spins up two Anvil testnets and connects them via Docker networks
pub async fn spinup_anvil_testnets(
    testnet1_state_path: &str,
    testnet2_state_path: &str,
) -> Result<(Testnet, Testnet)> {
    async fn setup_network(
        connection: &DockerBuilder,
        network: &'static str,
        origin: &ContainerAsync<GenericImage>,
        dest: &ContainerAsync<GenericImage>,
    ) -> Result<(EndpointSettings, EndpointSettings)> {
        if let Err(e) = connection
            .get_client()
            .create_network(CreateNetworkOptions {
                name: network,
                ..Default::default()
            })
            .await
        {
            match e {
                bollard::errors::Error::DockerResponseServerError {
                    status_code: 409, ..
                } => {}
                _ => return Err(e.into()),
            }
        }

        connection
            .get_client()
            .connect_network(network, ConnectNetworkOptions {
                container: origin.id(),
                ..Default::default()
            })
            .await?;

        connection
            .get_client()
            .connect_network(network, ConnectNetworkOptions {
                container: dest.id(),
                ..Default::default()
            })
            .await?;

        let origin_container_inspect = connection
            .get_client()
            .inspect_container(origin.id(), None)
            .await?;
        let origin_network_settings = origin_container_inspect
            .network_settings
            .unwrap()
            .networks
            .unwrap()[network]
            .clone();

        let dest_container_inspect = connection
            .get_client()
            .inspect_container(dest.id(), None)
            .await?;
        let dest_network_settings = dest_container_inspect
            .network_settings
            .unwrap()
            .networks
            .unwrap()[network]
            .clone();

        Ok((origin_network_settings, dest_network_settings))
    }

    let origin_state = fs::read_to_string(testnet1_state_path)?;
    let origin = start_anvil_container(&origin_state, false).await;

    let dest_state = fs::read_to_string(testnet2_state_path)?;
    let dest = start_anvil_container(&dest_state, false).await;

    let connection = DockerBuilder::new().await?;
    let validator_network_config = setup_network(
        &connection,
        VALIDATOR_NETWORK_NAME,
        &origin.container,
        &dest.container,
    )
    .await?;

    let relayer_network_config = setup_network(
        &connection,
        RELAYER_NETWORK_NAME,
        &origin.container,
        &dest.container,
    )
    .await?;

    Ok((
        Testnet {
            inner: origin,
            validator_network_ip: validator_network_config.0.ip_address.unwrap(),
            relayer_network_ip: relayer_network_config.0.ip_address.unwrap(),
        },
        Testnet {
            inner: dest,
            validator_network_ip: validator_network_config.1.ip_address.unwrap(),
            relayer_network_ip: relayer_network_config.1.ip_address.unwrap(),
        },
    ))
}

/// Cleans up Docker networks created for testing
pub async fn cleanup_networks() -> Result<()> {
    let connection = DockerBuilder::new().await?;
    for network_name in [VALIDATOR_NETWORK_NAME, RELAYER_NETWORK_NAME] {
        let network = connection
            .get_client()
            .inspect_network(network_name, None::<InspectNetworkOptions<String>>)
            .await?;
        for container in network.containers.unwrap().keys() {
            connection
                .get_client()
                .remove_container(
                    container,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await?;
        }

        connection.remove_network(network_name).await?;
    }

    Ok(())
}

/// Spins up a relayer for testing
pub async fn spinup_relayer(
    origin_testnet: &Testnet,
    dest_testnet: &Testnet,
    mut env: BlueprintEnvironment,
    tmp_dir: &Path,
) -> Result<()> {
    let data_dir = tmp_dir.join("relayer");
    fs::create_dir_all(&data_dir)?;

    let config_path = std::path::absolute(data_dir.join("agent-config.json"))?;

    let syncer_dir = tmp_dir.parent().unwrap().join("signatures-testnet1");
    new_agent_config(
        &config_path,
        &format!("http://{}:8545", origin_testnet.relayer_network_ip),
        &format!("http://{}:8545", dest_testnet.relayer_network_ip),
        &syncer_dir.to_string_lossy(),
    )?;

    // Give the relayer a new keystore
    let keystore_path = data_dir.join("keystore");
    fs::create_dir_all(&keystore_path)?;

    env.keystore_uri = format!("{}", std::path::absolute(keystore_path)?.display());
    env.keystore()
        .generate_from_string::<SpEcdsa>("//Relayer")?;

    let context = hyperlane_relayer_blueprint_lib::HyperlaneContext::new(env, data_dir).await?;
    let result = hyperlane_relayer_blueprint_lib::set_config(
        Context(Arc::new(context)),
        TangleArgs2(
            Some(vec![format!("file://{}", config_path.display())].into()).into(),
            String::from("testnet1,testnet2"),
        ),
    )
    .await?;
    assert_eq!(result.0, 0);

    Ok(())
}

/// Creates a new agent config file from a template
///
/// # Parameters
/// * `output_path` - Path to write the config file to
/// * `testnet1_rpc` - RPC URL for testnet1
/// * `testnet2_rpc` - RPC URL for testnet2
/// * `tmp_syncer_dir` - Directory for the signature syncer
pub fn new_agent_config(
    output_path: &Path,
    testnet1_rpc: &str,
    testnet2_rpc: &str,
    tmp_syncer_dir: &str,
) -> Result<()> {
    let agent_config_template = fs::read_to_string(AGENT_CONFIG_TEMPLATE_PATH)?;
    fs::write(
        output_path,
        agent_config_template
            .replace("{TESTNET_1_RPC}", testnet1_rpc)
            .replace("{TESTNET_2_RPC}", testnet2_rpc)
            .replace("{TMP_SYNCER_DIR}", tmp_syncer_dir),
    )?;

    Ok(())
}

/// Sets up temporary directory with chain configuration
pub fn setup_temp_dir(
    (testnet1_docker_rpc_url, testnet1_host_rpc_url): (String, String),
    (testnet2_docker_rpc_url, testnet2_host_rpc_url): (String, String),
) -> Result<TempDir> {
    const FILE_PREFIXES: [&str; 2] = ["testnet1", "testnet2"];

    let tempdir = tempfile::tempdir()?;

    // Create the signatures directory
    let signatures_path = tempdir.path().join("signatures-testnet1");
    fs::create_dir_all(&signatures_path)?;

    // Create the registry
    let registry_path = tempdir.path().join("chains");
    fs::create_dir(&registry_path)?;

    for (prefix, rpc_url) in FILE_PREFIXES
        .iter()
        .zip([&*testnet1_host_rpc_url, &*testnet2_host_rpc_url])
    {
        let testnet_path = registry_path.join(prefix);
        fs::create_dir(&testnet_path)?;

        let addresses_path = Path::new(TEST_ASSETS_PATH).join(format!("{prefix}-addresses.yaml"));
        fs::copy(addresses_path, testnet_path.join("addresses.yaml"))?;

        let metadata_template_path =
            Path::new(TEST_ASSETS_PATH).join(format!("{prefix}-metadata.yaml.template"));
        let testnet1_metadata = fs::read_to_string(metadata_template_path)?;
        fs::write(
            testnet_path.join("metadata.yaml"),
            testnet1_metadata.replace("{RPC_URL}", rpc_url),
        )?;
    }

    // Create agent config
    new_agent_config(
        &tempdir.path().join("agent-config.json"),
        &testnet1_docker_rpc_url,
        &testnet2_docker_rpc_url,
        &signatures_path.to_string_lossy(),
    )?;

    Ok(tempdir)
}
