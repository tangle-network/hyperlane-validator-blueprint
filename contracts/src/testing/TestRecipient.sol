// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.10;

import "@hyperlane-xyz/core/contracts/Mailbox.sol";
import "@hyperlane-xyz/core/contracts/interfaces/IMessageRecipient.sol";
import {IInterchainSecurityModule, ISpecifiesInterchainSecurityModule} from "@hyperlane-xyz/core/contracts/interfaces/IInterchainSecurityModule.sol";

// An echo receiver
contract TestRecipient is IMessageRecipient, ISpecifiesInterchainSecurityModule {
    event Received(uint32, bytes32, bytes);

    IInterchainSecurityModule public interchainSecurityModule;

    function handle(
        uint32 _origin,
        bytes32 _sender,
        bytes calldata _data
    ) external payable virtual override {
        emit Received(_origin, _sender, _data);
    }
}
