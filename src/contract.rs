use crate::operation::{
    Call, CallDeployCode, CallDeployErc20Token, CallDeposit, CallEvm, CallFtOnTransfer,
    CallFtTransfer, CallFtTransferCall, CallRegisterRelayer, CallStorageDeposit,
    CallStorageUnregister, CallStorageWithdraw, CallSubmit, CallWithdraw, View, ViewResultDetails,
};
use crate::{EvmCallTransaction, Result};
use aurora_engine::fungible_token::FungibleTokenMetadata;
use aurora_engine::parameters::{
    DeployErc20TokenArgs, FunctionCallArgsV2, GetStorageAtArgs, InitCallArgs, IsUsedProofCallArgs,
    NEP141FtOnTransferArgs, NewCallArgs, StorageBalance, StorageDepositCallArgs,
    StorageWithdrawCallArgs, TransactionStatus, TransferCallArgs, TransferCallCallArgs,
    ViewCallArgs,
};
use aurora_engine::proof::Proof;
use aurora_engine_types::parameters::WithdrawCallArgs;
use borsh::BorshSerialize;
#[cfg(feature = "ethabi")]
use ethabi::ParamType;
use ethereum_types::{Address, H256, U256};
use std::borrow::{Borrow, BorrowMut};
use std::marker::PhantomData;
use std::path::Path;
use std::str::FromStr;
use workspaces::network::{Betanet, Mainnet, Sandbox, Testnet};
use workspaces::types::SecretKey;
use workspaces::{Account, AccountId, Contract, Network, Worker};

// pub const AURORA_LOCAL_CHAIN_ID: u64 = 1313161556;
// pub const AURORA_ACCOUNT_ID: &str = "aurora.test.near";
// pub const OWNER_ACCOUNT_ID: &str = "owner.test.near";
// pub const PROVER_ACCOUNT_ID: &str = "prover.test.near";
// pub const EVM_CUSTODIAN_ADDRESS: &str = "096DE9C2B8A5B8c22cEe3289B101f6960d68E51E";

// lazy_static! {
//     static ref DEFAULT_AURORA_ACCOUNT_ID: AccountId =
//         AccountId::from_str("aurora.test.near").unwrap();
//     static ref DEFAULT_OWNER_ACCOUNT_ID: AccountId =
//         AccountId::from_str("owner.test.near").unwrap();
//     static ref DEFAULT_PROVER_ACCOUNT_ID: AccountId =
//         AccountId::from_str("prover.test.near").unwrap();
// }

#[derive(Debug, Clone)]
enum AccountKind {
    Account {
        contract_id: AccountId,
        inner: Account,
    },
    Contract(Contract),
}

impl AccountKind {
    fn call<'a, F: AsRef<str>>(&'a self, function: &'a F) -> EvmCallTransaction {
        let transaction = match self {
            AccountKind::Account { contract_id, inner } => {
                inner.call(contract_id, function.as_ref())
            }
            AccountKind::Contract(con) => con.call(function.as_ref()),
        };
        EvmCallTransaction::call(transaction)
    }

    async fn view<F: AsRef<str>>(
        &self,
        function: &F,
        args: Vec<u8>,
    ) -> Result<workspaces::result::ViewResultDetails> {
        Ok(match self {
            AccountKind::Account { contract_id, inner } => {
                inner.view(contract_id, function.as_ref(), args).await?
            }
            AccountKind::Contract(con) => con.view(function.as_ref(), args).await?,
        })
    }

    fn id(&self) -> &AccountId {
        match self {
            AccountKind::Account { inner, .. } => inner.id(),
            AccountKind::Contract(con) => con.id(),
        }
    }
}

// TODO(engine): Self should be able to call owner functions.
pub trait EvmSelf: EvmUser {}

pub trait EvmOwner: EvmUser {}

pub trait EvmProver: EvmUser {}

pub trait EvmUser: private::Sealed {}

pub trait EvmTester {}

#[derive(Debug, Clone)]
pub struct EvmAccount<U: EvmUser> {
    account: AccountKind,
    phantom: PhantomData<U>,
}

impl<U: EvmSelf> EvmAccount<U> {
    pub fn with_self(contract: Contract) -> EvmAccount<U> {
        Self {
            account: AccountKind::Contract(contract),
            phantom: PhantomData::default(),
        }
    }
}

impl<U: EvmOwner> EvmAccount<U> {
    pub fn with_owner(account: Account, contract_id: AccountId) -> EvmAccount<U> {
        Self {
            account: AccountKind::Account {
                contract_id,
                inner: account,
            },
            phantom: PhantomData::default(),
        }
    }
}

impl<U: EvmProver> EvmAccount<U> {
    pub fn with_prover(account: Account, contract_id: AccountId) -> EvmAccount<U> {
        Self {
            account: AccountKind::Account {
                contract_id,
                inner: account,
            },
            phantom: PhantomData::default(),
        }
    }
}

impl<U: EvmUser> EvmAccount<U> {
    pub async fn new(account: Account, contract_id: AccountId) -> EvmAccount<U> {
        Self {
            account: AccountKind::Account {
                contract_id,
                inner: account,
            },
            phantom: PhantomData::default(),
        }
    }

    fn near_call<'a, F: AsRef<str>>(&'a self, function: &'a F) -> EvmCallTransaction {
        self.account.call(function)
    }

    async fn near_view<F: AsRef<str>>(
        &self,
        function: &F,
        args: Vec<u8>,
    ) -> Result<workspaces::result::ViewResultDetails> {
        self.account.view(function, args).await
    }

    pub fn id(&self) -> &AccountId {
        self.account.id()
    }

    pub fn deploy_code(&self, code: Vec<u8>) -> CallDeployCode {
        CallDeployCode(self.near_call(&Call::DeployCode).args(code))
    }

    pub fn deploy_erc20_token<A: AsRef<str>>(&self, account_id: A) -> CallDeployErc20Token {
        // TODO: impl Error for parse account error
        let args = DeployErc20TokenArgs {
            nep141: aurora_engine_types::account_id::AccountId::new(account_id.as_ref()).unwrap(),
        };
        CallDeployErc20Token(self.near_call(&Call::DeployErc20Token).args_borsh(args))
    }

    pub fn call<A: Into<U256>>(&self, contract: Address, amount: A, input: Vec<u8>) -> CallEvm {
        let value: U256 = amount.into();
        let mut buf = [0u8; 32];
        value.to_big_endian(&mut buf);
        let args = FunctionCallArgsV2 {
            contract: aurora_engine_types::types::Address::new(contract),
            value: buf,
            input,
        };
        CallEvm(self.near_call(&Call::Evm).args_borsh(args))
    }

    pub fn submit(&self, input: Vec<u8>) -> CallSubmit {
        CallSubmit(self.near_call(&Call::Submit).args(input))
    }

    pub fn register_relayer<A: Into<Address>>(&self, address: A) -> CallRegisterRelayer {
        CallRegisterRelayer(
            self.near_call(&Call::RegisterRelayer)
                .args(address.into().0.to_vec()),
        )
    }

    pub fn ft_on_transfer<S: AsRef<str>, A: Into<u128>>(
        &self,
        sender_id: S,
        amount: A,
        message: String,
    ) -> Result<CallFtOnTransfer> {
        let sender_id = AccountId::from_str(sender_id.as_ref())?;
        let args = NEP141FtOnTransferArgs {
            // TODO: impl error
            sender_id: aurora_engine_types::account_id::AccountId::new(sender_id.as_str()).unwrap(),
            amount: aurora_engine_types::types::Balance::new(amount.into()),
            msg: message,
        };
        Ok(CallFtOnTransfer(
            self.near_call(&Call::FtOnTransfer).args_json(args),
        ))
    }

    pub fn withdraw<A: Into<Address>>(&self, receiver_address: A, amount: u128) -> CallWithdraw {
        let args = WithdrawCallArgs {
            recipient_address: aurora_engine_types::types::Address::new(receiver_address.into()),
            amount: aurora_engine_types::types::NEP141Wei::new(amount),
        };
        CallWithdraw(self.near_call(&Call::Withdraw).args_borsh(args))
    }

    pub fn deposit(&self, proof: Proof) -> CallDeposit {
        CallDeposit(self.near_call(&Call::Deposit).args_borsh(proof))
    }

    pub fn ft_transfer<R: AsRef<str>>(
        &self,
        receiver_id: R,
        amount: u128,
        memo: Option<String>,
    ) -> CallFtTransfer {
        let args = TransferCallArgs {
            // TODO: impl error
            receiver_id: aurora_engine_types::account_id::AccountId::new(receiver_id.as_ref())
                .unwrap(),
            amount: aurora_engine_types::types::NEP141Wei::new(amount),
            memo,
        };
        CallFtTransfer(self.near_call(&Call::FtTransfer).args_json(args))
    }

    pub fn ft_transfer_call<R: AsRef<str>>(
        &self,
        receiver_id: R,
        amount: u128,
        memo: Option<String>,
        message: String,
    ) -> CallFtTransferCall {
        let args = TransferCallCallArgs {
            receiver_id: aurora_engine_types::account_id::AccountId::new(receiver_id.as_ref())
                .unwrap(),
            amount: aurora_engine_types::types::NEP141Wei::new(amount),
            memo,
            msg: message,
        };
        CallFtTransferCall(self.near_call(&Call::FtTransferCall).args_json(args))
    }

    // TODO we are not NEP-145 compliant
    pub fn storage_deposit<A: AsRef<str>>(
        &self,
        account_id: Option<A>,
        registration_only: Option<bool>,
    ) -> CallStorageDeposit {
        let args = StorageDepositCallArgs {
            account_id: account_id
                .map(|a| aurora_engine_types::account_id::AccountId::new(a.as_ref()).unwrap()),
            registration_only,
        };
        CallStorageDeposit(self.near_call(&Call::StorageDeposit).args_json(args))
    }

    // TODO we are not NEP-145 compliant
    pub fn storage_unregister(&self, force: bool) -> CallStorageUnregister {
        let val = serde_json::json!({ "force": force });
        CallStorageUnregister(self.near_call(&Call::StorageUnregister).args_json(val))
    }

    // TODO we are not NEP-145 compliant
    pub fn storage_withdraw(&self, amount: Option<u128>) -> CallStorageWithdraw {
        let args = StorageWithdrawCallArgs {
            amount: amount.map(aurora_engine_types::types::Yocto::new),
        };
        CallStorageWithdraw(self.near_call(&Call::StorageWithdraw).args_json(args))
    }

    pub async fn version(&self) -> Result<ViewResultDetails<String>> {
        ViewResultDetails::try_from(self.near_view(&View::Version, vec![]).await?)
    }

    pub async fn owner(&self) -> Result<ViewResultDetails<AccountId>> {
        ViewResultDetails::try_from(self.near_view(&View::Owner, vec![]).await?)
    }

    pub async fn bridge_prover(&self) -> Result<ViewResultDetails<AccountId>> {
        ViewResultDetails::try_from(self.near_view(&View::BridgeProver, vec![]).await?)
    }

    pub async fn chain_id(&self) -> Result<ViewResultDetails<u128>> {
        Ok(ViewResultDetails::from_u256(
            self.near_view(&View::ChainId, vec![]).await?,
        ))
    }

    pub async fn upgrade_index(&self) -> Result<ViewResultDetails<u64>> {
        Ok(ViewResultDetails::from(
            self.near_view(&View::UpgradeIndex, vec![]).await?,
        ))
    }

    pub async fn paused_precompiles(&self) -> Result<ViewResultDetails<u32>> {
        Ok(ViewResultDetails::from(
            self.near_view(&View::PausedPrecompiles, vec![]).await?,
        ))
    }

    pub async fn block_hash(&self, block_height: u64) -> Result<ViewResultDetails<H256>> {
        // TODO: check if this actually needs to be borsh. Should be equivalent.
        let args = block_height.try_to_vec()?;
        Ok(ViewResultDetails::from(
            self.near_view(&View::BlockHash, args).await?,
        ))
    }

    #[cfg(not(feature = "ethabi"))]
    pub async fn code<A: Into<Address>>(&self, address: A) -> Result<ViewResultDetails<Vec<u8>>> {
        let address = aurora_engine_types::types::Address::new(address.into());
        Ok(ViewResultDetails::from(
            self.near_view(&View::Code, address.try_to_vec()?).await?,
        ))
    }

    #[cfg(feature = "ethabi")]
    pub async fn code(&self, types: &[ParamType], address: Address) -> Result<ViewCode> {
        let address = aurora_engine_types::types::Address::new(address);
        ViewCode::decode(
            types,
            self.near_view(&View::Code, address.try_to_vec()?).await?,
        )
    }

    pub async fn balance<A: Into<Address>>(&self, address: A) -> Result<ViewResultDetails<u128>> {
        Ok(ViewResultDetails::from_u256(
            self.near_view(&View::Balance, address.into().0.to_vec())
                .await?,
        ))
    }

    pub async fn nonce<A: Into<Address>>(&self, address: A) -> Result<ViewResultDetails<u128>> {
        Ok(ViewResultDetails::from_u256(
            self.near_view(&View::Nonce, address.into().0.to_vec())
                .await?,
        ))
    }

    pub async fn storage<A: Into<Address>, K: Into<H256>>(
        &self,
        address: A,
        key: K,
    ) -> Result<ViewResultDetails<H256>> {
        let args = GetStorageAtArgs {
            address: aurora_engine_types::types::Address::new(address.into()),
            key: key.into().0,
        };
        Ok(ViewResultDetails::from(
            self.near_view(&View::Storage, args.try_to_vec()?).await?,
        ))
    }

    pub async fn view<A: Into<Address>, V: Into<U256>>(
        &self,
        sender: A,
        address: A,
        amount: V,
        input: Vec<u8>,
    ) -> Result<ViewResultDetails<TransactionStatus>> {
        let mut buf = [0u8; 32];
        amount.into().to_big_endian(&mut buf);
        let args = ViewCallArgs {
            sender: aurora_engine_types::types::Address::new(sender.into()),
            address: aurora_engine_types::types::Address::new(address.into()),
            amount: buf,
            input,
        };
        ViewResultDetails::try_from(self.near_view(&View::Evm, args.try_to_vec()?).await?)
    }

    pub async fn is_proof_used(&self, proof: Proof) -> Result<ViewResultDetails<bool>> {
        let args = IsUsedProofCallArgs { proof };
        ViewResultDetails::try_from(
            self.near_view(&View::IsProofUsed, args.try_to_vec()?)
                .await?,
        )
    }

    pub async fn ft_total_supply(&self) -> Result<ViewResultDetails<u128>> {
        ViewResultDetails::try_from(self.near_view(&View::FtTotalSupply, vec![]).await?)
    }

    pub async fn ft_balance_of<A: AsRef<str>>(
        &self,
        account_id: A,
    ) -> Result<ViewResultDetails<u128>> {
        let args = serde_json::to_string(&account_id.as_ref())?;
        ViewResultDetails::try_from(
            self.near_view(&View::FtBalanceOf, args.as_bytes().to_vec())
                .await?,
        )
    }

    pub async fn ft_metadata(&self) -> Result<ViewResultDetails<FungibleTokenMetadata>> {
        ViewResultDetails::try_from(self.near_view(&View::FtMetadata, vec![]).await?)
    }

    pub async fn eth_balance_of<A: Into<Address>>(
        &self,
        address: A,
    ) -> Result<ViewResultDetails<U256>> {
        Ok(ViewResultDetails::from(
            self.near_view(&View::BalanceOfEth, address.into().0.to_vec())
                .await?,
        ))
    }

    pub async fn eth_total_supply(&self) -> Result<ViewResultDetails<U256>> {
        ViewResultDetails::try_from_json(self.near_view(&View::EthTotalSupply, vec![]).await?)
    }

    pub async fn storage_balance_of<A: AsRef<str>>(
        &self,
        account_id: A,
    ) -> Result<ViewResultDetails<StorageBalance>> {
        let args = serde_json::to_string(account_id.as_ref())?;
        ViewResultDetails::try_from(
            self.near_view(&View::StorageBalanceOf, args.as_bytes().to_vec())
                .await?,
        )
    }
}

/// A collection of sources where you can get the contract.
pub enum EvmContractSource<P: AsRef<Path>> {
    /// A path to the file containing the contract binary.
    Dir(P),
    /// Source the contract binary from NEAR testnet.
    Testnet,
    /// Source the contract binary from NEAR mainnet.
    Mainnet,
}

pub struct EthProverConfig {
    pub account_id: AccountId,
    pub evm_custodian_address: String,
}

pub struct DeployConfig {
    /// The owner ID of the contract.
    pub owner_id: AccountId,
    /// The prover ID of the contract.
    pub prover_id: AccountId,
    /// The optional configuration for the Ethereum prover (bridge).
    pub eth_prover_config: Option<EthProverConfig>,
    /// The Ethereum chain ID.
    pub chain_id: U256,
}

// TODO: Put all parameters per input, not as the struct args!
// TODO: implement a signer when a method is called, return a signer with
// TODO: builder
// information required about the transaction to be made. Then give the option
// to sign with another key, or with some default. Preferably, run `transact`.

/// A wrapper over workspaces' `Contract` type which provides ease of use when interacting with
/// the Aurora EVM contract.
///
/// ## Constructors
///
/// The contract *must* first be deployed before you can interact with it else it will return
/// errors that the contract is invalid. This can be done by invoking the `deploy_and_init` method
/// which will deploy and call `new` on the contract automatically using the provided parameters
/// from the `DeployConfig`.
///
/// If it already has been deployed and exists on the network, it is fine to construct the
/// `EvmContract` directly using the `new` method. Though the naming may be a bit confusing, this
/// follows Rust language conventions of creating a new `EvmContract` struct.
///
/// The final constructor is `from_secret_key` which will create a new `EvmContract` struct directly
/// from the provided `AccountId` and `SecretKey`. This also does not deploy the contract onto the
/// network.
///
/// ## Deployment
///
/// It should not be expected that the underlying methods return the same results as the wrapped
/// contract. Instead, it returns only the EVM result, logs, and other related meta data such as
/// data usage on both ETH and NEAR.
///
/// This type *can not* implement `Default` as the deployment may not already exist. Likewise, the
/// library does not provide a ready built EVM binary to be deployed. This must be specified.
#[derive(Debug, Clone)]
pub struct EvmContract<N: Network + 'static, U: EvmSelf> {
    contract: EvmAccount<U>,
    phantom: PhantomData<N>,
}

impl<N: Network + 'static, U: EvmSelf> AsRef<EvmAccount<U>> for EvmContract<N, U> {
    fn as_ref(&self) -> &EvmAccount<U> {
        &self.contract
    }
}

impl<N: Network + 'static, U: EvmSelf> AsMut<EvmAccount<U>> for EvmContract<N, U> {
    fn as_mut(&mut self) -> &mut EvmAccount<U> {
        &mut self.contract
    }
}

impl<N: Network + 'static, U: EvmSelf> Borrow<EvmAccount<U>> for EvmContract<N, U> {
    fn borrow(&self) -> &EvmAccount<U> {
        &self.contract
    }
}

impl<N: Network + 'static, U: EvmSelf> BorrowMut<EvmAccount<U>> for EvmContract<N, U> {
    fn borrow_mut(&mut self) -> &mut EvmAccount<U> {
        &mut self.contract
    }
}

// TODO have another PhantomData (maybe) which will note if its the public, owner, etc.
impl<N: Network + 'static, U: EvmSelf> From<Contract> for EvmContract<N, U> {
    fn from(contract: Contract) -> Self {
        EvmContract {
            contract: EvmAccount::with_self(contract),
            phantom: Default::default(),
        }
    }
}

impl<U: EvmSelf> EvmContract<Sandbox, U> {
    pub async fn deploy_and_init<P: AsRef<Path>>(
        account: Account,
        deploy_config: DeployConfig,
        source: EvmContractSource<P>,
        worker: &Worker<Sandbox>,
    ) -> Result<EvmContract<Sandbox, U>> {
        let contract = match source {
            EvmContractSource::Dir(path) => {
                let wasm = std::fs::read(path)?;
                account.deploy(&wasm).await?.into_result()?
            }
            EvmContractSource::Testnet => {
                let testnet_worker = workspaces::testnet().await?;
                let account_id = account.id();
                worker
                    .import_contract(account_id, &testnet_worker)
                    .transact()
                    .await?
            }
            EvmContractSource::Mainnet => {
                let mainnet_worker = workspaces::mainnet().await?;
                let account_id = account.id();
                worker
                    .import_contract(account_id, &mainnet_worker)
                    .transact()
                    .await?
            }
        };

        Self::deploy_and_init_inner(contract, deploy_config).await
    }
}

impl<U: EvmSelf> EvmContract<Betanet, U> {
    pub async fn deploy_and_init<P: AsRef<Path>>(
        account: Account,
        deploy_config: DeployConfig,
        path: P,
    ) -> Result<EvmContract<Betanet, U>> {
        let contract = deploy_contract(path, account).await?;
        Self::deploy_and_init_inner(contract, deploy_config).await
    }
}

impl<U: EvmSelf> EvmContract<Testnet, U> {
    pub async fn deploy_and_init<P: AsRef<Path>>(
        account: Account,
        deploy_config: DeployConfig,
        path: P,
    ) -> Result<EvmContract<Testnet, U>> {
        let contract = deploy_contract(path, account).await?;
        Self::deploy_and_init_inner(contract, deploy_config).await
    }
}

impl<U: EvmSelf> EvmContract<Mainnet, U> {
    pub async fn deploy_and_init<P: AsRef<Path>>(
        account: Account,
        deploy_config: DeployConfig,
        path: P,
    ) -> Result<EvmContract<Mainnet, U>> {
        let contract = deploy_contract(path, account).await?;
        Self::deploy_and_init_inner(contract, deploy_config).await
    }
}

impl<N: Network + 'static, U: EvmSelf> EvmContract<N, U> {
    pub async fn new<C: Into<Contract>>(contract: C) -> EvmContract<N, U> {
        EvmContract {
            contract: EvmAccount::with_self(contract.into()),
            phantom: Default::default(),
        }
    }

    async fn deploy_and_init_inner(
        contract: Contract,
        deploy_config: DeployConfig,
    ) -> Result<EvmContract<N, U>> {
        let new_args = NewCallArgs {
            chain_id: aurora_engine_types::types::u256_to_arr(&deploy_config.chain_id),
            // TODO: https://github.com/aurora-is-near/aurora-engine/issues/604, unwrap is safe here
            owner_id: aurora_engine_types::account_id::AccountId::from_str(
                deploy_config.owner_id.as_str(),
            )
            .unwrap(),
            bridge_prover_id: aurora_engine_types::account_id::AccountId::from_str(
                deploy_config.prover_id.as_str(),
            )
            .unwrap(),
            upgrade_delay_blocks: 1,
        };
        contract
            .call("new")
            .args_borsh(new_args)
            .transact()
            .await?
            .into_result()?;

        if let Some(eth_prover_config) = deploy_config.eth_prover_config {
            let new_eth_connector_args = InitCallArgs {
                prover_account: aurora_engine_types::account_id::AccountId::from_str(
                    eth_prover_config.account_id.as_str(),
                )
                .unwrap(),
                eth_custodian_address: eth_prover_config.evm_custodian_address,
                metadata: FungibleTokenMetadata::default(),
            };
            contract
                .call("new_eth_connector")
                .args_borsh(new_eth_connector_args)
                .transact()
                .await?
                .into_result()?;
        }

        Ok(EvmContract {
            contract: EvmAccount::with_self(contract),
            phantom: Default::default(),
        })
    }

    pub fn from_secret_key<D: AsRef<str>>(
        id: D,
        sk: SecretKey,
        worker: &Worker<N>,
    ) -> Result<EvmContract<N, U>> {
        let account_id = AccountId::from_str(id.as_ref())?;
        let contract = Contract::from_secret_key(account_id, sk, worker);
        Ok(EvmContract {
            contract: EvmAccount::with_self(contract),
            phantom: Default::default(),
        })
    }

    pub fn as_account(&self) -> &EvmAccount<U> {
        &self.contract
    }
}

async fn deploy_contract<P: AsRef<Path>>(path: P, account: Account) -> Result<Contract> {
    let wasm = std::fs::read(path)?;
    Ok(account.deploy(&wasm).await?.into_result()?)
}

mod private {
    pub trait Sealed {}
}
