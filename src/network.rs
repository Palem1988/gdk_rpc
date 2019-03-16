use bitcoincore_rpc::Client;
use dirs;
use failure::Error;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;

use crate::errors::OptionExt;

#[derive(Serialize)]
pub struct Network {
    name: String,
    network: String,
    rpc_url: String,
    rpc_cred: Option<(String, String)>, // (username, password)
    rpc_cookie: Option<String>,
    tx_explorer_url: String,
}

lazy_static! {
    static ref NETWORKS: HashMap<String, Network> = {
        let mut networks = HashMap::new();

        let rpc_url = env::var("BITCOIND_URL")
            .ok()
            .unwrap_or_else(|| "http://127.0.0.1:18443".to_string());

        let rpc_cookie = env::var("BITCOIND_DIR")
            .ok()
            .map_or_else(
                || {
                    dirs::home_dir()
                        .unwrap()
                        .join(".bitcoin")
                        .join("regtest")
                        .join(".cookie")
                },
                |p| Path::new(&p).join(".cookie"),
            )
            .to_string_lossy()
            .into_owned();

        networks.insert(
            "regtest".to_string(),
            Network {
                name: "Regtest".to_string(),
                network: "regtest".to_string(),
                rpc_url,
                rpc_cred: None,
                rpc_cookie: Some(rpc_cookie.to_string()),
                tx_explorer_url: "https://blockstream.info/tx/".to_string(),
            },
        );
        networks
    };
}

impl Network {
    pub fn networks() -> &'static HashMap<String, Network> {
        &NETWORKS
    }

    pub fn network(id: &String) -> Option<&'static Network> {
        NETWORKS.get(id)
    }

    pub fn connect(&self) -> Result<Client, Error> {
        let cred = self
            .rpc_cred
            .clone()
            .or_else(|| {
                self.rpc_cookie
                    .as_ref()
                    .and_then(|path| read_cookie(path).ok())
            })
            .or_err("missing rpc credentials")?;

        let (rpc_user, rpc_pass) = cred;

        Ok(Client::new(
            self.rpc_url.clone(),
            Some(rpc_user),
            Some(rpc_pass),
        ))
    }
}

fn read_cookie(path: &String) -> Result<(String, String), Error> {
    let contents = fs::read_to_string(path)?;
    let parts: Vec<&str> = contents.split(":").collect();
    Ok((parts[0].to_string(), parts[1].to_string()))
}
