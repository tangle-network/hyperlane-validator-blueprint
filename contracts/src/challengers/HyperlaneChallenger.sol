// SPDX-License-Identifier: UNLICENSE
pragma solidity >=0.8.13;

import "./Challenger.sol";

/**
 * @title EquivocationChallenger
 * @dev A Hyperlane-specific implementation of the challenger interface.
 * This contract validates Hyperlane-specific fraud proofs by checking for double-signed
 * checkpoints with conflicting roots but the same index.
 */
contract EquivocationChallenger is Challenger {
    using ECDSA for bytes32;

    /// @notice The domain where the merkle root signatures are validated
    uint32 public immutable originDomain;
    
    /// @notice The address of the merkle tree contract on the origin domain
    address public immutable merkleTreeHook;

    /// @notice Emitted when a double signature challenge is validated
    event DoubleSignatureProofValidated(
        uint256 indexed serviceId,
        address indexed operator, 
        address indexed challenger, 
        bytes32 conflictRoot1,
        bytes32 conflictRoot2,
        uint32 index
    );

    /**
     * @notice Structure representing a Hyperlane checkpoint
     * @param merkleTreeHookAddress The address of the merkle tree hook
     * @param mailboxDomain The domain of the mailbox
     * @param root The root of the merkle tree
     * @param index The index of the checkpoint
     */
    struct Checkpoint {
        address merkleTreeHookAddress;
        uint32 mailboxDomain;
        bytes32 root;
        uint32 index;
    }

    /**
     * @dev Constructor to initialize the challenger contract
     * @param _slashPercentage The default slashing percentage (0-100)
     * @param _originDomain The domain ID where the merkle tree is located
     * @param _merkleTreeHook The address of the merkle tree hook contract
     */
    constructor(
        uint8 _slashPercentage,
        uint32 _originDomain,
        address _merkleTreeHook
    ) Challenger(_slashPercentage) {
        require(_merkleTreeHook != address(0), "HyperlaneChallenger: merkle tree hook cannot be zero address");
        
        originDomain = _originDomain;
        merkleTreeHook = _merkleTreeHook;
    }

    /**
     * @notice Validates a proof of misbehavior for Hyperlane validators
     * @dev The proof data must be ABI-encoded as (Checkpoint, bytes, Checkpoint, bytes)
     *      representing (checkpoint1, signature1, checkpoint2, signature2)
     * @param serviceId The service ID the challenge is for
     * @param operator The address of the operator
     * @param proofData The encoded proof data containing two checkpoints and signatures
     * @return valid Whether the proof is valid
     */
    function _validateProof(uint256 serviceId, address operator, bytes calldata proofData) internal override returns (bool) {
        // Ensure the operator is enrolled and has a public key registered
        if (!operatorEnrollment[serviceId][operator]) {
            return false;
        }
        
        bytes memory pubKey = operatorPublicKeys[serviceId][operator];
        if (pubKey.length == 0) {
            return false;
        }
        
        // Decode the proof data
        if (proofData.length < 64) { // Minimum size check
            return false;
        }

        try this.decodeProofData(proofData) returns (
            Checkpoint memory checkpoint1,
            bytes memory signature1,
            Checkpoint memory checkpoint2,
            bytes memory signature2
        ) {
            // Verify both checkpoints are for the same domain and merkle tree hook
            if (checkpoint1.mailboxDomain != originDomain || 
                checkpoint2.mailboxDomain != originDomain) {
                return false;
            }
            
            if (checkpoint1.merkleTreeHookAddress != merkleTreeHook || 
                checkpoint2.merkleTreeHookAddress != merkleTreeHook) {
                return false;
            }
            
            // Verify both checkpoints have the same index but different roots
            if (checkpoint1.index != checkpoint2.index || 
                checkpoint1.root == checkpoint2.root) {
                return false;
            }
            
            // Verify the signatures match the operator's public key
            if (!verifySignature(checkpoint1, signature1, pubKey) || 
                !verifySignature(checkpoint2, signature2, pubKey)) {
                return false;
            }
            
            // If we get here, the proof is valid - emit the event
            emit DoubleSignatureProofValidated(
                serviceId,
                operator,
                msg.sender,
                checkpoint1.root,
                checkpoint2.root,
                checkpoint1.index
            );
            
            return true;
        } catch {
            return false;
        }
    }

    /**
     * @notice External function to decode proof data for use in _validateProof
     * @dev This is in a separate function to allow for try/catch in _validateProof
     * @param proofData The encoded proof data
     * @return checkpoint1 The first checkpoint
     * @return signature1 The signature for the first checkpoint
     * @return checkpoint2 The second checkpoint
     * @return signature2 The signature for the second checkpoint
     */
    function decodeProofData(bytes calldata proofData) external pure returns (
        Checkpoint memory checkpoint1,
        bytes memory signature1,
        Checkpoint memory checkpoint2,
        bytes memory signature2
    ) {
        (checkpoint1, signature1, checkpoint2, signature2) = abi.decode(
            proofData,
            (Checkpoint, bytes, Checkpoint, bytes)
        );
        return (checkpoint1, signature1, checkpoint2, signature2);
    }

    /**
     * @notice Verifies a signature for a checkpoint against a public key
     * @param checkpoint The checkpoint data
     * @param signature The signature to verify
     * @param publicKey The public key to verify against
     * @return valid Whether the signature is valid
     */
    function verifySignature(
        Checkpoint memory checkpoint,
        bytes memory signature,
        bytes memory publicKey
    ) internal pure returns (bool) {
        bytes32 checkpointHash = keccak256(
            abi.encodePacked(
                checkpoint.merkleTreeHookAddress,
                checkpoint.mailboxDomain,
                checkpoint.root,
                checkpoint.index
            )
        );
        
        // For ECDSA signatures (Ethereum style)
        bytes32 prefixedHash = checkpointHash.toEthSignedMessageHash();
        address recoveredSigner = ECDSA.recover(prefixedHash, signature);
        
        // Derive the address from the public key and compare
        address derivedAddress = address(
            uint160(uint256(keccak256(publicKey)))
        );
        
        return recoveredSigner == derivedAddress;
    }
} 