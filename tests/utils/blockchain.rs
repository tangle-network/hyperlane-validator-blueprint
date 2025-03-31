use blueprint_sdk::alloy::network::Ethereum;
use blueprint_sdk::alloy::network::EthereumWallet;
use blueprint_sdk::alloy::network::NetworkWallet;
use blueprint_sdk::alloy::primitives::Address;
use blueprint_sdk::alloy::primitives::B256;
use blueprint_sdk::alloy::primitives::Bytes;
use blueprint_sdk::alloy::primitives::U256;
use blueprint_sdk::alloy::providers::Provider;
use blueprint_sdk::alloy::providers::ProviderBuilder;
use blueprint_sdk::alloy::providers::RootProvider;
use blueprint_sdk::alloy::rpc::types::TransactionRequest;
use blueprint_sdk::alloy::signers::local::MnemonicBuilder;
use blueprint_sdk::alloy::signers::local::PrivateKeySigner;
use blueprint_sdk::alloy::signers::local::coins_bip39::English;
use blueprint_sdk::evm::util::get_wallet_provider_http;
use color_eyre::Result;
use std::process::Command;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Mnemonic for test wallets
pub const TEST_MNEMONIC: &str = "test test test test test test test test test test test junk";

/// Wallet derivation indices
pub enum WalletIndex {
    Validator = 0,
    Relayer = 1,
    Challenge = 2,
    Deploy = 3,
}

/// Gets a wallet based on the specified index
///
/// # Parameters
/// * `idx` - The wallet index to use
pub fn wallet_for(idx: WalletIndex) -> PrivateKeySigner {
    // Use a different HD path than the default m/44'/60'/0'/0/0 to avoid conflicts with the
    // default deployer account
    let hd_path = &format!("m/44'/60'/0'/0/{}", idx as usize);

    MnemonicBuilder::<English>::default()
        .phrase(TEST_MNEMONIC)
        .derivation_path(hd_path)
        .unwrap()
        .build()
        .unwrap()
}

/// Get a wallet and provider for a private key and RPC URL
///
/// # Parameters
/// * `key` - The private key to use (hex string)
/// * `rpc` - The RPC URL to connect to
pub fn wallet_for_key(key: &str, rpc: &str) -> (PrivateKeySigner, RootProvider) {
    let signer = PrivateKeySigner::from_str(key).unwrap();
    let wallet = EthereumWallet::new(signer.clone());
    (signer, get_wallet_provider_http(rpc, wallet.clone()))
}

/// Gets a provider for the specified RPC URL
///
/// # Parameters
/// * `rpc_url` - The RPC URL to connect to
pub fn provider(rpc_url: &str) -> RootProvider {
    ProviderBuilder::new()
        .on_http(rpc_url.parse().unwrap())
        .root()
        .clone()
}

/// Mine a block on the specified chain using cast command
///
/// # Parameters
/// * `rpc_url` - The RPC URL to connect to
/// * `n` - Number of blocks to mine (default 1)
pub async fn mine_block(rpc_url: &str, n: Option<u64>) -> Result<()> {
    let n = n.unwrap_or(1);
    let output = Command::new("cast")
        .args(["rpc", "anvil_mine", &n.to_string(), "--rpc-url", rpc_url])
        .output()?;

    if !output.status.success() {
        return Err(color_eyre::eyre::eyre!(
            "Failed to mine blocks: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // Give the chain some time to process blocks
    tokio::time::sleep(Duration::from_secs(1)).await;

    Ok(())
}

/// Increases time on the specified chain using cast command
///
/// # Parameters
/// * `rpc_url` - The RPC URL to connect to
/// * `seconds` - Number of seconds to increase time by
pub async fn increase_time(rpc_url: &str, seconds: u64) -> Result<()> {
    let output = Command::new("cast")
        .args([
            "rpc",
            "anvil_increaseTime",
            &seconds.to_string(),
            "--rpc-url",
            rpc_url,
        ])
        .output()?;

    if !output.status.success() {
        return Err(color_eyre::eyre::eyre!(
            "Failed to increase time: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // Mine a block to apply the time change
    mine_block(rpc_url, Some(1)).await?;

    Ok(())
}

/// Gets the current unix timestamp
pub fn get_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs()
}

/// Waits for a transaction to be mined
///
/// # Parameters
/// * `http_url` - The HTTP RPC URL to connect to
/// * `tx_hash` - The transaction hash to wait for
/// * `timeout` - The timeout duration
pub async fn wait_for_tx(http_url: &str, tx_hash: B256, timeout: Duration) -> Result<()> {
    let start = std::time::Instant::now();
    let provider = ProviderBuilder::new()
        .on_http(http_url.parse().unwrap())
        .root()
        .clone();

    while start.elapsed() < timeout {
        if let Ok(Some(_)) = provider.get_transaction_receipt(tx_hash).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(color_eyre::eyre::eyre!("Transaction timed out"))
}

/// Transfers ETH from one address to another
///
/// # Parameters
/// * `http_url` - The HTTP RPC URL to connect to
/// * `from` - The wallet to use as signer
/// * `to` - The address to send to
/// * `amount` - The amount to send
pub async fn transfer_eth(
    http_url: &str,
    from: &PrivateKeySigner,
    to: Address,
    amount: U256,
) -> Result<()> {
    let signer = EthereumWallet::new(from.clone());

    let tx = TransactionRequest::default()
        .to(to)
        .value(amount)
        .from(from.address())
        .build_typed_tx()
        .unwrap();

    let signed_tx =
        <EthereumWallet as NetworkWallet<Ethereum>>::sign_transaction(&signer, tx).await?;

    let provider: RootProvider<Ethereum> = ProviderBuilder::new()
        .on_http(http_url.parse().unwrap())
        .root()
        .clone();

    let receipt = provider.send_tx_envelope(signed_tx).await?;
    let tx_hash = receipt.tx_hash();

    wait_for_tx(http_url, *tx_hash, Duration::from_secs(10)).await?;
    Ok(())
}
