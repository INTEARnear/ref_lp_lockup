use near_sdk::{AccountId, env, ext_contract, near, NearToken, PanicOnDefault, Gas, PromiseOrValue, PromiseError, require, assert_one_yocto, log};
use near_sdk::json_types::{U64, U128};
use near_sdk::store::TreeMap;

const STORAGE_DEPOSIT: NearToken = NearToken::from_millinear(100); // 0.1Ⓝ

#[derive(PanicOnDefault)]
#[near(contract_state)]
pub struct LockupContract {
    pool_id: U64,
    ref_address: AccountId,
    lockups: TreeMap<AccountId, Lockup>,
    audited: bool,
}

#[near(serializers = [json, borsh])]
#[derive(Clone)]
pub struct Lockup {
    amount: U128,
    duration_ns: U64,
    timestamp: U64,
}

#[ext_contract(ext_mft_token)]
pub trait MFTToken {
    #[payable]
    fn mft_transfer(
        &mut self,
        token_id: String,
        receiver_id: AccountId,
        amount: U128,
        memo: Option<String>,
    );
}

#[near]
impl LockupContract {
    #[init]
    pub fn new(pool_id: U64, ref_address: AccountId) -> Self {
        Self {
            pool_id,
            ref_address,
            lockups: TreeMap::new(b"l".to_vec()),
            audited: false,
        }
    }

    #[payable]
    pub fn register_lockup(&mut self) {
        require!(env::attached_deposit() == STORAGE_DEPOSIT, "Attached deposit is not equal to the storage cost");
        if self.lockups.contains_key(&env::predecessor_account_id()) {
            panic!("Lockup already exists");
        }
        let lockup = Lockup {
            amount: 0.into(),
            duration_ns: 0.into(),
            timestamp: env::block_timestamp().into(),
        };
        self.lockups.insert(env::predecessor_account_id(), lockup);
    }

    pub fn mft_on_transfer(
        &mut self,
        token_id: String,
        sender_id: AccountId,
        amount: U128,
        msg: String,
    ) -> PromiseOrValue<U128> {
        require!(env::predecessor_account_id() == self.ref_address, "Only the ref finance tokens are supported");
        require!(token_id == format!(":{}", self.pool_id.0));
        let duration_ns: U64 = msg.parse::<u64>().unwrap().into(); // Ref will refund the deposit if msg is not valid
        let mut lockup = Lockup {
            amount,
            duration_ns,
            timestamp: env::block_timestamp().into(),
        };
        if let Some(l) = self.lockups.get(&sender_id) {
            if l.timestamp.0 + l.duration_ns.0 <= env::block_timestamp() + duration_ns.0 {
                lockup.amount = (lockup.amount.0 + l.amount.0).into();
            } else {
                panic!("Trying to extend a lockup with new expiration date before the existing lockup expiration date.");
            }
        }
        self.lockups.insert(sender_id, lockup);

        PromiseOrValue::Value(0.into())
    }

    #[payable]
    pub fn withdraw(&mut self) {
        assert_one_yocto();
        require!(self.audited, "Withdraw function is a week point of the contract. The contract should be audited before it's enabled.");
        let lockup = self.lockups.remove(&env::predecessor_account_id()).expect("Lockup not found");
        require!(lockup.timestamp.0 + lockup.duration_ns.0 <= env::block_timestamp());
        ext_mft_token::ext(self.ref_address.clone())
            .with_attached_deposit(NearToken::from_yoctonear(1))
            .mft_transfer(
                format!(":{}", self.pool_id.0),
                env::predecessor_account_id(),
                lockup.amount,
                None,
            )
            .then(Self::ext(env::current_account_id())
                .with_static_gas(Gas::from_tgas(50))
                .withdraw_callback(env::predecessor_account_id(), lockup));
    }

    pub fn get_lockup(&self, account_id: AccountId) -> Option<Lockup> {
        self.lockups.get(&account_id).cloned()
    }

    pub fn get_lockups(&self, skip: U64, take: U64) -> Vec<(AccountId, Lockup)> {
        self.lockups.iter().skip(skip.0 as usize).take(take.0 as usize).map(|(acc, l)| (acc.clone(), l.clone())).collect::<Vec<_>>()
    }

    #[payable]
    pub fn set_audited(&mut self) {
        assert_one_yocto();
        require!(env::predecessor_account_id() == "slimedragon.near".parse::<AccountId>().unwrap());
        self.audited = true;
    }

    #[private]
    pub fn withdraw_callback(&mut self, account_id: AccountId, lockup: Lockup, #[callback_result] call_result: Result<(), PromiseError>) {
        if let Err(_) = call_result {
            log!("Withdraw failed");
            self.lockups.insert(account_id, lockup); // If someone locks liquidity during the withdrawal, they're stupid, I don't want to support that, but maybe I'll add a check to refund or merge the deposits before doing an audit.
        } else {
            log!("Withdrawn {} shares", lockup.amount.0);
        }
    }
}
