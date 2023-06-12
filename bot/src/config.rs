use serde::Deserialize;
use starknet::core::types::FieldElement;
use std::env;
use std::fs;

macro_rules! pub_struct {
    ($name:ident {$($field:ident: $t:ty,)*}) => {
        #[derive(Deserialize)]
        pub struct $name {
            $(pub $field: $t),*
        }
    }
}
pub_struct!(Apibara {
    finality: String,
    starting_block: u64,
    batch_size: u64,
    stream: String,
});

pub_struct!(Contract {
    starknetid: FieldElement,
    naming: FieldElement,
    renewal: FieldElement,
    erc20: FieldElement,
    pricing: FieldElement,
});

pub_struct!(Database {
    name: String,
    connection_string: String,
});

pub_struct!(DevnetProvider {
    is_devnet: bool,
    is_testnet: bool,
    gateway: String,
    feeder_gateway: String,
});

pub_struct!(MyAccount {
    private_key: String,
    address: String,
});

pub_struct!(Discord {
    token: String,
    channel_id: u64,
});

pub_struct!(Config {
    apibara: Apibara,
    contract: Contract,
    database: Database,
    devnet_provider: DevnetProvider,
    account: MyAccount,
    discord: Discord,
});

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