use bigdecimal::{num_bigint::BigInt, FromPrimitive};

lazy_static::lazy_static! {
    static ref PRICE_DOMAIN_LEN_1: BigInt = BigInt::from_u128(1068493150684932 * 365).unwrap();
    static ref PRICE_DOMAIN_LEN_2: BigInt = BigInt::from_u128(657534246575343 * 365).unwrap();
    static ref PRICE_DOMAIN_LEN_3: BigInt = BigInt::from_u128(410958904109590 * 365).unwrap();
    static ref PRICE_DOMAIN_LEN_4: BigInt = BigInt::from_u128(232876712328767 * 365).unwrap();
    static ref PRICE_DOMAIN: BigInt = BigInt::from_u128(24657534246575 * 365).unwrap();
}


pub fn get_renewal_price(domain: String) -> BigInt {
    let domain_name = domain
            .strip_suffix(".stark")
            .ok_or_else(|| anyhow::anyhow!("Invalid domain name: {:?}", domain))
            .unwrap();
    let domain_len = domain_name.len();
    match domain_len {
        1 => PRICE_DOMAIN_LEN_1.clone(),
        2 => PRICE_DOMAIN_LEN_2.clone(),
        3 => PRICE_DOMAIN_LEN_3.clone(),
        4 => PRICE_DOMAIN_LEN_4.clone(),
        _ => PRICE_DOMAIN.clone(),
    }
}
