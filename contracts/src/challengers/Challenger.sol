// SPDX-License-Identifier: UNLICENSE
pragma solidity >=0.8.13;

import "./IRemoteChallenger.sol";
import "../SlashingPrecompile.sol";
import "tnt-core/Permissions.sol";
import "@openzeppelin/contracts/access/Ownable.sol";
import "@openzeppelin/contracts/security/ReentrancyGuard.sol";
import "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";

/**
 * @title Challenger
 * @dev Base contract for challenger implementations that handle fraud proofs and slashing.
 * A single challenger can serve multiple service instances with proper isolation.
 */
abstract contract Challenger is IRemoteChallenger, Ownable, ReentrancyGuard, RootChainEnabled {
    using ECDSA for bytes32;

    /// @notice Default slashing percentage for new services
    uint8 public defaultSlashPercentage;

    /// @notice Maps serviceId → operator → enrollment status
    mapping(uint256 => mapping(address => bool)) public operatorEnrollment;

    /// @notice Maps serviceId → operator → public key
    mapping(uint256 => mapping(address => bytes)) public operatorPublicKeys;

    /// @notice Maps serviceId → mailbox addresses
    mapping(uint256 => address[2]) public serviceMailboxes;

    /// @notice Emitted when an operator is enrolled for a service
    event OperatorEnrolled(uint256 indexed serviceId, address indexed operator, bytes publicKey);

    /// @notice Emitted when an operator is unenrolled from a service
    event OperatorUnenrolled(uint256 indexed serviceId, address indexed operator);

    /// @notice Emitted when a challenge is submitted and processed
    event ChallengeProcessed(uint256 indexed serviceId, address indexed operator, address indexed challenger, uint8 slashPercentage);

    /// @notice Emitted when a mailbox is registered for a service
    event ServiceMailboxRegistered(uint256 indexed serviceId, address indexed mailboxAddress1, address indexed mailboxAddress2);

    /**
     * @dev Constructor to initialize the challenger contract
     * @param _defaultSlashPercentage The default slashing percentage (0-100) for new services
     */
    constructor(uint8 _defaultSlashPercentage) Ownable() {
        require(_defaultSlashPercentage > 0 && _defaultSlashPercentage <= 100, "Challenger: slash percentage must be between 1 and 100");
        defaultSlashPercentage = _defaultSlashPercentage;
    }

    /**
     * @notice Enrolls an operator for a specific service
     * @param serviceId The service ID to enroll for
     * @param operator The address of the operator to enroll
     * @param publicKey Optional public key of the operator (empty bytes for no key)
     */
    function enrollOperator(uint256 serviceId, address operator, bytes calldata publicKey) external {
        require(operator != address(0), "Challenger: operator cannot be zero address");
        require(!operatorEnrollment[serviceId][operator], "Challenger: operator already enrolled for this service");

        operatorEnrollment[serviceId][operator] = true;

        // Store public key if provided
        if (publicKey.length > 0) {
            // Verify that the address derived from the public key matches the operator address
            address derivedAddress = address(uint160(uint256(keccak256(publicKey))));
            require(derivedAddress == operator, "Challenger: public key does not match operator address");

            operatorPublicKeys[serviceId][operator] = publicKey;
        }

        emit OperatorEnrolled(serviceId, operator, publicKey);
    }

    /**
     * @notice Unenrolls an operator from a specific service
     * @param serviceId The service ID to unenroll from
     * @param operator The address of the operator to unenroll
     */
    function unenrollOperator(uint256 serviceId, address operator) external onlyOwner {
        require(operator != address(0), "Challenger: operator cannot be zero address");
        require(operatorEnrollment[serviceId][operator], "Challenger: operator not enrolled for this service");

        operatorEnrollment[serviceId][operator] = false;
        delete operatorPublicKeys[serviceId][operator];

        emit OperatorUnenrolled(serviceId, operator);
    }

    /**
     * @notice Validates a proof and slashes an operator if the proof is valid
     * @param serviceId The service ID the challenge is for
     * @param operator The address of the operator to challenge
     * @param proofData The proof data for the challenge (implementation specific)
     */
    function handleChallenge(uint256 serviceId, address operator, bytes calldata proofData) external nonReentrant {
        require(operatorEnrollment[serviceId][operator], "Challenger: operator not enrolled for this service");

        // Validate the proof
        require(_validateProof(serviceId, operator, proofData), "Challenger: invalid proof");

        // Encode operator address for the precompile
        bytes memory offender = abi.encodePacked(operator);

        // Call the SlashingPrecompile to slash the operator
        // The runtime will handle any delay logic needed
        SLASHING_PRECOMPILE.slash(offender, serviceId, defaultSlashPercentage);

        emit ChallengeProcessed(serviceId, operator, msg.sender, defaultSlashPercentage);
    }

    /**
     * @notice Registers mailbox addresses for a specific service
     * @param serviceId The service ID to register the mailbox for
     * @param mailboxAddress1 The address of the mailbox
     * @param mailboxAddress2 The address of the mailbox   
     */
    function registerServiceMailbox(uint256 serviceId, address mailboxAddress1, address mailboxAddress2) external onlyFromRootChain {
        require(mailboxAddress1 != address(0) && mailboxAddress2 != address(0), "Challenger: mailbox addresses cannot be zero addresses");
        require(mailboxAddress1 != mailboxAddress2, "Challenger: mailbox addresses cannot be the same");

        serviceMailboxes[serviceId] = [mailboxAddress1, mailboxAddress2];
        emit ServiceMailboxRegistered(serviceId, mailboxAddress1, mailboxAddress2);
    }

    /**
     * @notice Check if an operator is enrolled for a specific service
     * @param serviceId The service ID to check
     * @param operator The address of the operator
     * @return enrolled Whether the operator is enrolled
     */
    function isOperatorEnrolled(uint256 serviceId, address operator) external view returns (bool) {
        return operatorEnrollment[serviceId][operator];
    }

    /**
     * @notice Get the public key of an operator for a specific service
     * @param serviceId The service ID to check
     * @param operator The address of the operator
     * @return publicKey The operator's public key (empty if not set)
     */
    function getOperatorPublicKey(uint256 serviceId, address operator) external view returns (bytes memory) {
        return operatorPublicKeys[serviceId][operator];
    }

    /**
     * @notice Validates a proof of misbehavior - to be implemented by derived contracts
     * @param serviceId The service ID the challenge is for
     * @param operator The address of the operator
     * @param proofData The proof data for the challenge
     * @return valid Whether the proof is valid
     */
    function _validateProof(uint256 serviceId, address operator, bytes calldata proofData) internal virtual returns (bool);
} 