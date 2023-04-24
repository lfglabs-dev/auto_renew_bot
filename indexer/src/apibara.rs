use crate::config::Config;
use apibara_core::{
    node::v1alpha2::DataFinality,
    starknet::v1alpha2::{FieldElement, Filter, HeaderFilter},
};
use apibara_sdk::Configuration;
use lazy_static::lazy_static;

lazy_static! {
    pub static ref STARKNET_ID_UPDATE: FieldElement =
        FieldElement::from_hex("0x440fc089956d79d058ea92abe99c718a6b1441e3aaec132cc38a01e9b895cb",)
            .unwrap();
    pub static ref DOMAIN_TO_ADDRESS_UPDATE: FieldElement = FieldElement::from_hex(
        "0x1b6d26c8b545f51ff2731ca42b81aa226221630abc95fd9d1bcacbe75bce7a1",
    )
    .unwrap();
    pub static ref ADDRESS_TO_DOMAIN_UPDATE: FieldElement = FieldElement::from_hex(
        "0x25d4f50ffa759476dcb003b1c94b6b1976321ccceae5f223696598ed626e9d3",
    )
    .unwrap();
    pub static ref DOMAIN_TRANSFER: FieldElement =
        FieldElement::from_hex("0x8abcf1baae89ea5d4ea9baa068abfdf471eb2b8ed985f88579b0128d4a597f",)
            .unwrap();
}

pub fn create_apibara_config(conf: &Config) -> Configuration<Filter> {
    Configuration::<Filter>::default()
        .with_finality(match conf.apibara.finality.as_str() {
            "Pending" => DataFinality::DataStatusPending,
            "Accepted" => DataFinality::DataStatusAccepted,
            "Finalized" => DataFinality::DataStatusFinalized,
            "Unknown" => DataFinality::DataStatusUnknown,
            _ => {
                panic!("error: finality must be Pending | Accepted | Finalized | Unknown");
            }
        })
        .with_batch_size(conf.apibara.batch_size)
        .with_starting_block(conf.apibara.starting_block)
        .with_filter(|f: Filter| -> Filter {
            f.with_header(HeaderFilter::weak())
                .add_event(|ev| {
                    ev.with_from_address(conf.contract.recipient.clone())
                        .with_keys(vec![STARKNET_ID_UPDATE.clone()])
                })
                .add_event(|ev| {
                    ev.with_from_address(conf.contract.recipient.clone())
                        .with_keys(vec![DOMAIN_TO_ADDRESS_UPDATE.clone()])
                })
                .add_event(|ev| {
                    ev.with_from_address(conf.contract.recipient.clone())
                        .with_keys(vec![ADDRESS_TO_DOMAIN_UPDATE.clone()])
                })
                .add_event(|ev| {
                    ev.with_from_address(conf.contract.recipient.clone())
                        .with_keys(vec![DOMAIN_TRANSFER.clone()])
                })
        })
}
