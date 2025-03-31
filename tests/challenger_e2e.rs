use blueprint_sdk as sdk;
use blueprint_sdk::alloy::dyn_abi::abi;
use blueprint_sdk::alloy::primitives::{B256, U256, keccak256};
use blueprint_sdk::alloy::rlp::BytesMut;
use blueprint_sdk::evm::util::get_wallet_provider_http;
use bollard::container::RemoveContainerOptions;
use bollard::models::EndpointSettings;
use bollard::network::{ConnectNetworkOptions, CreateNetworkOptions, InspectNetworkOptions};
use color_eyre::Report;
use color_eyre::Result;
use dockworker::DockerBuilder;
use futures::StreamExt;
use hyperlane_relayer_blueprint_lib;
use hyperlane_validator_blueprint_lib as blueprint;
use sdk::Job;
use sdk::alloy::network::EthereumWallet;
use sdk::alloy::primitives::{Address, Bytes, address};
use sdk::alloy::providers::{Provider, RootProvider};
use sdk::alloy::rpc::types::Filter;
use sdk::alloy::signers::local::PrivateKeySigner;
use sdk::alloy::sol;
use sdk::alloy::sol_types::SolEvent;
use sdk::crypto::sp_core::SpEcdsa;
use sdk::extract::Context;
use sdk::keystore::backends::Backend;
use sdk::runner::config::BlueprintEnvironment;
use sdk::serde::to_field;
use sdk::tangle::extract::TangleArgs2;
use sdk::tangle::layers::TangleLayer;
use sdk::testing::tempfile::{self, TempDir};
use sdk::testing::utils::anvil::start_anvil_container;
use sdk::testing::utils::setup_log;
use sdk::testing::utils::tangle::{OutputValue, TangleTestHarness};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use testcontainers::ContainerAsync;
use testcontainers::GenericImage;

// Import our utility modules
mod utils;
use utils::blockchain::{
    WalletIndex, increase_time, mine_block, provider, wallet_for, wallet_for_key,
};
use utils::challenger::{
    create_fraudulent_checkpoint_proof, create_simple_challenge_proof,
    encode_equivocation_challenger_params, encode_simple_challenger_params,
    verify_operator_enrollment,
};
use utils::network::{
    AGENT_CONFIG_TEMPLATE_PATH, TEST_ASSETS_PATH, TESTNET1_STATE_PATH, TESTNET2_STATE_PATH,
    Testnet, cleanup_networks, new_agent_config, setup_temp_dir, spinup_anvil_testnets,
    spinup_relayer,
};

const TESTNET1_MAILBOX: Address = address!("0xB7f8BC63BbcaD18155201308C8f3540b07f84F5e");
const MESSAGE: &str = "Hello";
const SLASH_PERCENTAGE: u8 = 10; // 10% slash percentage
const ORIGIN_DOMAIN: u32 = 1337; // Example domain ID for origin chain
const DESTINATION_DOMAIN: u32 = 31338; // Example domain ID for destination chain

// Define contract interfaces
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

sol!(
    #[allow(missing_docs, clippy::too_many_arguments)]
    #[sol(rpc)]
    #[derive(Debug)]
    SimpleChallenger,
    "contracts/out/SimpleChallenger.sol/SimpleChallenger.json"
);

sol!(
    #[allow(missing_docs, clippy::too_many_arguments)]
    #[sol(rpc)]
    #[derive(Debug)]
    EquivocationChallenger,
    "contracts/out/EquivocationChallenger.sol/EquivocationChallenger.json"
);

sol!(
    #[allow(missing_docs, clippy::too_many_arguments)]
    #[sol(rpc)]
    #[derive(Debug)]
    HyperlaneValidatorBlueprint,
    "contracts/out/HyperlaneValidatorBlueprint.sol/HyperlaneValidatorBlueprint.json"
);

#[tokio::test]
async fn challenger_test() -> Result<()> {
    setup_log();

    match challenger_test_inner().await {
        Ok(_) => Ok(()),
        Err(err) => {
            eprintln!("Error: {err:?}");
            let _ = cleanup_networks().await;
            Err(err)
        }
    }
}

#[tokio::test]
async fn validator_challenger_test() -> Result<()> {
    setup_log();

    match validator_challenger_test_inner().await {
        Ok(_) => Ok(()),
        Err(err) => {
            eprintln!("Error: {err:?}");
            let _ = cleanup_networks().await;
            Err(err)
        }
    }
}

async fn challenger_test_inner() -> Result<()> {
    // Spin up the testnets
    let (origin_testnet, dest_testnet) =
        spinup_anvil_testnets(TESTNET1_STATE_PATH, TESTNET2_STATE_PATH).await?;

    // The validator itself uses the IPs internal to the Docker network.
    // When it comes time to relay the message, the command is run outside the Docker network,
    // so we need to get both addresses.
    let origin_http = &origin_testnet.http;
    let dest_http = &dest_testnet.http;

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

    // Setup temporary directory
    let tempdir = setup_temp_dir(
        (testnet1_docker_rpc_url, testnet1_host_rpc_url.clone()),
        (testnet2_docker_rpc_url, testnet2_host_rpc_url.clone()),
    )?;
    let temp_dir_path = tempdir.path().to_path_buf();

    // Initialize test harness
    let harness = TangleTestHarness::setup(tempdir).await?;

    let ctx =
        blueprint::HyperlaneContext::new(harness.env().clone(), temp_dir_path.clone()).await?;
    let harness = harness.set_context(ctx);

    // Create a test environment and let tangle initialize it
    let (mut test_env, service_id, _) = harness.setup_services::<1>(false).await?;

    // Initialize the test environment (load the contracts)
    test_env.initialize().await?;

    // Add a job to set the config with challenger capability
    test_env
        .add_job(blueprint::set_config.layer(TangleLayer))
        .await;

    // Start the test environment (this will start the config job)
    test_env.start().await?;

    // Pass the arguments
    let agent_config_path = std::path::absolute(temp_dir_path.join("agent-config.json"))?;
    let config_urls = to_field(Some(vec![format!(
        "file://{}",
        agent_config_path.display()
    )]))?;
    let origin_chain_name = to_field(String::from("testnet1"))?;

    // Execute job and verify result
    let call = harness
        .submit_job(service_id, 0, vec![config_urls, origin_chain_name])
        .await?;

    let results = harness.wait_for_job_execution(0, call).await?;

    harness.verify_job(&results, vec![OutputValue::Uint64(0)]);
    assert_eq!(results.service_id, service_id);

    // Now we'll test creating a challenger by setting up a transaction that creates
    // a fraudulent checkpoint that can be challenged

    sdk::info!("Setting up challenger...");

    // Get the provider for interacting with the blockchain
    let provider = provider(&testnet1_host_rpc_url);

    // Get a wallet for creating transactions
    let deployer_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    let (deployer_signer, provider_with_wallet) =
        wallet_for_key(deployer_key, &testnet1_host_rpc_url);

    // Deploy contracts needed for testing
    sdk::info!("Deploying SimpleChallenger");
    let simple_challenger =
        SimpleChallenger::deploy(provider_with_wallet.clone(), SLASH_PERCENTAGE).await?;

    // No initialization needed for SimpleChallenger since it's done in the constructor
    sdk::info!("SimpleChallenger deployed successfully");

    sdk::info!("Deploying EquivocationChallenger");
    let equivocation_challenger = EquivocationChallenger::deploy(
        provider_with_wallet.clone(),
        SLASH_PERCENTAGE,
        ORIGIN_DOMAIN,
        TESTNET1_MAILBOX,
    )
    .await?;

    // No initialization needed for EquivocationChallenger since it's done in the constructor
    sdk::info!("EquivocationChallenger deployed successfully");

    // Enroll an operator (using the deployer as the operator for this test)
    sdk::info!("Enrolling test operator");
    let operator_address = deployer_signer.address();

    // Enroll the test operator in both challengers
    let tx = simple_challenger
        .enrollOperator(U256::from(service_id), operator_address, Bytes::default())
        .send()
        .await?;
    tx.get_receipt().await?;

    let tx = equivocation_challenger
        .enrollOperator(U256::from(service_id), operator_address, Bytes::default())
        .send()
        .await?;
    tx.get_receipt().await?;

    // Create a fraudulent checkpoint that will be used to challenge the validator
    sdk::info!("Creating fraudulent checkpoint proof for testing...");
    // Convert the key to a PrivateKeySigner for the create_fraudulent_checkpoint_proof function
    let signer = PrivateKeySigner::from_str(deployer_key).unwrap();
    let checkpoint_proof =
        create_fraudulent_checkpoint_proof(&signer, operator_address, service_id as u32, 1).await?;

    // Submit challenge to the EquivocationChallenger
    sdk::info!("Submitting equivocation challenge...");
    let tx = equivocation_challenger
        .handleChallenge(
            U256::from(service_id),
            operator_address,
            checkpoint_proof.clone(),
        )
        .send()
        .await?;
    tx.get_receipt().await?;

    // Verify that the challenge was submitted successfully
    let is_challenged = equivocation_challenger
        .isOperatorEnrolled(U256::from(service_id), operator_address)
        .call()
        .await?
        ._0;
    assert!(
        !is_challenged,
        "Operator should not be enrolled after challenge by EquivocationChallenger"
    );

    // Also create and submit a simple challenge
    sdk::info!("Creating and submitting simple challenge...");
    let simple_proof = create_simple_challenge_proof(
        operator_address,
        service_id as u32,
        Some("Test challenge - failed to perform validation duties"),
    );

    let tx = simple_challenger
        .handleChallenge(
            U256::from(service_id),
            operator_address,
            simple_proof.clone(),
        )
        .send()
        .await?;
    tx.get_receipt().await?;

    // Verify the simple challenge
    let is_challenged = simple_challenger
        .isOperatorEnrolled(U256::from(service_id), operator_address)
        .call()
        .await?
        ._0;
    assert!(
        !is_challenged,
        "Operator should not be enrolled after challenge by SimpleChallenger"
    );

    // Advance time to allow the challenge to finalize
    sdk::info!("Advancing time to finalize challenges...");
    increase_time(&testnet1_host_rpc_url, 86400).await?;

    // No need to submit challenges again as they are already finalized

    // Verify the operator has been slashed (no longer enrolled)
    let is_enrolled = verify_operator_enrollment(
        &provider,
        equivocation_challenger.address().clone(),
        operator_address,
        service_id as u32,
    )
    .await?;

    assert!(
        !is_enrolled,
        "Operator should be unenrolled after challenge is processed"
    );

    sdk::info!("Challenger test completed successfully");

    // Clean up resources
    cleanup_networks().await?;

    Ok(())
}

async fn validator_challenger_test_inner() -> Result<()> {
    // Clean up any existing networks before starting
    let _ = cleanup_networks().await;

    // Spin up the testnets
    let (origin_testnet, dest_testnet) =
        spinup_anvil_testnets(TESTNET1_STATE_PATH, TESTNET2_STATE_PATH).await?;

    // The validator itself uses the IPs internal to the Docker network.
    // When it comes time to relay the message, the command is run outside the Docker network,
    // so we need to get both addresses.
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

    let origin_http = &origin_testnet.http;
    let dest_http = &dest_testnet.http;

    // Setup temporary directory
    let tempdir = setup_temp_dir(
        (testnet1_docker_rpc_url, testnet1_host_rpc_url.clone()),
        (testnet2_docker_rpc_url, testnet2_host_rpc_url.clone()),
    )?;
    let temp_dir_path = tempdir.path().to_path_buf();

    // Initialize test harness
    let harness = TangleTestHarness::setup(tempdir).await?;

    // Create hyperlane context
    let ctx =
        blueprint::HyperlaneContext::new(harness.env().clone(), temp_dir_path.clone()).await?;
    let harness = harness.set_context(ctx);

    // Setup services
    let (mut test_env, service_id, _) = harness.setup_services::<1>(false).await?;
    test_env.initialize().await?;
    test_env
        .add_job(blueprint::set_config.layer(TangleLayer))
        .await;

    test_env.start().await?;

    // Configure the validator with challenger
    let agent_config_path = std::path::absolute(temp_dir_path.join("agent-config.json"))?;
    let config_urls = to_field(Some(vec![format!(
        "file://{}",
        agent_config_path.display()
    )]))?;
    let origin_chain_name = to_field(String::from("testnet1"))?;

    // Submit the job
    let call = harness
        .submit_job(service_id, 0, vec![config_urls, origin_chain_name])
        .await?;

    let results = harness.wait_for_job_execution(0, call).await?;
    harness.verify_job(&results, vec![OutputValue::Uint64(0)]);

    sdk::info!("Validator running, starting relayer...");
    spinup_relayer(
        &origin_testnet,
        &dest_testnet,
        harness.env().clone(),
        &temp_dir_path,
    )
    .await?;

    sdk::info!("Setting up validator challenger environment");

    // Create a deployer wallet for contract interactions
    let deployer_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    let (deployer_signer, deployer_provider) = wallet_for_key(deployer_key, &testnet1_host_rpc_url);

    // Deploy the test contracts for challenger testing
    sdk::info!("Deploying challenger contracts for testing");
    let simple_challenger =
        SimpleChallenger::deploy(deployer_provider.clone(), SLASH_PERCENTAGE).await?;

    // No initialization needed for SimpleChallenger since it's done in the constructor
    sdk::info!("SimpleChallenger deployed successfully");

    // Wait for validator to settle
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Mine a block to trigger validator activities
    mine_block(origin_http, Some(1)).await?;

    // Test enrolling an operator in the challenger
    let service_id = 1u64;
    let operator_address = deployer_signer.address();

    let tx = simple_challenger
        .enrollOperator(U256::from(service_id), operator_address, Bytes::default())
        .send()
        .await?;
    tx.get_receipt().await?;

    sdk::info!("Enrolled test operator in the challenger contract");

    // Create a simple challenge proof and submit it
    let simple_proof = create_simple_challenge_proof(
        operator_address,
        service_id as u32,
        Some("Testing validator challenger system"),
    );

    let tx = simple_challenger
        .handleChallenge(U256::from(service_id), operator_address, simple_proof)
        .send()
        .await?;
    tx.get_receipt().await?;

    // Verify the challenge was submitted successfully
    let is_enrolled = simple_challenger
        .isOperatorEnrolled(U256::from(service_id), operator_address)
        .call()
        .await?
        ._0;
    assert!(
        !is_enrolled,
        "Operator should not be enrolled after challenge"
    );

    sdk::info!("Successfully challenged operator in validator system");

    // Give validator time to react to the new block
    tokio::time::sleep(Duration::from_secs(5)).await;

    sdk::info!("Validator challenger test completed successfully");

    // Clean up
    cleanup_networks().await?;

    Ok(())
}
