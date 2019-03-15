use hex;
use std::fmt;

use bip39::{Language, Mnemonic, Seed};
use bitcoin::{consensus::serialize, Network as BNetwork, PrivateKey, Transaction};
use bitcoin_hashes::hex::{FromHex, ToHex};
use bitcoin_hashes::sha256d::Hash as Sha256dHash;
use bitcoincore_rpc::{Client as RpcClient, Error as CoreError, RpcApi};
use failure::Error;
use serde_json::Value;

use crate::errors::OptionExt;

const SAT_PER_BTC: f64 = 100_000_000.0;
const SAT_PER_MBTC: f64 = 100_000.0;
const SAT_PER_BIT: f64 = 100.0;
const PER_PAGE: u32 = 10;

pub struct Wallet {
    rpc: &'static RpcClient,
}

impl Wallet {
    pub fn new(rpc: &'static RpcClient) -> Self {
        Wallet { rpc }
    }

    pub fn register(&self, mnemonic: &String) -> Result<(), Error> {
        let mnem = Mnemonic::from_phrase(&mnemonic[..], Language::English)?;
        let seed = Seed::new(&mnem, "");

        // TODO seed -> secret key conversion
        let skey = secp256k1::SecretKey::from_slice(&seed.as_bytes()[0..32]).unwrap();

        // TODO network
        let bkey = PrivateKey {
            compressed: false,
            network: BNetwork::Testnet,
            key: skey,
        };
        let wif = bkey.to_wif();

        // XXX this operation is destructive and would replace any prior seed stored in bitcoin core
        // TODO make sure the wallet is unused before doing this!
        let args = [json!(true), json!(wif)];
        let res: Result<Value, CoreError> = self.rpc.call("sethdseed", &args);

        match res {
            Ok(_) => Ok(()),
            // https://github.com/apoelstra/rust-jsonrpc/pull/16
            Err(CoreError::JsonRpc(jsonrpc::error::Error::NoErrorOrResult)) => Ok(()),
            Err(CoreError::JsonRpc(jsonrpc::error::Error::Rpc(rpc_error))) => {
                if rpc_error.code != -5
                    || rpc_error.message
                        != "Already have this key (either as an HD seed or as a loose private key)"
                {
                    bail!("{:?}", rpc_error)
                }
                Ok(())
            }
            Err(err) => bail!(err),
        }
    }

    pub fn login(&self, mnemonic: &String) -> Result<(), Error> {
        // just as pass-through to register for now
        self.register(mnemonic)
    }

    pub fn get_account(&self) -> Result<Value, Error> {
        let balance: f64 = self.rpc.call("getbalance", &[])?;
        let balance = btc_to_sat(balance);
        let balance_f = balance as f64;
        let exchange_rate = 420.0; // TODO

        Ok(json!({
            "type": "core",
            "pointer": 0,
            "receiving_id": "",
            "name": "RPC wallet",
            "has_transactions": true, // TODO

            "satoshi": balance.to_string(),
            "bits": (balance_f / SAT_PER_BIT).to_string(),
            "ubts": (balance_f / SAT_PER_BIT).to_string(),
            "mbtc": (balance_f / SAT_PER_MBTC).to_string(),
            "btc": (balance_f / SAT_PER_BTC).to_string(),

            "fiat_rate": (exchange_rate).to_string(),
            "fiat_currency": "USD", // TODO
            "fiat": (balance_f * exchange_rate).to_string(),
        }))
    }

    pub fn get_transactions(&self, details: &Value) -> Result<Value, Error> {
        let page = details.get("page").req()?.as_u64().req()? as u32;

        // fetch listtranssactions
        let txdescs = self.rpc.call::<Value>(
            "listtransactions",
            &[json!("*"), json!(PER_PAGE), json!(PER_PAGE * page)],
        )?;
        let txdescs = txdescs.as_array().unwrap();
        let potentially_has_more = txdescs.len() as u32 == PER_PAGE;

        // fetch full transactions and convert to GDK format
        let txs = txdescs
            .into_iter()
            .filter(|txdesc| txdesc.get("category").unwrap().as_str().unwrap() != "immature")
            .map(|txdesc| {
                let txid = Sha256dHash::from_hex(txdesc.get("txid").unwrap().as_str().unwrap())?;
                let blockhash =
                    Sha256dHash::from_hex(txdesc.get("blockhash").unwrap().as_str().unwrap())?;
                let tx = self.rpc.get_raw_transaction(&txid, Some(&blockhash))?;

                format_gdk_tx(txdesc, tx)
            })
            .collect::<Result<Vec<Value>, Error>>()?;

        Ok(json!({
            "list": txs,
            "page_id": page,
            "next_page_id": if potentially_has_more { Some(page+1) } else { None },
        }))
    }
}

impl fmt::Debug for Wallet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Wallet {{ }}")
    }
}

fn btc_to_sat(amount: f64) -> u64 {
    (amount * SAT_PER_BTC) as u64
}

fn format_gdk_tx(txdesc: &Value, tx: Transaction) -> Result<Value, Error> {
    let rawtx = serialize(&tx);
    println!("tx: {:#?}", txdesc);
    let fee = txdesc
        .get("fee")
        .map_or(0, |f| btc_to_sat(f.as_f64().unwrap() * -1.0));
    let weight = tx.get_weight();
    let vsize = (weight as f32 / 4.0) as u32;
    let type_str = match txdesc.get("category").req()?.as_str().req()? {
        "send" => "outgoing",
        "receive" => "incoming",
        _ => bail!("invalid tx category"),
    };

    Ok(json!({
        "block_height": 1, // TODO not available in txdesc. fetch by block hash or derive from tip height and confirmations?
        "created_at": txdesc.get("time").req()?.as_u64().req()?, // TODO to UTC string
        "type": type_str,
        "memo": txdesc.get("label").ok_or(""),

        "txhash": tx.txid().to_hex(),
        "transaction": hex::encode(&rawtx),

        "transaction_version": tx.version,
        "transaction_locktime": tx.lock_time,
        "transaction_size": rawtx.len(),
        "transaction_vsize": vsize,
        "transaction_weight": weight,

        "rbf_optin": txdesc.get("bip125-replaceable").req()?.as_str().req()? == "yes",
        "cap_cpfp": false, // TODO
        "can_rbf": false, // TODO
        "has_payment_request": false, // TODO
        "server_signed": false,
        "user_signed": true,
        "instant": false,

        "fee": fee,
        "fee_rate": (fee as f64)/(vsize as f64),

        //"inputs": tx.input.iter().map(format_gdk_input).collect(),
        //"outputs": tx.output.iter().map(format_gdk_output).collect(),
    }))
}
