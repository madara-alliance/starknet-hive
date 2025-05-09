// SPDX-License-Identifier: MIT
// Compatible with OpenZeppelin Contracts for Cairo ^0.18.0
use contracts::paymaster::interface::OutsideExecution;
#[starknet::interface]
pub trait IExecuteFromOutsideCallback<TContractState> {
    fn execute_from_outside(
        ref self: TContractState, outside_execution: OutsideExecution, signature: Array<felt252>,
    );
}

#[starknet::contract(account)]
mod ExecutableAccount {
    use contracts::paymaster::interface::{OutsideExecution, OffChainMessageOutsideExecutionRev1};
    use core::ecdsa::check_ecdsa_signature;
    use starknet::{get_tx_info, get_caller_address, get_contract_address};
    use super::{IExecuteFromOutsideCallback};
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess, Map};

    use openzeppelin::account::AccountComponent;
    use openzeppelin::introspection::src5::SRC5Component;
    use openzeppelin::upgrades::UpgradeableComponent;
    use openzeppelin::upgrades::interface::IUpgradeable;
    use starknet::{SyscallResultTrait, ClassHash, account::Call, ContractAddress};

    component!(path: AccountComponent, storage: account, event: AccountEvent);
    component!(path: SRC5Component, storage: src5, event: SRC5Event);
    component!(path: UpgradeableComponent, storage: upgradeable, event: UpgradeableEvent);

    #[abi(embed_v0)]
    impl AccountMixinImpl = AccountComponent::AccountMixinImpl<ContractState>;

    impl AccountInternalImpl = AccountComponent::InternalImpl<ContractState>;
    impl UpgradeableInternalImpl = UpgradeableComponent::InternalImpl<ContractState>;

    #[storage]
    struct Storage {
        #[substorage(v0)]
        account: AccountComponent::Storage,
        #[substorage(v0)]
        src5: SRC5Component::Storage,
        #[substorage(v0)]
        upgradeable: UpgradeableComponent::Storage,
        /// Keeps track of used nonces for outside transactions (`execute_from_outside`)
        outside_nonces: Map<felt252, bool>,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        #[flat]
        AccountEvent: AccountComponent::Event,
        #[flat]
        SRC5Event: SRC5Component::Event,
        #[flat]
        UpgradeableEvent: UpgradeableComponent::Event,
    }

    #[constructor]
    fn constructor(ref self: ContractState, public_key: felt252) {
        self.account.initializer(public_key);
    }

    #[abi(embed_v0)]
    impl UpgradeableImpl of IUpgradeable<ContractState> {
        fn upgrade(ref self: ContractState, new_class_hash: ClassHash) {
            self.account.assert_only_self();
            self.upgradeable.upgrade(new_class_hash);
        }
    }

    #[abi(embed_v0)]
    impl ExecuteFromOutsideCallback of IExecuteFromOutsideCallback<ContractState> {
        fn execute_from_outside(
            ref self: ContractState, outside_execution: OutsideExecution, signature: Array<felt252>,
        ) {
            let nonce = outside_execution.nonce;
            assert(!self.outside_nonces.read(nonce), 'oz/duplicated-outside-nonce');
            self.outside_nonces.write(nonce, true);
            let hash = outside_execution.get_message_hash_rev_1();
            self._execute_from_outside_callback(outside_execution, hash, signature);
        }
    }

    #[generate_trait]
    impl InternalFunctions of InternalFunctionsTrait {
        fn _execute_from_outside_callback(
            self: @ContractState,
            outside_execution: OutsideExecution,
            outside_tx_hash: felt252,
            signature: Array<felt252>,
        ) -> Array<Span<felt252>> {
            let validation_result = self._validate_transaction(outside_tx_hash, signature.span());
            assert(!validation_result, 'oz/validation-result');
            let execution_result = self._execute_calls(outside_execution.calls);
            execution_result
        }

        fn _execute_calls(self: @ContractState, mut calls: Span<Call>) -> Array<Span<felt252>> {
            let mut res = array![];
            loop {
                match calls.pop_front() {
                    Option::Some(call) => {
                        let _res = self._execute_single_call(call);
                        res.append(_res);
                    },
                    Option::None => { break (); }
                };
            };
            res
        }

        fn _execute_single_call(self: @ContractState, call: @Call) -> Span<felt252> {
            let Call { to, selector, calldata } = *call;
            starknet::syscalls::call_contract_syscall(to, selector, calldata).unwrap_syscall()
        }


        fn _validate_transaction(
            self: @ContractState, outside_tx_hash: felt252, signature: Span<felt252>
        ) -> bool {
            self._is_valid_signature(outside_tx_hash, signature)
        }


        fn _is_valid_signature(
            self: @ContractState, outside_tx_hash: felt252, signature: Span<felt252>
        ) -> bool {
            let public_key = self.account.Account_public_key.read();
            self._is_valid_strk_signature(outside_tx_hash, public_key, signature)
        }

        fn _is_valid_strk_signature(
            self: @ContractState,
            outside_tx_hash: felt252,
            public_key: felt252,
            signature: Span<felt252>
        ) -> bool {
            let valid_length = signature.len() == 2;
            if valid_length {
                check_ecdsa_signature(
                    outside_tx_hash, public_key, *signature.at(0_u32), *signature.at(1_u32)
                )
            } else {
                false
            }
        }
    }
}

