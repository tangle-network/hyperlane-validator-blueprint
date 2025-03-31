//! Utility modules for testing
#![allow(dead_code)]

use blueprint_sdk::alloy::primitives::{Address, address};

pub mod blockchain;
pub mod challenger;
pub mod network;

pub const TESTNET1_MAILBOX: Address = address!("0xB7f8BC63BbcaD18155201308C8f3540b07f84F5e");
pub const MESSAGE: &str = "Hello";
pub const SLASH_PERCENTAGE: u8 = 10;
pub const ORIGIN_DOMAIN: u32 = 31337;
pub const DESTINATION_DOMAIN: u32 = 31338;
