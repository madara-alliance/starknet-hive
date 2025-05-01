use crate::{
    assert_eq_result, assert_matches_result,
    utils::v7::{
        accounts::{
            account::{Account, ConnectedAccount},
            call::Call,
            deployment::helpers::get_contract_address,
            factory::{open_zeppelin::OpenZeppelinAccountFactory, AccountFactory},
        },
        endpoints::{
            errors::OpenRpcTestGenError,
            utils::{get_selector_from_name, wait_for_sent_transaction},
        },
        providers::provider::Provider,
        signers::{key_pair::SigningKey, local_wallet::LocalWallet, signer::Signer},
    },
    RandomizableAccountsTrait, RunnableTrait,
};
use starknet_types_core::felt::Felt;
use starknet_types_rpc::{BlockId, BlockTag, DeployAccountTxnReceipt, TxnReceipt};

const DEFAULT_ACCOUNT_CLASS_HASH: Felt =
    Felt::from_hex_unchecked("0x07dc7899aa655b0aae51eadff6d801a58e97dd99cf4666ee59e704249e51adf2");

#[derive(Clone, Debug)]
pub struct TestCase {}

impl RunnableTrait for TestCase {
    type Input = super::TestSuiteKatana;
    async fn run(test_input: &Self::Input) -> Result<Self, OpenRpcTestGenError> {
        let funding_account = test_input.random_paymaster_account.random_accounts()?;

        let provider = test_input.random_paymaster_account.random_accounts()?.provider().clone();
        let chain_id = provider.chain_id().await?;

        // Precompute the contract address of the new account with the given parameters:
        let signer = LocalWallet::from(SigningKey::from_random());
        let class_hash = DEFAULT_ACCOUNT_CLASS_HASH;
        let salt = Felt::from_hex_unchecked("0x123");
        let ctor_args = [signer.get_public_key().await?.scalar()];
        let computed_address = get_contract_address(salt, class_hash, &ctor_args, Felt::ZERO);

        // send enough tokens to the new_account's address just to send the deploy account tx
        let amount = Felt::from_hex_unchecked("0x1ba32524a30000");
        let recipient = computed_address;

        let transfer_execution = funding_account
            .execute_v1(vec![Call {
                to: Felt::from_hex("0x49D36570D4E46F48E99674BD3FCC84644DDD6B96F7C741B1562B82F9E004DC7")?,
                selector: get_selector_from_name("transfer")?,
                calldata: vec![recipient, amount, Felt::ZERO],
            }])
            .send()
            .await?;

        wait_for_sent_transaction(transfer_execution.transaction_hash, &funding_account).await?;

        let factory = OpenZeppelinAccountFactory::new(class_hash, chain_id, &signer, &provider).await?;
        let res = factory.deploy_v1(salt).send().await?;
        // the contract address in the send tx result must be the same as the computed one
        assert_eq_result!(res.contract_address, computed_address);

        wait_for_sent_transaction(res.transaction_hash, &funding_account).await?;

        let receipt = provider.get_transaction_receipt(res.transaction_hash).await?;

        assert_matches_result!(
            receipt,
            TxnReceipt::DeployAccount(DeployAccountTxnReceipt { contract_address, .. })  => {
                // the contract address in the receipt must be the same as the computed one
                assert_eq_result!(contract_address, computed_address)
            }
        );

        // Verify the `getClassHashAt` returns the same class hash that we use for the account
        // deployment
        let res = provider.get_class_hash_at(BlockId::Tag(BlockTag::Pending), computed_address).await?;
        assert_eq_result!(res, class_hash);

        Ok(Self {})
    }
}
