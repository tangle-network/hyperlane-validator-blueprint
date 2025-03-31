use super::config::ORIGIN_DOMAIN;
use blueprint_sdk as sdk;
use blueprint_sdk::alloy::network::EthereumWallet;
use blueprint_sdk::alloy::primitives::{Address, B256, Bytes, keccak256};
use blueprint_sdk::alloy::providers::Provider;
use blueprint_sdk::alloy::rpc::types::{TransactionInput, TransactionRequest};
use blueprint_sdk::alloy::signers::Signer;
use blueprint_sdk::alloy::signers::local::PrivateKeySigner;
use blueprint_sdk::tangle::subxt_core::tx::signer;
use color_eyre::Result;

/// Encodes parameters for the EquivocationChallenger initialization
///
/// # Parameters
/// * `domain` - Domain value for the challenger
/// * `time_window` - Time window value in seconds
pub fn encode_equivocation_challenger_params(domain: u32, time_window: u32) -> Bytes {
    let mut params = Vec::new();
    params.extend_from_slice(&domain.to_be_bytes());
    params.extend_from_slice(&time_window.to_be_bytes());

    Bytes::from(params)
}

/// Encodes parameters for the SimpleChallenger initialization
///
/// # Parameters
/// * `slash_percentage` - Percentage to slash for violations (1-100)
pub fn encode_simple_challenger_params(slash_percentage: u8) -> Bytes {
    Bytes::from(vec![slash_percentage])
}

/// Creates a simple challenge proof for the SimpleChallenger
///
/// # Parameters
/// * `operator_address` - Address of the operator to challenge
/// * `service_id` - Service ID to create the proof for
/// * `reason` - Reason for the challenge (optional string)
pub fn create_simple_challenge_proof(
    operator_address: Address,
    service_id: u32,
    reason: Option<&str>,
) -> Bytes {
    let reason_bytes = match reason {
        Some(r) => r.as_bytes().to_vec(),
        None => b"Generic test violation".to_vec(),
    };

    // Create a simple proof by concatenating the operator address, service ID, and reason
    let mut proof = Vec::new();
    proof.extend_from_slice(operator_address.as_slice());
    proof.extend_from_slice(&service_id.to_be_bytes());
    proof.extend_from_slice(&reason_bytes);

    Bytes::from(proof)
}

/// Creates a fraudulent checkpoint proof for testing the EquivocationChallenger
/// This is a simplified version that might need adaptation for the actual contract
///
/// # Parameters
/// * `wallet` - The wallet to sign with
/// * `operator_address` - Address of the operator to challenge
/// * `service_id` - Service ID for the challenge
/// * `index` - Index of the checkpoint
pub async fn create_fraudulent_checkpoint_proof(
    signer: &PrivateKeySigner,
    operator_address: Address,
    service_id: u32,
    index: u32,
) -> Result<Bytes> {
    // Create two different root values for the same index
    let root1 = B256::new([0; 32]);
    let root2 = B256::new([1; 32]);

    // Create two checkpoints with the same index but different roots
    let checkpoint1 = (operator_address, ORIGIN_DOMAIN, root1, index);
    let checkpoint2 = (operator_address, ORIGIN_DOMAIN, root2, index);

    // Concatenate data for first checkpoint and sign it
    let mut checkpoint1_data = Vec::new();
    checkpoint1_data.extend_from_slice(checkpoint1.0.as_slice());
    checkpoint1_data.extend_from_slice(&checkpoint1.1.to_be_bytes());
    checkpoint1_data.extend_from_slice(checkpoint1.2.as_slice());
    checkpoint1_data.extend_from_slice(&checkpoint1.3.to_be_bytes());

    let hash1 = keccak256(&checkpoint1_data);
    let signature1 = signer.sign_hash(&hash1).await?;

    // Concatenate data for second checkpoint and sign it
    let mut checkpoint2_data = Vec::new();
    checkpoint2_data.extend_from_slice(checkpoint2.0.as_slice());
    checkpoint2_data.extend_from_slice(&checkpoint2.1.to_be_bytes());
    checkpoint2_data.extend_from_slice(checkpoint2.2.as_slice());
    checkpoint2_data.extend_from_slice(&checkpoint2.3.to_be_bytes());

    let hash2 = keccak256(&checkpoint2_data);
    let signature2 = signer.sign_hash(&hash2).await?;

    // Combine everything into a proof
    let mut proof = Vec::new();
    proof.extend_from_slice(checkpoint1.0.as_slice());
    proof.extend_from_slice(&checkpoint1.1.to_be_bytes());
    proof.extend_from_slice(checkpoint1.2.as_slice());
    proof.extend_from_slice(&checkpoint1.3.to_be_bytes());
    proof.extend_from_slice(&signature1.as_bytes().to_vec());
    proof.extend_from_slice(checkpoint2.0.as_slice());
    proof.extend_from_slice(&checkpoint2.1.to_be_bytes());
    proof.extend_from_slice(checkpoint2.2.as_slice());
    proof.extend_from_slice(&checkpoint2.3.to_be_bytes());
    proof.extend_from_slice(&signature2.as_bytes().to_vec());
    proof.extend_from_slice(&service_id.to_be_bytes());

    Ok(Bytes::from(proof))
}

/// Verifies if an operator is enrolled in a challenger
///
/// # Parameters
/// * `provider` - The provider to use
/// * `challenger_address` - Address of the challenger contract
/// * `operator_address` - Address of the operator to check
/// * `service_id` - Service ID to check enrollment for
pub async fn verify_operator_enrollment(
    provider: &impl Provider,
    challenger_address: Address,
    operator_address: Address,
    service_id: u32,
) -> Result<bool> {
    // Function selector for isOperatorEnrolled(address,uint256)
    let is_operator_enrolled_selector = [0x58, 0x47, 0x60, 0x26];

    // Encode the function call data
    let mut data = Vec::from(is_operator_enrolled_selector);

    // Pad address to 32 bytes
    data.extend_from_slice(&[0; 12]);
    data.extend_from_slice(operator_address.as_slice());

    // Encode service ID as uint256
    let mut service_id_bytes = [0u8; 32];
    service_id_bytes[28..32].copy_from_slice(&service_id.to_be_bytes());
    data.extend_from_slice(&service_id_bytes);

    // Create transaction request
    let tx_request = TransactionRequest::default()
        .to(challenger_address)
        .input(Bytes::from(data).into());

    // Call contract view function
    let result = provider.call(&tx_request).await?;

    // Parse result - true is represented as 1 in the last byte
    let is_enrolled = !result.is_empty() && result[result.len() - 1] == 1;
    Ok(is_enrolled)
}
