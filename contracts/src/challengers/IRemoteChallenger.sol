// SPDX-License-Identifier: UNLICENSE
pragma solidity >=0.8.13;

/**
 * @title IRemoteChallenger
 * @dev Interface for challenger contracts that can validate fraud proofs and trigger slashing.
 * This interface allows blueprints to interact with different challenger implementations
 * in a standardized way.
 */
interface IRemoteChallenger {
    /// @notice Handles a challenge for an operator
    /// @param serviceId The ID of the service
    /// @param operator The address of the operator
    /// @param proofData The proof data for the challenge
    function handleChallenge(uint256 serviceId, address operator, bytes calldata proofData) external;

    /// @notice Enrolls an operator for a specific service
    /// @param serviceId The service ID to enroll for
    /// @param operator The address of the operator to enroll
    /// @param publicKey Optional public key of the operator (empty bytes for no key)
    function enrollOperator(uint256 serviceId, address operator, bytes calldata publicKey) external;

    /// @notice Unenrolls an operator from a specific service
    /// @param serviceId The service ID to unenroll from
    /// @param operator The address of the operator to unenroll
    function unenrollOperator(uint256 serviceId, address operator) external;

    /// @notice Registers mailbox addresses for a specific service
    /// @param serviceId The service ID to register the mailbox for
    /// @param mailboxAddress1 The address of the mailbox
    /// @param mailboxAddress2 The address of the mailbox   
    function registerServiceMailbox(uint256 serviceId, address mailboxAddress1, address mailboxAddress2) external;
}
