use blueprint_sdk as sdk;
use color_eyre::Report;
use color_eyre::Result;
use futures::StreamExt;
use hyperlane_validator_blueprint_lib as blueprint;
use sdk::Job;
use sdk::alloy::primitives::Bytes;
use sdk::alloy::providers::Provider;
use sdk::alloy::rpc::types::Filter;
use sdk::alloy::sol;
use sdk::alloy::sol_types::SolEvent;
use sdk::serde::to_field;
use sdk::tangle::layers::TangleLayer;
use sdk::testing::utils::setup_log;
use sdk::testing::utils::tangle::{OutputValue, TangleTestHarness};
use utils::DESTINATION_DOMAIN;
use utils::MESSAGE;
use utils::TESTNET1_MAILBOX;

use std::time::Duration;

// Import our utility modules
mod utils;
use utils::blockchain::{mine_block, wallet_for_key};
use utils::network::{
    TESTNET1_STATE_PATH, TESTNET2_STATE_PATH, cleanup_networks, setup_temp_dir,
    spinup_anvil_testnets, spinup_relayer,
};

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

#[tokio::test]
#[serial_test::serial]
async fn validator_test() -> Result<()> {
    setup_log();

    match validator_test_inner().await {
        Ok(_) => Ok(()),
        Err(err) => {
            eprintln!("Error: {err:?}");
            let _ = cleanup_networks().await;
            Err(err)
        }
    }
}

async fn validator_test_inner() -> Result<()> {
    let (origin_testnet, dest_testnet) =
        spinup_anvil_testnets(TESTNET1_STATE_PATH, TESTNET2_STATE_PATH).await?;

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

    let ctx =
        blueprint::HyperlaneContext::new(harness.env().clone(), temp_dir_path.clone()).await?;
    let harness = harness.set_context(ctx);

    let (mut test_env, service_id, _) = harness.setup_services::<1>(false).await?;
    test_env.initialize().await?;
    test_env
        .add_job(blueprint::set_config.layer(TangleLayer))
        .await;

    test_env.start().await?;

    let agent_config_path = std::path::absolute(temp_dir_path.join("agent-config.json"))?;
    let config_urls = to_field(Some(vec![format!(
        "file://{}",
        agent_config_path.display()
    )]))?;
    let origin_chain_name = to_field(String::from("testnet1"))?;

    let call = harness
        .submit_job(service_id, 0, vec![config_urls, origin_chain_name])
        .await?;

    let results = harness.wait_for_job_execution(0, call).await?;

    harness.verify_job(&results, vec![OutputValue::Uint64(0)]);
    assert_eq!(results.service_id, service_id);

    sdk::info!("Validator running, starting relayer...");
    spinup_relayer(
        &origin_testnet,
        &dest_testnet,
        harness.env().clone(),
        &temp_dir_path,
    )
    .await?;

    sdk::info!("Relayer running, sending message...");
    std::env::set_current_dir(temp_dir_path)?;

    sdk::info!("Getting Testnet1's mailbox");
    let (_testnet1_wallet, testnet1_provider) = wallet_for_key(
        &hex::encode(harness.alloy_key.to_bytes()),
        &testnet1_host_rpc_url,
    );
    let testnet1_mailbox = Mailbox::new(TESTNET1_MAILBOX, testnet1_provider.clone());

    let (_testnet2_wallet, testnet2_provider) = wallet_for_key(
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        &testnet2_host_rpc_url,
    );

    sdk::info!("Deploying recipient");
    let recipient = TestRecipient::deploy(testnet2_provider.clone()).await?;

    sdk::info!(
        "Dispatching message `{MESSAGE:?}` to recipient `{}`",
        recipient.address(),
    );
    let tx = testnet1_mailbox
        .dispatch_2(
            DESTINATION_DOMAIN,
            recipient.address().into_word(),
            Bytes::from(MESSAGE),
        )
        .send()
        .await?;
    let receipt = tx.get_receipt().await?;
    if !receipt.status() {
        sdk::error!("Failed to dispatch message");
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
    sdk::info!("Message ID: {message_id}");

    mine_block(&testnet1_host_rpc_url, Some(1)).await?;

    let received_event_filter = Filter::new()
        .address(*recipient.address())
        .event("Received(uint32,bytes32,bytes)")
        .select(0..);

    let mut stream = testnet2_provider
        .watch_logs(&received_event_filter)
        .await?
        .into_stream();

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

                sdk::info!(
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

    cleanup_networks().await?;

    Ok(())
}
