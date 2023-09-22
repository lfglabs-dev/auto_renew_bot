use bigdecimal::{num_bigint::BigInt, BigDecimal};
use num_integer::Integer;
use starknet::core::types::FieldElement;

lazy_static::lazy_static! {
    static ref TWO_POW_128: BigInt = BigInt::from(2).pow(128);
}

pub fn to_uint256(n: BigInt) -> (FieldElement, FieldElement) {
    let (n_high, n_low) = n.div_rem(&TWO_POW_128);
    let (_, low_bytes) = n_low.to_bytes_be();
    let (_, high_bytes) = n_high.to_bytes_be();

    (
        FieldElement::from_byte_slice_be(&low_bytes).unwrap(),
        FieldElement::from_byte_slice_be(&high_bytes).unwrap(),
    )
}

pub fn hex_to_bigdecimal(hex: &str) -> Option<BigDecimal> {
    let without_prefix = hex.trim_start_matches("0x");
    BigInt::parse_bytes(without_prefix.as_bytes(), 16).map(BigDecimal::from)
}
