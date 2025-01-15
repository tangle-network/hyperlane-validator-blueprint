use crate::HyperlaneContext;
use bollard::container::RemoveContainerOptions;
use bollard::network::{ConnectNetworkOptions, CreateNetworkOptions, InspectNetworkOptions};
use color_eyre::Report;
use color_eyre::Result;
use dockworker::DockerBuilder;
use gadget_macros::ext::blueprint_serde::to_field;
use gadget_testing_utils::anvil::start_anvil_container;
use gadget_testing_utils::tangle::{OutputValue, TangleTestHarness};
use std::path::Path;
use std::process::Command;
use tangle_blueprint_sdk::gadget_logging::setup_log;
use tempfile::TempDir;
use testcontainers::ContainerAsync;
use testcontainers::GenericImage;

const AGENT_CONFIG_TEMPLATE_PATH: &str = "./test_assets/agent-config.json.template";
const TEST_ASSETS_PATH: &str = "./test_assets";

fn setup_temp_dir(
    (testnet1_docker_rpc_url, testnet1_host_rpc_url): (String, String),
    (testnet2_docker_rpc_url, testnet2_host_rpc_url): (String, String),
) -> TempDir {
    const FILE_PREFIXES: [&str; 2] = ["testnet1", "testnet2"];

    let tempdir = tempfile::tempdir().unwrap();

    // Create the signatures directory
    let signatures_path = tempdir.path().join("signatures-testnet1");
    std::fs::create_dir_all(&signatures_path).unwrap();

    // Create the registry
    let registry_path = tempdir.path().join("chains");
    std::fs::create_dir(&registry_path).unwrap();

    for (prefix, rpc_url) in FILE_PREFIXES
        .iter()
        .zip([&*testnet1_host_rpc_url, &*testnet2_host_rpc_url])
    {
        let testnet_path = registry_path.join(prefix);
        std::fs::create_dir(&testnet_path).unwrap();

        let addresses_path = Path::new(TEST_ASSETS_PATH).join(format!("{prefix}-addresses.yaml"));
        std::fs::copy(addresses_path, testnet_path.join("addresses.yaml")).unwrap();

        let metadata_template_path =
            Path::new(TEST_ASSETS_PATH).join(format!("{prefix}-metadata.yaml.template"));
        let testnet1_metadata = std::fs::read_to_string(metadata_template_path).unwrap();
        std::fs::write(
            testnet_path.join("metadata.yaml"),
            testnet1_metadata.replace("{RPC_URL}", rpc_url),
        )
        .unwrap();
    }

    // Create agent config
    let agent_config_template = std::fs::read_to_string(AGENT_CONFIG_TEMPLATE_PATH).unwrap();
    std::fs::write(
        tempdir.path().join("agent-config.json"),
        agent_config_template
            .replace("{TESTNET_1_RPC}", &testnet1_docker_rpc_url)
            .replace("{TESTNET_2_RPC}", &testnet2_docker_rpc_url)
            .replace("{TMP_SYNCER_DIR}", &signatures_path.to_string_lossy()),
    )
    .unwrap();

    tempdir
}

const TESTNET1_STATE_PATH: &str = "./test_assets/testnet1-state.json";
const TESTNET2_STATE_PATH: &str = "./test_assets/testnet2-state.json";

async fn spinup_anvil_testnets() -> Result<(
    (ContainerAsync<GenericImage>, String),
    (ContainerAsync<GenericImage>, String),
)> {
    let (origin_container, _, _) = start_anvil_container(TESTNET1_STATE_PATH, false).await;

    let (dest_container, _, _) = start_anvil_container(TESTNET2_STATE_PATH, false).await;

    let connection = DockerBuilder::new().await?;
    if let Err(e) = connection
        .get_client()
        .create_network(CreateNetworkOptions {
            name: "hyperlane_validator_test_net",
            ..Default::default()
        })
        .await
    {
        match e {
            bollard::errors::Error::DockerResponseServerError {
                status_code: 409, ..
            } => {}
            _ => panic!("{e}"),
        }
    }

    connection
        .get_client()
        .connect_network("hyperlane_validator_test_net", ConnectNetworkOptions {
            container: origin_container.id(),
            ..Default::default()
        })
        .await?;

    connection
        .get_client()
        .connect_network("hyperlane_validator_test_net", ConnectNetworkOptions {
            container: dest_container.id(),
            ..Default::default()
        })
        .await?;

    let origin_container_inspect = connection
        .get_client()
        .inspect_container(origin_container.id(), None)
        .await?;
    let origin_network_settings = origin_container_inspect
        .network_settings
        .unwrap()
        .networks
        .unwrap()["hyperlane_validator_test_net"]
        .clone();

    let dest_container_inspect = connection
        .get_client()
        .inspect_container(dest_container.id(), None)
        .await?;
    let dest_network_settings = dest_container_inspect
        .network_settings
        .unwrap()
        .networks
        .unwrap()["hyperlane_validator_test_net"]
        .clone();

    Ok((
        (
            origin_container,
            origin_network_settings.ip_address.unwrap(),
        ),
        (dest_container, dest_network_settings.ip_address.unwrap()),
    ))
}

//#[ignore] // TODO: Invalid signer error from relayer
#[tokio::test(flavor = "multi_thread")]
async fn validator() -> Result<()> {
    setup_log();

    let ((origin_container, origin_container_ip), (dest_container, dest_container_ip)) =
        spinup_anvil_testnets().await?;

    // The validator itself uses the IPs internal to the Docker network.
    // When it comes time to relay the message, the command is run outside the Docker network,
    // so we need to get both addresses.
    //
    // The internal address is written to `agent-config.json`.
    // The host addresses are written to `testnet{1,2}-metadata.yaml`.
    let testnet1_docker_rpc_url = format!("{}:8545", origin_container_ip);
    let testnet2_docker_rpc_url = format!("{}:8545", dest_container_ip);

    let origin_ports = origin_container.ports().await?;
    let dest_ports = dest_container.ports().await?;

    let testnet1_host_rpc_url = format!(
        "127.0.0.1:{}",
        origin_ports.map_to_host_port_ipv4(8545).unwrap()
    );
    let testnet2_host_rpc_url = format!(
        "127.0.0.1:{}",
        dest_ports.map_to_host_port_ipv4(8545).unwrap()
    );

    let tempdir = setup_temp_dir(
        (testnet1_docker_rpc_url, testnet1_host_rpc_url.clone()),
        (testnet2_docker_rpc_url, testnet2_host_rpc_url),
    );
    let temp_dir_path = tempdir.path();

    let harness = TangleTestHarness::setup().await?;

    let ctx = HyperlaneContext::new(harness.env.clone(), temp_dir_path.into()).await?;

    let handler = crate::SetConfigEventHandler::new(&harness.env, ctx)
        .await
        .map_err(|e| Report::msg(e.to_string()))?;

    let (_blueprint_id, service_id) = harness.setup_service(vec![handler]).await?;

    // Pass the arguments
    let agent_config_path = std::path::absolute(temp_dir_path.join("agent-config.json")).unwrap();
    let config_urls = to_field(Some(vec![format!(
        "file://{}",
        agent_config_path.display()
    )]))?;
    let origin_chain_name = to_field(String::from("testnet1"))?;

    // Execute job and verify result
    let results = harness
        .execute_job(service_id, 0, vec![config_urls, origin_chain_name], vec![
            OutputValue::Uint64(0),
        ])
        .await?;

    assert_eq!(results.service_id, service_id);

    // The validator is now running, send a self-relayed message
    tracing::info!("Validator running, sending message...");

    std::env::set_current_dir(temp_dir_path).expect("Failed to change directory");
    let send_msg_output = Command::new("hyperlane")
        .args([
            "send",
            "message",
            "--registry",
            ".",
            "--relay",
            "--origin",
            "testnet1",
            "--destination",
            "testnet2",
            "--quick",
        ])
        .env(
            "HYP_KEY",
            "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
        )
        .output()
        .expect("Failed to run command");

    if !send_msg_output.status.success() {
        tracing::error!("Failed to send test message");
        tracing::error!("{}", String::from_utf8_lossy(&send_msg_output.stdout));
        return Err(Report::msg("Failed to send test message"));
    }

    let stdout = String::from_utf8_lossy(&send_msg_output.stdout);

    let mut msg_id = None;
    for line in String::from_utf8_lossy(&send_msg_output.stdout).lines() {
        let Some(id) = line.strip_prefix("Message ID: ") else {
            continue;
        };

        msg_id = Some(id.to_string());
        break;
    }

    let Some(msg_id) = msg_id else {
        panic!("No message ID found in output: {stdout}")
    };

    tracing::info!("Message ID: {msg_id}");

    // Give the command a few seconds
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    tracing::info!("Mining a block");
    Command::new("cast")
        .args([
            "rpc",
            "anvil_mine",
            "1",
            "--rpc-url",
            &*testnet1_host_rpc_url,
        ])
        .output()?;

    // Give the command a few seconds
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let msg_status_output = Command::new("hyperlane")
        .args([
            "status",
            "--registry",
            ".",
            "--origin",
            "testnet1",
            "--destination",
            "testnet2",
            "--id",
            &*msg_id,
        ])
        .env(
            "HYP_KEY",
            "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
        )
        .output()
        .expect("Failed to run command");

    assert!(msg_status_output.status.success());
    assert!(
        String::from_utf8_lossy(&msg_status_output.stdout)
            .contains(&format!("Message {msg_id} was delivered"))
    );

    drop(origin_container);
    drop(dest_container);

    // Cleanup network
    let connection = DockerBuilder::new().await?;
    let network = connection
        .get_client()
        .inspect_network(
            "hyperlane_validator_test_net",
            None::<InspectNetworkOptions<String>>,
        )
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

    connection
        .remove_network("hyperlane_validator_test_net")
        .await?;

    Ok(())
}
