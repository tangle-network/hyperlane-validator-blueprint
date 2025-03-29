// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;

/// @dev The SlashingPrecompile contract's address (it's a reduced version of the Services precompile contract)
address constant SLASHING_PRECOMPILE_ADDRESS = 0x0000000000000000000000000000000000000900;

/// @dev The Services contract's instance.
SlashingPrecompile constant SLASHING_PRECOMPILE = SlashingPrecompile(SLASHING_PRECOMPILE_ADDRESS);

/// @title Pallet Services Interface
/// @dev The interface through which solidity contracts will interact with Services pallet
/// We follow this same interface including four-byte function selectors, in the precompile that
/// wraps the pallet
/// @custom:address 0x0000000000000000000000000000000000000900
interface SlashingPrecompile {
    /// @dev Slash an operator for a service.
    /// @param offender The offender in SCALE-encoded format.
    /// @param service_id The service ID.
    /// @param percent The slash percentage (0-100).
    /// @custom:selector 64a798ac
    function slash(bytes calldata offender, uint256 service_id, uint8 percent) external;

    /// @dev Dispute an unapplied slash.
    /// @param era The era number.
    /// @param index The index of the slash.
    /// @custom:selector fac9efa3
    function dispute(uint32 era, uint32 index) external;
}