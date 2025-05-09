use blockifier::{execution::call_info::CallInfo, transaction::objects::TransactionExecutionInfo};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize, Serializer};
use starknet_api::block::BlockNumber;
use starknet_devnet_types::{
    contract_address::ContractAddress,
    emitted_event::{Event, OrderedEvent},
    felt::{BlockHash, Felt, TransactionHash},
    messaging::{MessageToL1, MessageToL2, OrderedMessageToL1},
    rpc::{
        transaction_receipt::{DeployTransactionReceipt, FeeAmount, FeeInUnits, TransactionReceipt},
        transactions::{
            DeclareTransaction, DeployAccountTransaction, InvokeTransaction, Transaction, TransactionTrace,
            TransactionType, TransactionWithHash,
        },
    },
};
use starknet_rs_core::{
    types::{ExecutionResult, TransactionFinalityStatus},
    utils::get_selector_from_name,
};

use super::{
    constants::UDC_CONTRACT_ADDRESS,
    errors::{DevnetResult, Error},
    traits::{HashIdentified, HashIdentifiedMut},
};

#[derive(Debug, Default, Deserialize)]
pub struct StarknetTransactions(IndexMap<TransactionHash, StarknetTransaction>);

impl Serialize for StarknetTransactions {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let inner_vec: Vec<&TransactionWithHash> = self.0.values().map(|transaction| &transaction.inner).collect();
        inner_vec.serialize(serializer)
    }
}

impl StarknetTransactions {
    pub fn insert(&mut self, transaction_hash: &TransactionHash, transaction: StarknetTransaction) {
        self.0.insert(*transaction_hash, transaction);
    }

    pub fn get(&self, transaction_hash: &TransactionHash) -> Option<&StarknetTransaction> {
        self.0.get(transaction_hash)
    }

    pub fn iter(&self) -> indexmap::map::Iter<'_, Felt, StarknetTransaction> {
        self.0.iter()
    }
}

impl HashIdentifiedMut for StarknetTransactions {
    type Hash = TransactionHash;
    type Element = StarknetTransaction;
    fn get_by_hash_mut(&mut self, hash: &Self::Hash) -> Option<&mut StarknetTransaction> {
        self.0.get_mut(hash)
    }
}

impl HashIdentified for StarknetTransactions {
    type Hash = TransactionHash;
    type Element = StarknetTransaction;
    fn get_by_hash(&self, hash: Self::Hash) -> Option<&StarknetTransaction> {
        self.0.get(&hash)
    }
}

#[allow(unused)]
#[derive(Debug, Serialize, Deserialize)]
pub struct StarknetTransaction {
    pub inner: TransactionWithHash,
    #[serde(skip_serializing)]
    pub(crate) finality_status: TransactionFinalityStatus,
    #[serde(skip_serializing)]
    pub(crate) execution_result: ExecutionResult,
    #[serde(skip_serializing)]
    pub(crate) block_hash: Option<BlockHash>,
    #[serde(skip_serializing)]
    pub(crate) block_number: Option<BlockNumber>,
    #[serde(skip)]
    pub(crate) execution_info: TransactionExecutionInfo,
    #[serde(skip)]
    pub(crate) trace: Option<TransactionTrace>,
}

impl StarknetTransaction {
    pub fn create_accepted(
        transaction: &TransactionWithHash,
        execution_info: TransactionExecutionInfo,
        trace: TransactionTrace,
    ) -> Self {
        Self {
            finality_status: TransactionFinalityStatus::AcceptedOnL2,
            execution_result: match execution_info.is_reverted() {
                true => ExecutionResult::Reverted {
                    reason: execution_info.revert_error.clone().unwrap_or("No revert error".to_string()),
                },
                false => ExecutionResult::Succeeded,
            },
            inner: transaction.clone(),
            block_hash: None,
            block_number: None,
            execution_info,
            trace: Some(trace),
        }
    }

    pub fn get_events(&self) -> Vec<Event> {
        let mut events: Vec<Event> = vec![];

        fn get_blockifier_events_recursively(
            call_info: &blockifier::execution::call_info::CallInfo,
        ) -> Vec<(OrderedEvent, ContractAddress)> {
            let mut events: Vec<(OrderedEvent, ContractAddress)> = vec![];

            events.extend(
                call_info
                    .execution
                    .events
                    .iter()
                    .map(|e| (OrderedEvent::from(e), call_info.call.storage_address.into())),
            );

            call_info.inner_calls.iter().for_each(|call| {
                events.extend(get_blockifier_events_recursively(call));
            });

            events
        }

        let call_infos = vec![
            self.execution_info.validate_call_info.as_ref(),
            self.execution_info.execute_call_info.as_ref(),
            self.execution_info.fee_transfer_call_info.as_ref(),
        ];

        for inner_call_info in call_infos.into_iter().flatten() {
            let mut not_sorted_events = get_blockifier_events_recursively(inner_call_info);
            not_sorted_events.sort_by_key(|(ordered_event, _)| ordered_event.order);
            events.extend(not_sorted_events.into_iter().map(|(ordered_event, address)| Event {
                from_address: address,
                keys: ordered_event.keys,
                data: ordered_event.data,
            }));
        }

        events
    }

    /// Scans through events and gets information from Event generated from UDC with specific
    /// ContractDeployed. Returns the contract address
    ///
    /// # Arguments
    /// * `events` - The events that will be searched
    pub fn get_deployed_address_from_events(events: &[Event]) -> DevnetResult<Option<ContractAddress>> {
        let contract_deployed_event_key =
            Felt::from(get_selector_from_name("ContractDeployed").map_err(|_| Error::FormatError)?);

        let udc_address = ContractAddress::new(Felt::from_prefixed_hex_str(UDC_CONTRACT_ADDRESS)?)?;

        let deployed_address = events
            .iter()
            .find(|e| e.from_address == udc_address && e.keys.contains(&contract_deployed_event_key))
            .map(|e| e.data.first().cloned().unwrap_or_default());

        Ok(if let Some(contract_address) = deployed_address {
            Some(ContractAddress::new(contract_address)?)
        } else {
            None
        })
    }

    pub fn get_receipt(&self) -> DevnetResult<TransactionReceipt> {
        let transaction_events = self.get_events();

        let transaction_messages = self.get_l2_to_l1_messages();

        // decide what units to set for actual fee.
        // L1 Handler transactions are in WEI
        // V3 transactions are in STRK(FRI)
        // Other transactions versions are in ETH(WEI)
        let fee_amount = FeeAmount { amount: self.execution_info.actual_fee };
        let actual_fee_in_units = match self.inner.transaction {
            Transaction::L1Handler(_) => FeeInUnits::WEI(fee_amount),
            Transaction::Declare(DeclareTransaction::V3(_))
            | Transaction::DeployAccount(DeployAccountTransaction::V3(_))
            | Transaction::Invoke(InvokeTransaction::V3(_)) => FeeInUnits::FRI(fee_amount),
            _ => FeeInUnits::WEI(fee_amount),
        };

        let mut common_receipt = self.inner.create_common_receipt(
            &transaction_events,
            &transaction_messages,
            self.block_hash.as_ref(),
            self.block_number,
            &self.execution_result,
            self.finality_status,
            actual_fee_in_units,
            &self.execution_info,
        );

        match &self.inner.transaction {
            Transaction::DeployAccount(deploy_account_txn) => {
                Ok(TransactionReceipt::Deploy(DeployTransactionReceipt {
                    common: common_receipt,
                    contract_address: *deploy_account_txn.get_contract_address(),
                }))
            }
            Transaction::Invoke(_) => {
                let deployed_address = StarknetTransaction::get_deployed_address_from_events(&transaction_events)?;

                let receipt = if let Some(contract_address) = deployed_address {
                    common_receipt.r#type = TransactionType::Deploy;
                    TransactionReceipt::Deploy(DeployTransactionReceipt { common: common_receipt, contract_address })
                } else {
                    TransactionReceipt::Common(common_receipt)
                };

                Ok(receipt)
            }
            Transaction::L1Handler(l1_transaction) => {
                let msg_hash = MessageToL2::try_from(l1_transaction)?.hash()?;
                Ok(TransactionReceipt::L1Handler(
                    starknet_devnet_types::rpc::transaction_receipt::L1HandlerTransactionReceipt {
                        common: common_receipt,
                        message_hash: msg_hash,
                    },
                ))
            }
            _ => Ok(TransactionReceipt::Common(common_receipt)),
        }
    }

    pub fn get_trace(&self) -> Option<TransactionTrace> {
        self.trace.clone()
    }

    pub fn get_l2_to_l1_messages(&self) -> Vec<MessageToL1> {
        let mut messages = vec![];

        fn get_blockifier_messages_recursively(call_info: &CallInfo) -> Vec<OrderedMessageToL1> {
            let mut messages = vec![];

            // Ensure we always take the address of the contract that is sending the message.
            // In the case of a library syscall, storage address will automatically refer to the
            // caller address.
            let from_address = call_info.call.storage_address;

            messages.extend(call_info.execution.l2_to_l1_messages.iter().map(|m| OrderedMessageToL1 {
                order: m.order,
                message: MessageToL1 {
                    to_address: m.message.to_address.into(),
                    from_address: from_address.into(),
                    payload: m.message.payload.0.iter().map(|p| (*p).into()).collect(),
                },
            }));

            call_info.inner_calls.iter().for_each(|inner_call| {
                messages.extend(get_blockifier_messages_recursively(inner_call));
            });

            messages
        }

        let call_infos = self.execution_info.non_optional_call_infos();

        for inner_call_info in call_infos {
            let mut not_sorted_messages = get_blockifier_messages_recursively(inner_call_info);
            not_sorted_messages.sort_by_key(|message| message.order);
            messages.extend(not_sorted_messages.into_iter().map(|m| m.message));
        }

        messages
    }
}
