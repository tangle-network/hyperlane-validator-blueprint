use blueprint_sdk::alloy::network::EthereumWallet;
use blueprint_sdk::alloy::primitives::{address, Address, Bytes};
use blueprint_sdk::alloy::providers::{Provider, RootProvider};
use blueprint_sdk::alloy::rpc::types::Filter;
use blueprint_sdk::alloy::signers::local::PrivateKeySigner;
use blueprint_sdk::alloy::sol;
use blueprint_sdk::alloy::sol_types::SolEvent;
use blueprint_sdk::alloy::transports::BoxTransport;
use blueprint_sdk::config::GadgetConfiguration;
use blueprint_sdk::contexts::keystore::KeystoreContext;
use blueprint_sdk::crypto::sp_core::SpEcdsa;
use blueprint_sdk::keystore::backends::Backend;
use blueprint_sdk::logging::{self, setup_log};
use blueprint_sdk::macros::ext::blueprint_serde::to_field;
use blueprint_sdk::macros::ext::futures::StreamExt;
use blueprint_sdk::testing::tempfile::{self, TempDir};
use blueprint_sdk::testing::utils::anvil::start_anvil_container;
use blueprint_sdk::testing::utils::harness::TestHarness;
use blueprint_sdk::testing::utils::runner::TestEnv;
use blueprint_sdk::testing::utils::tangle::{OutputValue, TangleTestHarness};
use blueprint_sdk::utils::evm::get_wallet_provider_http;
use bollard::container::RemoveContainerOptions;
use bollard::models::EndpointSettings;
use bollard::network::{ConnectNetworkOptions, CreateNetworkOptions, InspectNetworkOptions};
use color_eyre::Report;
use color_eyre::Result;
use dockworker::DockerBuilder;
use hyperlane_validator_blueprint::{HyperlaneContext, SetConfigEventHandler};
use std::path::Path;
use std::process::Command;
use std::str::FromStr;
use std::time::Duration;
use testcontainers::ContainerAsync;
use testcontainers::GenericImage;

const AGENT_CONFIG_TEMPLATE_PATH: &str = "./test_assets/agent-config.json.template";
const TEST_ASSETS_PATH: &str = "./test_assets";

fn setup_temp_dir(
    (testnet1_docker_rpc_url, testnet1_host_rpc_url): (String, String),
    (testnet2_docker_rpc_url, testnet2_host_rpc_url): (String, String),
) -> Result<TempDir> {
    const FILE_PREFIXES: [&str; 2] = ["testnet1", "testnet2"];

    let tempdir = tempfile::tempdir()?;

    // Create the signatures directory
    let signatures_path = tempdir.path().join("signatures-testnet1");
    std::fs::create_dir_all(&signatures_path)?;

    // Create the registry
    let registry_path = tempdir.path().join("chains");
    std::fs::create_dir(&registry_path)?;

    for (prefix, rpc_url) in FILE_PREFIXES
        .iter()
        .zip([&*testnet1_host_rpc_url, &*testnet2_host_rpc_url])
    {
        let testnet_path = registry_path.join(prefix);
        std::fs::create_dir(&testnet_path)?;

        let addresses_path = Path::new(TEST_ASSETS_PATH).join(format!("{prefix}-addresses.yaml"));
        std::fs::copy(addresses_path, testnet_path.join("addresses.yaml"))?;

        let metadata_template_path =
            Path::new(TEST_ASSETS_PATH).join(format!("{prefix}-metadata.yaml.template"));
        let testnet1_metadata = std::fs::read_to_string(metadata_template_path)?;
        std::fs::write(
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

fn new_agent_config(
    output_path: &Path,
    testnet1_rpc: &str,
    testnet2_rpc: &str,
    tmp_syncer_dir: &str,
) -> Result<()> {
    let agent_config_template = std::fs::read_to_string(AGENT_CONFIG_TEMPLATE_PATH)?;
    std::fs::write(
        output_path,
        agent_config_template
            .replace("{TESTNET_1_RPC}", testnet1_rpc)
            .replace("{TESTNET_2_RPC}", testnet2_rpc)
            .replace("{TMP_SYNCER_DIR}", tmp_syncer_dir),
    )?;

    Ok(())
}

const TESTNET1_STATE_PATH: &str = "./test_assets/testnet1-state.json";
const TESTNET2_STATE_PATH: &str = "./test_assets/testnet2-state.json";

const VALIDATOR_NETWORK_NAME: &str = "hyperlane_validator_test_net";
const RELAYER_NETWORK_NAME: &str = "hyperlane_relayer_test_net";

struct Testnet {
    container: ContainerAsync<GenericImage>,
    validator_network_ip: String,
    relayer_network_ip: String,
}

async fn spinup_anvil_testnets() -> Result<(Testnet, Testnet)> {
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
            .connect_network(
                network,
                ConnectNetworkOptions {
                    container: origin.id(),
                    ..Default::default()
                },
            )
            .await?;

        connection
            .get_client()
            .connect_network(
                network,
                ConnectNetworkOptions {
                    container: dest.id(),
                    ..Default::default()
                },
            )
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

    let (origin_container, _, _) = start_anvil_container(TESTNET1_STATE_PATH, false).await;

    let (dest_container, _, _) = start_anvil_container(TESTNET2_STATE_PATH, false).await;

    let connection = DockerBuilder::new().await?;
    let validator_network_config = setup_network(
        &connection,
        VALIDATOR_NETWORK_NAME,
        &origin_container,
        &dest_container,
    )
    .await?;

    let relayer_network_config = setup_network(
        &connection,
        RELAYER_NETWORK_NAME,
        &origin_container,
        &dest_container,
    )
    .await?;

    Ok((
        Testnet {
            container: origin_container,
            validator_network_ip: validator_network_config.0.ip_address.unwrap(),
            relayer_network_ip: relayer_network_config.0.ip_address.unwrap(),
        },
        Testnet {
            container: dest_container,
            validator_network_ip: validator_network_config.1.ip_address.unwrap(),
            relayer_network_ip: relayer_network_config.1.ip_address.unwrap(),
        },
    ))
}

async fn spinup_relayer(
    origin_testnet: &Testnet,
    dest_testnet: &Testnet,
    mut env: GadgetConfiguration,
    tmp_dir: &Path,
) -> Result<()> {
    let data_dir = tmp_dir.join("relayer");
    std::fs::create_dir_all(&data_dir)?;

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
    std::fs::create_dir_all(&keystore_path)?;

    env.keystore_uri = format!("{}", std::path::absolute(keystore_path)?.display());
    env.keystore()
        .generate_from_string::<SpEcdsa>("//Relayer")?;

    let context = hyperlane_relayer_blueprint::HyperlaneContext::new(env, data_dir).await?;
    let result = hyperlane_relayer_blueprint::set_config(
        context,
        Some(vec![format!("file://{}", config_path.display())]),
        String::from("testnet1,testnet2"),
    )
    .await?;
    assert_eq!(result, 0);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn validator() -> Result<()> {
    color_eyre::install()?;
    setup_log();

    // Test logic is separated so that cleanup is performed regardless of failure
    let res = validator_test_inner().await;

    // Cleanup networks
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

    res
}

sol!(
    #[allow(missing_docs, clippy::too_many_arguments)]
    #[sol(rpc)]
    #[derive(Debug)]
    Mailbox,
    "contracts/out/Mailbox.sol/Mailbox.json"
);

sol!(
    #[allow(missing_docs, clippy::too_many_arguments)]
    #[sol(rpc)]
    #[derive(Debug)]
    TestRecipient,
    "contracts/out/TestRecipient.sol/TestRecipient.json"
);

async fn mine_block(rpc_url: &str) -> Result<()> {
    logging::debug!("Mining a block");
    Command::new("cast")
        .args(["rpc", "anvil_mine", "1", "--rpc-url", rpc_url])
        .output()?;

    // Give the command a few seconds
    tokio::time::sleep(Duration::from_secs(5)).await;

    Ok(())
}

fn wallet_for(key: &str, rpc: &str) -> (EthereumWallet, RootProvider<BoxTransport>) {
    let wallet = EthereumWallet::new(PrivateKeySigner::from_str(key).unwrap());

    let provider = get_wallet_provider_http(rpc, wallet.clone());
    (wallet, provider)
}

const TESTNET1_MAILBOX: Address = address!("0xB7f8BC63BbcaD18155201308C8f3540b07f84F5e");
const MESSAGE: &str = "Hello";

async fn validator_test_inner() -> Result<()> {
    let (origin_testnet, dest_testnet) = spinup_anvil_testnets().await?;

    // The validator itself uses the IPs internal to the Docker network.
    // When it comes time to relay the message, the command is run outside the Docker network,
    // so we need to get both addresses.
    //
    // The internal address is written to `agent-config.json`.
    // The host addresses are written to `testnet{1,2}-metadata.yaml`.
    let testnet1_docker_rpc_url = format!("http://{}:8545", origin_testnet.validator_network_ip);
    let testnet2_docker_rpc_url = format!("http://{}:8545", dest_testnet.validator_network_ip);

    let origin_ports = origin_testnet.container.ports().await?;
    let dest_ports = dest_testnet.container.ports().await?;

    let testnet1_host_rpc_url = format!(
        "http://127.0.0.1:{}",
        origin_ports.map_to_host_port_ipv4(8545).unwrap()
    );
    let testnet2_host_rpc_url = format!(
        "http://127.0.0.1:{}",
        dest_ports.map_to_host_port_ipv4(8545).unwrap()
    );

    let tempdir = setup_temp_dir(
        (testnet1_docker_rpc_url, testnet1_host_rpc_url.clone()),
        (testnet2_docker_rpc_url, testnet2_host_rpc_url.clone()),
    )?;
    let temp_dir_path = tempdir.path().to_path_buf();

    let harness = TangleTestHarness::setup(tempdir).await?;

    let ctx = HyperlaneContext::new(harness.env().clone(), temp_dir_path.clone()).await?;

    let handler = SetConfigEventHandler::new(harness.env(), ctx).await?;

    // Setup service
    let (mut test_env, service_id, _) = harness.setup_services(false).await?;
    test_env.add_job(handler);

    tokio::spawn(async move {
        test_env.run_runner().await.unwrap();
    });

    // Pass the arguments
    let agent_config_path = std::path::absolute(temp_dir_path.join("agent-config.json"))?;
    let config_urls = to_field(Some(vec![format!(
        "file://{}",
        agent_config_path.display()
    )]))?;
    let origin_chain_name = to_field(String::from("testnet1"))?;

    // Execute job and verify result
    let results = harness
        .execute_job(
            service_id,
            0,
            vec![config_urls, origin_chain_name],
            vec![OutputValue::Uint64(0)],
        )
        .await?;

    assert_eq!(results.service_id, service_id);

    tracing::info!("Validator running, starting relayer...");
    spinup_relayer(
        &origin_testnet,
        &dest_testnet,
        harness.env().clone(),
        &temp_dir_path,
    )
    .await?;

    tracing::info!("Relayer running, sending message...");
    std::env::set_current_dir(temp_dir_path)?;

    tracing::info!("Getting Testnet1's mailbox");
    let (_testnet1_wallet, testnet1_provider) = wallet_for(
        &hex::encode(harness.alloy_key.to_bytes()),
        &testnet1_host_rpc_url,
    );
    let testnet1_mailbox = Mailbox::new(TESTNET1_MAILBOX, testnet1_provider.clone());

    let (_testnet2_wallet, testnet2_provider) = wallet_for(
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        &testnet2_host_rpc_url,
    );

    tracing::info!("Deploying recipient");
    let recipient = TestRecipient::deploy(testnet2_provider.clone()).await?;

    tracing::info!(
        "Dispatching message `{MESSAGE:?}` to recipient `{}`",
        recipient.address()
    );
    let tx = testnet1_mailbox
        .dispatch_2(31338, recipient.address().into_word(), Bytes::from(MESSAGE))
        .send()
        .await?;
    let receipt = tx.get_receipt().await?;
    if !receipt.status() {
        logging::error!("Failed to dispatch message");
        return Err(Report::msg("Failed to dispatch message"));
    }

    let mut message_id = None;
    for log in receipt.inner.logs() {
        let Ok(e) = Mailbox::DispatchId::decode_log_data(log.data(), true) else {
            continue;
        };

        message_id = Some(e.messageId);
    }

    let Some(message_id) = message_id else {
        return Err(Report::msg("No `DispatchId` event found"));
    };

    let message_id = hex::encode(message_id);
    tracing::info!("Message ID: {message_id}");

    mine_block(&testnet1_host_rpc_url).await?;

    let received_event_filter = Filter::new()
        .address(*recipient.address())
        .event("Received(uint32,bytes32,bytes)")
        .select(0..);

    let mut stream = testnet2_provider
        .watch_logs(&received_event_filter)
        .await?
        .into_stream();

    // Wait for message to be sent...
    let timeout_duration = Duration::from_secs(20);
    let timeout_result = tokio::time::timeout(timeout_duration, async {
        while let Some(logs) = stream.next().await {
            if let Some(log) = logs.into_iter().next() {
                let ack = TestRecipient::Received::decode_log_data(log.data(), true)?;
                if &ack._2 != MESSAGE.as_bytes() {
                    return Err(Report::msg(format!(
                        "Recipient received the wrong message: {:?}",
                        ack._2
                    )));
                }

                logging::info!(
                    "Recipient at `{}` received message `{}`",
                    recipient.address(),
                    MESSAGE
                );
                return Ok(());
            }
        }

        Err(Report::msg("Stream died, cannot check for Received event"))
    })
    .await;

    match timeout_result {
        Ok(res) => res?,
        Err(_) => return Err(Report::msg("The recipient never handled the message")),
    };

    Ok(())
}
