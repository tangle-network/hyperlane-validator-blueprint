// SPDX-License-Identifier: UNLICENSE
pragma solidity >=0.8.13;

import "@openzeppelin/contracts/access/Ownable.sol";
import "tnt-core/BlueprintServiceManagerBase.sol";
import "./challengers/IRemoteChallenger.sol";

/**
 * @title ChallengerEnrollment
 * @dev Base contract providing functionality for managing challenger enrollments.
 * This contract provides mechanisms for registering challengers and tracking service-specific challengers.
 */
abstract contract ChallengerEnrollment is BlueprintServiceManagerBase, Ownable {
    /// @notice Maps challenger address to whether it's registered
    mapping(address => bool) public registeredChallengers;
    
    /// @notice Maps service ID to array of challenger addresses
    mapping(uint256 => address[]) private _serviceChallengers;
    
    /// @notice Emitted when a new challenger is registered
    event ChallengerRegistered(address indexed challenger);
    
    /// @notice Emitted when a challenger is unregistered
    event ChallengerUnregistered(address indexed challenger);
    
    /// @notice Emitted when a challenger is registered for a service
    event ChallengerRegisteredForService(uint256 indexed serviceId, address indexed challenger);

    /**
     * @notice Registers a new challenger contract.
     * @param challenger The address of the challenger contract.
     */
    function registerChallenger(address challenger) external onlyOwner {
        require(challenger != address(0), "CE: challenger cannot be zero address");
        require(!registeredChallengers[challenger], "CE: challenger already registered");
        
        registeredChallengers[challenger] = true;
        
        emit ChallengerRegistered(challenger);
    }
    
    /**
     * @notice Unregisters a challenger contract.
     * @param challenger The address of the challenger contract.
     */
    function unregisterChallenger(address challenger) external onlyOwner {
        require(registeredChallengers[challenger], "CE: challenger not registered");
        registeredChallengers[challenger] = false;
        emit ChallengerUnregistered(challenger);
    }

    /**
     * @notice Registers a challenger for a specific service instance.
     * @param serviceId The service ID.
     * @param challenger The challenger address.
     */
    function _registerChallengerForService(uint256 serviceId, address challenger) internal {
        require(registeredChallengers[challenger], "CE: challenger not registered");
        
        // Check if already registered (prevent duplicates)
        address[] storage challengers = _serviceChallengers[serviceId];
        for (uint256 i = 0; i < challengers.length; i++) {
            if (challengers[i] == challenger) return;
        }
        
        // Add to the service challengers
        _serviceChallengers[serviceId].push(challenger);
        emit ChallengerRegisteredForService(serviceId, challenger);
    }

    /**
     * @notice Gets all challengers registered for a specific service instance.
     * @param serviceId The service ID.
     * @return Array of challenger addresses for the service.
     */
    function getServiceChallengers(uint256 serviceId) public view returns (address[] memory) {
        return _serviceChallengers[serviceId];
    }

    /**
     * @notice Checks if a challenger is registered.
     * @param challenger The challenger address to check.
     * @return Whether the challenger is registered.
     */
    function isRegisteredChallenger(address challenger) public view returns (bool) {
        return registeredChallengers[challenger];
    }
    
    /**
     * @notice Helper function to convert a public key to an operator address.
     * @param publicKey The public key to convert.
     * @return operator The operator address.
     */
    function _operatorAddressFromPublicKey(bytes memory publicKey) internal pure virtual returns (address);
} 