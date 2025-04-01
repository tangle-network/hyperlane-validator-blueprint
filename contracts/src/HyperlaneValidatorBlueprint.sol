// SPDX-License-Identifier: UNLICENSE
pragma solidity >=0.8.13;

import "./ChallengerEnrollment.sol";

/**
 * @title HyperlaneValidatorBlueprint
 * @dev This contract is a blueprint for a Hyperlane validator deployment.
 * It supports integration with challenger contracts for verifiable fraud proofs and slashing.
 * The blueprint itself acts as the service manager in Tangle's architecture.
 */
contract HyperlaneValidatorBlueprint is ChallengerEnrollment {
    struct HyperlaneRequestInputs {
        uint32 originDomain;
        uint32 destinationDomain;
        address[] challengers;
    }

    // Structure to store domain information for services
    struct ServiceDomains {
        uint32 originDomain;
        uint32 destinationDomain;
        address owner;
        uint64 ttl;
        address[] permittedCallers;
    }

    // Structure to store approved operator information
    struct OperatorApproval {
        address operator;
        bytes publicKey;
    }

    // Mapping of serviceId to domain information
    mapping(uint256 => ServiceDomains) public serviceDomains;

    // Mapping of requestId to request inputs
    mapping(uint64 => HyperlaneRequestInputs) private _pendingRequests;

    // Mapping of requestId to approved operators
    mapping(uint64 => OperatorApproval[]) private _approvedOperators;

    // Mapping of serviceId to operators
    mapping(uint256 => OperatorApproval[]) private _operators;

    // Event emitted when a service instance is created
    event ServiceInstanceCreated(
        uint256 indexed serviceId,
        address[] challengers,
        uint32 originDomain,
        address owner,
        address[] permittedCallers,
        uint64 ttl
    );

    // Event emitted when an operator approves a service request
    event OperatorApproved(
        uint64 indexed requestId,
        address indexed operator,
        bytes publicKey
    );

    // Event emitted when an operator rejects a service request
    event OperatorRejected(
        uint64 indexed requestId,
        address indexed operator
    );

    // Modifier to ensure the caller is a permitted caller
    modifier onlyPermittedCaller(uint256 serviceId) {
        // Check if the caller is in the permitted caller list
        for (uint256 i = 0; i < serviceDomains[serviceId].permittedCallers.length; i++) {
            if (serviceDomains[serviceId].permittedCallers[i] == msg.sender) {
                return;
            }
        }
        _;
    }

    /**
     * @notice Handles incoming service requests
     * @dev Stores request parameters for later processing during service initialization
     * @param params The request parameters
     */
    function onRequest(ServiceOperators.RequestParams calldata params) override external payable virtual onlyFromMaster {
        // Decode and store the blueprint-specific request inputs
        HyperlaneRequestInputs memory blueprintInputs = abi.decode(params.requestInputs, (HyperlaneRequestInputs));

        // Store request inputs for processing during service initialization
        _pendingRequests[params.requestId] = blueprintInputs;
    }

    /**
     * @notice Called when an operator approves a service request
     * @dev Tracks which operators have approved each request
     * @param operator The operator preferences data
     * @param requestId The request ID being approved
     * @param restakingPercent The percentage of stake being restaked
     */
    function onApprove(
        ServiceOperators.OperatorPreferences calldata operator,
        uint64 requestId,
        uint8 restakingPercent
    ) external payable override onlyFromMaster {
        // Store the approval
        address operatorAddress = _operatorAddressFromPublicKey(operator.ecdsaPublicKey);
        _approvedOperators[requestId].push(OperatorApproval({
            operator: operatorAddress,
            publicKey: operator.ecdsaPublicKey
        }));

        emit OperatorApproved(requestId, operatorAddress, operator.ecdsaPublicKey);
    }

    /**
     * @notice Called when a service is initialized with a specific service ID
     * @dev Processes the stored request parameters and sets up the service
     * @param requestId The original request ID
     * @param serviceId The assigned service ID
     * @param owner The owner address of the service
     * @param permittedCallers List of addresses allowed to call the service
     * @param ttl Time-to-live for the service in seconds
     */
    function onServiceInitialized(
        uint64 requestId,
        uint64 serviceId,
        address owner,
        address[] calldata permittedCallers,
        uint64 ttl
    ) override external onlyFromMaster {
        // Get the stored request inputs
        HyperlaneRequestInputs memory inputs = _pendingRequests[requestId];
        require(inputs.originDomain > 0, "HVB: no pending request found");

        // Process challengers
        for (uint256 i = 0; i < inputs.challengers.length; i++) {
            address challenger = inputs.challengers[i];

            // Validate the challenger is registered
            require(isRegisteredChallenger(challenger), "HVB: invalid challenger");

            // Register the challenger for this service instance
            _registerChallengerForService(serviceId, challenger);
        }

        // Enroll all approved operators in all challengers for this service
        OperatorApproval[] memory approvals = _approvedOperators[requestId];
        for (uint256 i = 0; i < approvals.length; i++) {
            address operator = approvals[i].operator;
            bytes memory publicKey = approvals[i].publicKey;

            // Add the operator to the service
            _operators[serviceId].push(approvals[i]);

            // Enroll this operator in all registered challengers for this service
            for (uint256 j = 0; j < inputs.challengers.length; j++) {
                IRemoteChallenger(inputs.challengers[j]).enrollOperator(serviceId, operator, publicKey);
            }
        }

        // Store domain information and service parameters
        serviceDomains[serviceId] = ServiceDomains({
            originDomain: inputs.originDomain,
            destinationDomain: inputs.destinationDomain,
            owner: owner,
            permittedCallers: permittedCallers,
            ttl: ttl
        });

        // Clean up the pending request and approvals
        delete _pendingRequests[requestId];
        delete _approvedOperators[requestId];

        // Emit event for service instance creation
        emit ServiceInstanceCreated(
            serviceId,
            inputs.challengers,
            inputs.originDomain,
            owner,
            permittedCallers,
            ttl
        );
    }

    function addChallenger(uint256 serviceId, address challenger) external onlyPermittedCaller(serviceId) {
        _registerChallengerForService(serviceId, challenger);
        for (uint256 i = 0; i < _operators[serviceId].length; i++) {
            IRemoteChallenger(challenger).enrollOperator(serviceId, _operators[serviceId][i].operator, _operators[serviceId][i].publicKey);
        }
    }

    /**
     * @notice Implementation of abstract function from ChallengerEnrollment
     * @param publicKey The public key to convert
     * @return The operator address derived from the public key
     */
    function _operatorAddressFromPublicKey(bytes memory publicKey) internal pure override returns (address) {
        return address(uint160(uint256(keccak256(publicKey))));
    }
}
