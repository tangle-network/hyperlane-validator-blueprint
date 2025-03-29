# Hyperlane Validator Blueprint

A modular slashing mechanism for Hyperlane validators on Tangle Network.

## Architecture Overview

This system enables economic security for Hyperlane's cross-chain messaging by implementing provable fraud conditions with corresponding slashing penalties.

## ELI5: How It All Works

### What are Challengers?

Challengers are specialized contracts that check if validators misbehaved. Think of them like security cameras with built-in fraud detection.

### The Complete Lifecycle:

1. **Operators Register:** Validators register for the Hyperlane Blueprint on Tangle.

2. **Customer Requests Instance:** A customer wants Hyperlane between Ethereum and Arbitrum. This creates instance #1 with service ID 1.

3. **Customer Selects Challengers:** During the service request, the customer chooses which challenger contracts will monitor their validator set. This allows customers to select their own security model.

4. **Operators Automatically Enrolled:** Selected validators are automatically enrolled in the customer's chosen challengers for that specific service instance.

5. **Validators Work:** Validators run their Hyperlane software, signing message roots that help messages go between Ethereum and Arbitrum.

6. **Something Goes Wrong:** Validator Bob signs two different message roots for the same checkpoint - signing two contradictory statements.

7. **Someone Reports It:** Charlie (could be another validator, a watchtower, or anyone) notices Bob's double-signing. Charlie submits both signatures to the HyperlaneChallenger contract, saying "Bob cheated on instance #1."

8. **Verification & Delay:** The challenger contract checks if the proof is valid (did Bob really sign contradictory statements?). If yes, it records the challenge but waits for a delay period.

9. **Slashing Execution:** After the delay, anyone can call the challenger to actually execute the slash. The challenger tells the Tangle system "Slash Bob's stake for service ID 1."

10. **Funds Slashed:** Tangle takes the specified percentage of Bob's staked funds for that service.

### Core Components

#### Blueprint Pattern

A blueprint represents a service template that can be instantiated multiple times:

- A single blueprint contract supports many service instances
- Each service instance has its own unique service ID
- The SlashingPrecompile uses service IDs to identify which stake to slash

#### HyperlaneValidatorBlueprint

Acts as the central coordination point for the Hyperlane validator service. It:

- Manages challenger enrollment/unenrollment for operators
- Serves multiple service instances, each with their own validator set
- Coordinates with the Tangle runtime for permissions
- Processes service requests including customer's challenger selections

#### Challengers

Reusable contracts that verify specific types of validator fraud:

- **SimpleChallenger**: Basic implementation for arbitrary slashing conditions
- **HyperlaneChallenger**: Specialized for Hyperlane-specific fraud proofs (e.g., double-signed checkpoints)

A single challenger can serve multiple service instances, allowing for efficient reuse of verification logic.

### Service Request Flow

When a customer wants to create a Hyperlane validator set:

1. **Prepare Request Inputs:** The customer specifies:

   - `challengers`: Array of challenger contract addresses for their security model
   - `slashPercentages`: Optional customized slashing percentages for each challenger
   - `originDomain`: The Hyperlane domain ID where the service will run
   - `destinationDomains`: The Hyperlane domain IDs the service will connect to

2. **Submit Request:** The customer submits their request with selected operators and the above parameters.

3. **Blueprint Processing:** The blueprint:

   - Registers the specified challengers for the service instance
   - Configures custom slashing percentages if provided
   - Stores domain information
   - Automatically enrolls assigned operators in all selected challengers
   - Returns a unique service ID to track the instance

4. **Validators Begin Work:** The enrolled operators can now perform validator tasks for the customer's instance.

### Security Model

The system follows a trust-but-verify approach:

- Validators operate freely until proven fraudulent
- Watchtowers monitor for fraud without permission
- Economic incentives align all actors to maintain system security
- Service-specific slashing protects unrelated stakes
- **Customer Choice:** Customers select their own security model by choosing challengers

### Customer-Selected Security

This architecture empowers customers to:

1. **Choose Security Level:** Select challengers with different verification methods
2. **Customize Slashing:** Set custom slashing percentages for different types of fraud
3. **Balance Risk:** Select the right challengers for their specific use case requirements

For example, a high-value DeFi application might select multiple stringent challengers with high slashing percentages, while a gaming application might select more lenient verification.

## Usage Flow

1. Deploy the blueprint once (managed by Tangle governance)
2. Create multiple service instances, each with a unique service ID
3. Customers select challengers during service request
4. Validators are automatically enrolled with proper service IDs
5. If fraud occurs, watchtowers submit proof to challengers
6. Slashing is executed only against the relevant service ID

This architecture provides flexibility while maintaining strong security guarantees across all Hyperlane validator deployments.
