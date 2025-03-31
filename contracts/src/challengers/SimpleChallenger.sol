// SPDX-License-Identifier: UNLICENSE
pragma solidity >=0.8.13;

import "./Challenger.sol";

/**
 * @title SimpleChallenger
 * @dev A basic implementation of the challenger interface that allows for handling slashing conditions.
 * It works with the Hyperlane validator blueprint and can be used to report and action provable slashing conditions.
 * A single challenger can be used for multiple service instances.
 */
contract SimpleChallenger is Challenger {
    /**
     * @dev Constructor to initialize the SimpleChallenger contract
     * @param _slashPercentage The default slashing percentage (0-100)
     */
    constructor(
        uint8 _slashPercentage
    ) Challenger(_slashPercentage) {}

    /**
     * @notice Validates a proof of misbehavior
     * @param operator The address of the operator
     * @param proofData The proof data for the challenge
     * @return valid Whether the proof is valid
     */
    function _validateProof(uint256 serviceId, address operator, bytes calldata proofData) internal override pure returns (bool) {
        // This is a placeholder for actual validation logic
        // In a real implementation, this would analyze the proof data to verify misbehavior
        
        // Example: Check if the proof is a signed message admitting fault
        // bytes32 messageHash = keccak256(abi.encodePacked("I misbehaved", operator));
        // address signer = messageHash.toEthSignedMessageHash().recover(proofData);
        // return signer == operator;
        
        // For this simple implementation, we'll assume all proofs with non-zero length are valid
        return proofData.length > 0;
    }
} 