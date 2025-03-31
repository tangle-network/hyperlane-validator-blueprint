use blueprint_sdk::alloy::primitives::Address;
use blueprint_sdk::alloy::primitives::address;
use blueprint_sdk::testing::tempfile::{self, TempDir};
use color_eyre::Result;
use std::fs;
use std::path::Path;

/// Template path for the agent configuration
pub const AGENT_CONFIG_TEMPLATE_PATH: &str = "./test_assets/agent-config.json.template";
/// Path to test assets
pub const TEST_ASSETS_PATH: &str = "./test_assets";
/// Path to testnet1 state file
pub const TESTNET1_STATE_PATH: &str = "./test_assets/testnet1-state.json";
/// Path to testnet2 state file
pub const TESTNET2_STATE_PATH: &str = "./test_assets/testnet2-state.json";

/// Default Mailbox address for tests
pub const TESTNET1_MAILBOX: Address = address!("0xB7f8BC63BbcaD18155201308C8f3540b07f84F5e");

/// Test message for cross-chain sending
pub const MESSAGE: &str = "Hello";

/// Test domain IDs
pub const ORIGIN_DOMAIN: u32 = 1337;
pub const DESTINATION_DOMAIN: u32 = 31338;

/// Sets up a temporary directory with all the necessary files for testing
///
/// # Parameters
/// * `testnet1_urls` - Tuple of (docker_rpc_url, host_rpc_url) for testnet1
/// * `testnet2_urls` - Tuple of (docker_rpc_url, host_rpc_url) for testnet2
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
        let testnet_metadata = fs::read_to_string(metadata_template_path)?;
        fs::write(
            testnet_path.join("metadata.yaml"),
            testnet_metadata.replace("{RPC_URL}", rpc_url),
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

/// Creates a new agent configuration file
///
/// # Parameters
/// * `output_path` - Path where the configuration file should be written
/// * `testnet1_rpc` - RPC URL for testnet1
/// * `testnet2_rpc` - RPC URL for testnet2
/// * `tmp_syncer_dir` - Directory path for the syncer
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
