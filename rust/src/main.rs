#![allow(unused)]
use bitcoin::hex::DisplayHex;
use bitcoincore_rpc::bitcoin::{Address, Amount, Network};
use bitcoincore_rpc::{Auth, Client, Error, RpcApi};
use serde::de::value;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, Write};

// // Node access params
const RPC_URL: &str = "http://127.0.0.1:18443"; // Default regtest RPC port
const RPC_USER: &str = "alice";
const RPC_PASS: &str = "password";

// You can use calls not provided in RPC lib API using the generic `call` function.
// An example of using the `send` RPC call, which doesn't have exposed API.
// You can also use serde_json `Deserialize` derivation to capture the returned json result.
fn send(rpc: &Client, addr: &str) -> bitcoincore_rpc::Result<String> {
    let args = [
        json!([{addr : 100 }]), // recipient address
        json!(null),            // conf target
        json!(null),            // estimate mode
        json!(null),            // fee rate in sats/vb
        json!(null),            // Empty option object
    ];

    #[derive(Deserialize)]
    struct SendResult {
        complete: bool,
        txid: String,
    }
    let send_result = rpc.call::<SendResult>("send", &args)?;
    assert!(send_result.complete);
    Ok(send_result.txid)
}

fn main() -> bitcoincore_rpc::Result<()> {
    // 1. Connect to the node
    let rpc = Client::new(
        RPC_URL,
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    // 2. Create wallets
    let existing_wallets = rpc.list_wallets()?;

    for &wallet_name in &["Minern", "Trader"] {
        if !existing_wallets.iter().any(|w| w == wallet_name) {
            match rpc.load_wallet(wallet_name) {
                Ok(_) => println!("Wallet {wallet_name} loaded successfully"),
                Err(Error::JsonRpc(bitcoincore_rpc::jsonrpc::Error::Rpc(ref rpc_error))) => {
                    // Vérifie si c’est une erreur -18 (does not exist), alors crée le wallet
                    if rpc_error.code == -18 {
                        rpc.create_wallet(wallet_name, None, None, None, None)?;
                    } else {
                        return Err(Error::JsonRpc(bitcoincore_rpc::jsonrpc::Error::Rpc(
                            rpc_error.clone(),
                        )));
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    let miner_rpc = Client::new(
        "http://127.0.0.1:18443/wallet/Miner",
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;
    let trader_rpc = Client::new(
        "http://127.0.0.1:18443/wallet/Trader",
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    let miner_balance = miner_rpc.get_balance(None, None)?;

    // 3. Miner generates an address with label "Mining Reward"
    let mining_address = miner_rpc
        .get_new_address(Some("Mining Reward"), None)?
        .require_network(Network::Regtest)
        .unwrap();
    rpc.generate_to_address(103, &mining_address)?; // Créé un UTXO de 50 BTC

    // 4. Mine until balance > 0
    let mut blocks_mined = 0;
    loop {
        let _ = rpc.generate_to_address(1, &mining_address)?;
        blocks_mined += 1;
        let bal = miner_rpc.get_balance(None, None)?;
        if bal.to_btc() > 0.0 {
            break;
        }
    }

    // Comment on why: coinbase rewards take 100 blocks to mature

    // 5. Print balance
    let miner_balance = miner_rpc.get_balance(None, None)?;

    // 6. Trader address
    let trader_address = trader_rpc
        .get_new_address(Some("Received"), None)?
        .require_network(Network::Regtest)
        .unwrap();

    // 7. Send 20 BTC from Miner to Trader
    // Wait for 100 blocks to mature
    // rpc.generate_to_address(100, &mining_address)?;
    let txid = miner_rpc.send_to_address(
        &trader_address,
        Amount::from_btc(20.0)?,
        Some("Payment"),
        Some("To Trader"),
        None,
        None,
        None,
        None,
    )?;

    // 8. Get tx from mempool
    let mempool_entry = rpc.get_mempool_entry(&txid)?;
    // println!("Mempool entry: {:?}", mempool_entry);

    // 9. Confirm transaction
    let blocks = rpc.generate_to_address(1, &mining_address)?;
    let block_hash = blocks[0];

    // 10. Fetch all details
    let tx = rpc.get_raw_transaction_info(&txid, Some(&block_hash))?;
    let mut total_input_amount = 0.0_f64;
    let mut input_addresses = vec![];

    for vin in tx.vin {
        let vin_txid = vin.txid.expect("No txid in input");
        let vout_index = vin.vout.expect("No vout");
        let input_tx = rpc.get_raw_transaction_info(&vin_txid, None)?;
        let input_vout = &input_tx.vout[vout_index as usize];
        let input_addr = input_vout
            .script_pub_key
            .address
            .as_ref()
            .map(|a| a.clone().assume_checked().to_string())
            .unwrap_or_else(|| "unknown".into());
        total_input_amount += input_vout.value.to_btc();
        input_addresses.push(input_addr);
    }

    let mut trader_output_address = String::new();
    let mut trader_output_amount = 0.0_f64;
    let mut change_address = String::new();
    let mut change_amount = 0.0_f64;

    for vout in tx.vout.iter() {
        let addr = vout
            .script_pub_key
            .address
            .as_ref()
            .map(|a| a.clone().assume_checked().to_string())
            .unwrap_or_else(|| "unknown".into());

        if addr == trader_address.to_string() {
            trader_output_address = addr;
            trader_output_amount = vout.value.to_btc();
        } else {
            change_address = addr;
            change_amount = vout.value.to_btc();
        }
    }

    let fee = mempool_entry.fees.base;
    let height = rpc.get_block_info(&block_hash)?.height;

    // 11. Write to `out.txt`
    let mut file = File::create("../test/out.txt")?;

    writeln!(file, "{txid}")?;
    writeln!(file, "{}", input_addresses[0])?;
    writeln!(file, "{total_input_amount}")?;
    writeln!(file, "{trader_output_address}")?;
    writeln!(file, "{trader_output_amount}")?;
    writeln!(file, "{change_address}")?;
    writeln!(file, "{change_amount}")?;
    writeln!(file, "{}", -fee.to_btc())?;
    writeln!(file, "{height}")?;
    writeln!(file, "{block_hash}")?;

    Ok(())
}
