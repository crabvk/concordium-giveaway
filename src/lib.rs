use concordium_std::{collections::*, *};
use std::cmp;

#[derive(Serialize, SchemaType)]
struct Config {
    factor: u8,
    max_giveaway: Amount,
}

#[contract_state(contract = "giveaway")]
#[derive(Serialize, SchemaType)]
struct State {
    config: Config,

    // Addresses which already got a giveaway
    senders: BTreeSet<AccountAddress>,
}

#[derive(Debug, PartialEq, Eq)]
enum InitError {
    ParseParams,
    ZeroAmount,
    FactorBelowTwo,
    ZeroMaxGiveaway,
}

impl From<ParseError> for InitError {
    fn from(_: ParseError) -> Self {
        InitError::ParseParams
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ReceiveError {
    ParseParams,
    ZeroAmount,
    ZeroBalance,
    DoubleSend,
    NotOwner,
}

impl From<ParseError> for ReceiveError {
    fn from(_: ParseError) -> Self {
        ReceiveError::ParseParams
    }
}

#[init(contract = "giveaway", parameter = "Config", payable)]
fn giveaway_init(ctx: &impl HasInitContext, amount: Amount) -> Result<State, InitError> {
    ensure_ne!(amount, Amount::zero(), InitError::ZeroAmount);

    let config: Config = ctx.parameter_cursor().get()?;
    ensure!(config.factor >= 2, InitError::FactorBelowTwo);
    ensure_ne!(
        config.max_giveaway,
        Amount::zero(),
        InitError::ZeroMaxGiveaway
    );

    let state = State {
        config,
        senders: BTreeSet::new(),
    };

    Ok(state)
}

#[receive(contract = "giveaway", name = "send", payable)]
fn giveaway_send<A: HasActions>(
    ctx: &impl HasReceiveContext,
    amount: Amount,
    state: &mut State,
) -> Result<A, ReceiveError> {
    ensure_ne!(amount, Amount::zero(), ReceiveError::ZeroAmount);
    let factor = state.config.factor as u64;

    let expected_return = if amount > state.config.max_giveaway {
        amount + state.config.max_giveaway * (factor - 1)
    } else {
        amount * factor
    };

    let balance = ctx.self_balance();
    let actual_return = cmp::min(balance + amount, expected_return);
    ensure_ne!(actual_return, amount, ReceiveError::ZeroBalance);

    let invoker = ctx.invoker();
    ensure!(!state.senders.contains(&invoker), ReceiveError::DoubleSend);

    state.senders.insert(invoker);

    Ok(A::simple_transfer(&invoker, actual_return))
}

#[receive(contract = "giveaway", name = "topup", payable)]
fn giveaway_topup<A: HasActions>(
    ctx: &impl HasReceiveContext,
    _amount: Amount,
    _state: &mut State,
) -> Result<A, ReceiveError> {
    let owner = ctx.owner();
    let sender = ctx.sender();
    ensure!(sender.matches_account(&owner), ReceiveError::NotOwner);

    Ok(A::accept())
}

#[receive(contract = "giveaway", name = "abort", payable)]
fn giveaway_abort<A: HasActions>(
    ctx: &impl HasReceiveContext,
    _amount: Amount,
    _state: &mut State,
) -> Result<A, ReceiveError> {
    let invoker = ctx.invoker();
    ensure_eq!(invoker, ctx.owner(), ReceiveError::NotOwner);

    Ok(A::simple_transfer(&invoker, ctx.self_balance()))
}

#[concordium_cfg_test]
mod giveaway_tests {
    use super::*;
    use test_infrastructure::*;

    fn new_config(factor: u8, max_giveaway: u64) -> Config {
        Config {
            factor,
            max_giveaway: Amount::from_gtu(max_giveaway),
        }
    }

    #[concordium_test]
    fn test_init() {
        let config = new_config(2, 10);
        let config_bytes = to_bytes(&config);

        let mut ctx = InitContextTest::empty();
        ctx.set_parameter(&config_bytes);

        let state = giveaway_init(&ctx, Amount::from_gtu(100))
            .unwrap_or_else(|_| fail!("Contract initialization failed"));

        claim_eq!(state.config.factor, 2, "Should set factor");

        claim_eq!(
            state.config.max_giveaway,
            Amount::from_gtu(10),
            "Should set max giveaway"
        );

        claim_eq!(state.senders.len(), 0, "Should not contain senders");
    }

    #[concordium_test]
    fn test_send() {
        let account = AccountAddress([1u8; 32]);
        let config = new_config(2, 10);

        let mut ctx = ReceiveContextTest::empty();
        ctx.set_invoker(account);
        ctx.set_self_balance(Amount::from_gtu(100));

        let mut state = State {
            config,
            senders: BTreeSet::new(),
        };

        let actions: ActionsTree = giveaway_send(&ctx, Amount::from_gtu(5), &mut state)
            .unwrap_or_else(|_| fail!("Send failed"));

        claim_eq!(
            actions,
            ActionsTree::simple_transfer(&account, Amount::from_gtu(10)),
            "Send produced incorrect result"
        );

        claim_eq!(state.senders.len(), 1, "Send did not add sender");
    }

    #[concordium_test]
    fn test_double_send() {
        let account = AccountAddress([1u8; 32]);
        let config = new_config(2, 10);

        let mut ctx = ReceiveContextTest::empty();
        ctx.set_invoker(account);
        ctx.set_self_balance(Amount::from_gtu(100));

        let mut senders = BTreeSet::new();
        senders.insert(account);

        let mut state = State { config, senders };

        let result: Result<ActionsTree, ReceiveError> =
            giveaway_send(&ctx, Amount::from_gtu(5), &mut state);

        claim_eq!(
            result.err().unwrap(),
            ReceiveError::DoubleSend,
            "Expected DoubleSend error"
        );
    }

    #[concordium_test]
    fn test_send_low_balance() {
        let account = AccountAddress([1u8; 32]);
        let config = new_config(2, 10);

        let mut ctx = ReceiveContextTest::empty();
        ctx.set_invoker(account);
        ctx.set_self_balance(Amount::from_gtu(2));

        let mut state = State {
            config,
            senders: BTreeSet::new(),
        };

        let actions: ActionsTree = giveaway_send(&ctx, Amount::from_gtu(5), &mut state)
            .unwrap_or_else(|_| fail!("Send failed"));

        claim_eq!(
            actions,
            ActionsTree::simple_transfer(&account, Amount::from_gtu(7)),
            "Send produced incorrect result"
        );

        claim_eq!(state.senders.len(), 1, "Send did not add sender");
    }

    #[concordium_test]
    fn test_send_big_amount() {
        let account = AccountAddress([1u8; 32]);
        let config = new_config(3, 10);

        let mut ctx = ReceiveContextTest::empty();
        ctx.set_invoker(account);
        ctx.set_self_balance(Amount::from_gtu(100));

        let mut state = State {
            config,
            senders: BTreeSet::new(),
        };

        let actions: ActionsTree = giveaway_send(&ctx, Amount::from_gtu(17), &mut state)
            .unwrap_or_else(|_| fail!("Send failed"));

        claim_eq!(
            actions,
            ActionsTree::simple_transfer(&account, Amount::from_gtu(37)),
            "Send produced incorrect result"
        );

        claim_eq!(state.senders.len(), 1, "Send did not add sender");
    }
}
