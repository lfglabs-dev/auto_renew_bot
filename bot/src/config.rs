use serde::de::MapAccess;
use serde::de::Visitor;
use serde::Deserialize;
use serde::Deserializer;
use starknet::core::types::FieldElement;
use std::collections::HashMap;
use std::env;
use std::fmt;
use std::fs;

macro_rules! pub_struct {
    ($($derive:path),*; $name:ident {$($field:ident: $t:ty),* $(,)?}) => {
        #[derive($($derive),*)]
        pub struct $name {
            $(pub $field: $t),*
        }
    }
}

pub_struct!(Clone, Deserialize; Contract {
    starknetid: FieldElement,
    naming: FieldElement,
    renewal: FieldElement,
    erc20: FieldElement,
    pricing: FieldElement,
    multicall: FieldElement,
});

pub_struct!(Clone, Deserialize; Database {
    name: String,
    connection_string: String,
    metadata_name : String,
    connection_string_metadata: String,
});

pub_struct!(Clone, Deserialize; MyAccount {
    private_key: FieldElement,
    address: FieldElement,
});

pub_struct!(Clone, Deserialize; Renewals {
    delay: u64,
    expiry_days: i64,
});

pub_struct!(Clone, Deserialize; IndexerServer {
    port: Vec<u16>,
    server_url: String,
});

pub_struct!(Clone, Deserialize; Rpc {
    rpc_url: String,
});

pub_struct!(Clone, Deserialize; Watchtower {
    enabled : bool,
    endpoint: String,
    app_id: String,
    token: String,
    types: WatchtowerTypes,
});

pub_struct!(Clone, Deserialize; WatchtowerTypes {
    info: String,
    warning: String,
    severe: String,
});

pub_struct!(Clone, Deserialize; Server {
    starknetid_api: String,
});

pub_struct!(Clone, Deserialize; Renewer {
    address: FieldElement,
    renewal_contract: FieldElement,
});

pub_struct!(Clone; Config {
    contract: Contract,
    database: Database,
    account: MyAccount,
    renewals: Renewals,
    indexer_server: IndexerServer,
    rpc: Rpc,
    watchtower: Watchtower,
    server: Server,
    renewers_mapping: HashMap<FieldElement, FieldElement>,
});

impl<'de> Deserialize<'de> for Config {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct OuterConfig {
            contract: Contract,
            database: Database,
            account: MyAccount,
            renewals: Renewals,
            indexer_server: IndexerServer,
            rpc: Rpc,
            watchtower: Watchtower,
            server: Server,
            renewers: HashMap<String, Renewer>,
        }

        let OuterConfig {
            contract,
            database,
            account,
            renewals,
            indexer_server,
            rpc,
            watchtower,
            server,
            renewers,
        } = OuterConfig::deserialize(deserializer)?;

        // Build atcoins mapping
        let renewers_mapping = renewers
            .into_values()
            .map(|val| (val.renewal_contract, val.address))
            .collect();

        Ok(Config {
            contract,
            database,
            account,
            renewals,
            indexer_server,
            rpc,
            watchtower,
            server,
            renewers_mapping,
        })
    }
}

pub fn load() -> Config {
    let args: Vec<String> = env::args().collect();
    let config_path = if args.len() <= 1 {
        "config.toml"
    } else {
        args.get(1).unwrap()
    };
    let file_contents = fs::read_to_string(config_path);
    if file_contents.is_err() {
        panic!("error: unable to read file with path \"{}\"", config_path);
    }

    match toml::from_str(file_contents.unwrap().as_str()) {
        Ok(loaded) => loaded,
        Err(err) => {
            panic!("error: unable to deserialize config. {}", err);
        }
    }
}
