#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, token, Address, Env};

fn setup(env: &Env) -> (LiquidityPoolContractClient, Address, Address, Address) {
    let admin = Address::generate(env);
    let token_a = env.register_stellar_asset_contract(admin.clone());
    let token_b = env.register_stellar_asset_contract(admin.clone());
    let contract_id = env.register(LiquidityPoolContract, (&admin, &token_a, &token_b));
    let client = LiquidityPoolContractClient::new(env, &contract_id);
    (client, admin, token_a, token_b)
}

#[test]
fn test_initialize() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _token_a, _token_b) = setup(&env);
    let stats = client.get_stats();
    assert_eq!(stats.reserve_a, 0);
    assert_eq!(stats.reserve_b, 0);
    assert_eq!(stats.total_shares, 0);
}

#[test]
fn test_add_liquidity() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, token_a, token_b) = setup(&env);
    let provider = Address::generate(&env);
    token::StellarAssetClient::new(&env, &token_a).mint(&provider, &10_000);
    token::StellarAssetClient::new(&env, &token_b).mint(&provider, &10_000);
    let shares = client.add_liquidity(&provider, &10_000, &10_000);
    assert!(shares > 0);
}

#[test]
fn test_swap() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, token_a, token_b) = setup(&env);
    let provider = Address::generate(&env);
    token::StellarAssetClient::new(&env, &token_a).mint(&provider, &10_000);
    token::StellarAssetClient::new(&env, &token_b).mint(&provider, &10_000);
    client.add_liquidity(&provider, &10_000, &10_000);
    let trader = Address::generate(&env);
    token::StellarAssetClient::new(&env, &token_a).mint(&trader, &10_000);
    let out = client.swap(&trader, &10_000, &1, &true);
    assert!(out > 0);
}

#[test]
fn test_swap_slippage_protection() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, token_a, token_b) = setup(&env);
    let provider = Address::generate(&env);
    token::StellarAssetClient::new(&env, &token_a).mint(&provider, &10_000);
    token::StellarAssetClient::new(&env, &token_b).mint(&provider, &10_000);
    client.add_liquidity(&provider, &10_000, &10_000);
    let trader = Address::generate(&env);
    token::StellarAssetClient::new(&env, &token_a).mint(&trader, &10_000);
    let result = client.try_swap(&trader, &10_000, &999_999, &true);
    assert!(result.is_err());
}
