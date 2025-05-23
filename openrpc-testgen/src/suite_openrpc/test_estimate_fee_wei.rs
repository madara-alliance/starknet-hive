use std::{path::PathBuf, str::FromStr, sync::Arc};

use starknet_types_core::felt::Felt;
use starknet_types_rpc::PriceUnit;

use crate::{
    assert_result,
    utils::v7::{
        accounts::account::Account,
        endpoints::{declare_contract::get_compiled_contract, errors::OpenRpcTestGenError},
    },
    RunnableTrait,
};

const BLOB_GAS_PRICE: Felt = Felt::from_hex_unchecked("0x1");
const GAS_PRICE: Felt = Felt::from_hex_unchecked("0x1");

#[derive(Clone, Debug)]
pub struct TestCase {}

impl RunnableTrait for TestCase {
    type Input = super::TestSuiteOpenRpc;

    async fn run(test_input: &Self::Input) -> Result<Self, OpenRpcTestGenError> {
        let (flattened_sierra_class, compiled_class_hash) = get_compiled_contract(
            PathBuf::from_str("target/dev/contracts_contracts_smpl14_HelloStarknet.contract_class.json")?,
            PathBuf::from_str("target/dev/contracts_contracts_smpl14_HelloStarknet.compiled_contract_class.json")?,
        )
        .await?;

        let estimate_fee = test_input
            .random_paymaster_account
            .declare_v2(Arc::new(flattened_sierra_class), compiled_class_hash)
            .estimate_fee()
            .await?;

        assert_result!(
            estimate_fee.unit == PriceUnit::Wei,
            format!("Estimate fee unit expected: {:?}, actual: {:?}", PriceUnit::Wei, estimate_fee.unit)
        );

        assert_result!(
            estimate_fee.gas_price == GAS_PRICE,
            format!("Estimate fee gas price expected: {:?}, actual: {:?}", GAS_PRICE, estimate_fee.gas_price)
        );

        assert_result!(
            estimate_fee.data_gas_price == BLOB_GAS_PRICE,
            format!(
                "Estimate fee data gas price expected: {:?}, actual: {:?}",
                BLOB_GAS_PRICE, estimate_fee.data_gas_price
            )
        );

        let data_fee = estimate_fee.data_gas_consumed * estimate_fee.data_gas_price;

        let fee = estimate_fee.gas_consumed * estimate_fee.gas_price;

        let overall_fee = data_fee + fee;

        assert_result!(
            overall_fee == estimate_fee.overall_fee,
            format!("Estimate fee overall fee expected: {:?}, actual: {:?}", overall_fee, estimate_fee.overall_fee)
        );

        Ok(Self {})
    }
}
