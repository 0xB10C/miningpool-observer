use std::collections::HashMap;
use std::convert::TryFrom;

use actix_web::{error, Error};

use miningpool_observer_shared::tags;

const ERROR_INVALID_INT: &str = "INVALID INT";
const ERROR_INVALID_BLOCK_HASH: &str = "INVALID BLOCK HASH";
const ERROR_INVALID_TXID: &str = "INVALID TXID";

pub fn parse_uint(uint_str: &str) -> Result<u32, Error> {
    match uint_str.parse::<u32>() {
        Ok(uint) => Ok(uint),
        Err(e) => {
            log::warn!(target: "web_handler", "uint_str: {}", e);
            Err(error::ErrorNotFound(ERROR_INVALID_INT))
        }
    }
}

pub fn parse_block_hash_str(hash_str: &str) -> Result<Vec<u8>, Error> {
    // Note: We deliberately don't require a number of leading zeros here. How
    // many zeros a block hash has depends on the difficulty. On regtest (and
    // sometimes on signet and testnet) there are fewer than six. Looking up an
    // unknown hash fails in the database query anyway.
    if hash_str.len() != 64 {
        log::warn!(target: "web_handler", "parse_block_hash_str: invalid block hash {}", hash_str);
        return Err(error::ErrorNotFound(ERROR_INVALID_BLOCK_HASH));
    }

    match hex::decode(hash_str) {
        Err(e) => {
            log::warn!(target: "web_handler", "parse_block_hash_str: invalid block hash {}: {}", hash_str, e);
            Err(error::ErrorNotFound(ERROR_INVALID_BLOCK_HASH))
        }
        Ok(hash) => Ok(hash),
    }
}

pub fn parse_txid_str(txid_str: &str) -> Result<Vec<u8>, Error> {
    if txid_str.len() != 64 {
        log::warn!(target: "web_handler", "parse_txid_str: invalid txid {}", txid_str);
        return Err(error::ErrorNotFound(ERROR_INVALID_TXID));
    }

    match hex::decode(&txid_str) {
        Err(e) => {
            log::warn!(target: "web_handler", "parse_txid_str: invalid txid {}: {}", txid_str, e);
            Err(error::ErrorNotFound(ERROR_INVALID_TXID))
        }
        Ok(txid) => Ok(txid),
    }
}

pub fn tx_tag_id_to_tag() -> impl tera::Function {
    Box::new(
        move |args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
            match args.get("id") {
                Some(val) => match tera::from_value::<i32>(val.clone()) {
                    Ok(v) => {
                        match tags::TxTag::try_from(v) {
                            Ok(v) => {
                                Ok(tera::to_value(v.value()).unwrap())
                            },
                            Err(e) => {
                                Err(format!("Could not find TxTag for value {:?}. Is the mapping implemented?: {:?}", v, e).into())
                            }
                        }
                    }
                    Err(_) => Err(format!("Can't parse 'id' with val={} as i32.", val).into()),
                },
                None => Err("No parameter 'id' passed to tag_id_to_tag()".into()),
            }
        },
    )
}

pub fn block_tag_id_to_tag() -> impl tera::Function {
    Box::new(
        move |args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
            match args.get("id") {
                Some(val) => match tera::from_value::<i32>(val.clone()) {
                    Ok(v) => {
                        match tags::BlockTag::try_from(v) {
                            Ok(v) => {
                                Ok(tera::to_value(v.value()).unwrap())
                            },
                            Err(e) => {
                                Err(format!("Could not find BlockTag for value {:?}. Is the mapping implemented?: {:?}", v, e).into())
                            }
                        }
                    }
                    Err(_) => Err(format!("Can't parse 'id' with val={} as i32.", val).into()),
                },
                None => Err("No parameter 'id' passed to tag_id_to_tag()".into()),
            }
        },
    )
}

/// Converts seconds to a duration String.
pub fn seconds_to_duration() -> impl tera::Function {
    Box::new(
        move |args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
            match args.get("seconds") {
                Some(val) => match tera::from_value::<i32>(val.clone()) {
                    Ok(v) => Ok(if v < 0 {
                        tera::to_value("Unknown").unwrap()
                    } else {
                        let d = std::time::Duration::from_secs(v as u64);
                        let seconds = d.as_secs() % 60;
                        let minutes = (d.as_secs() / 60) % 60;
                        let hours = (d.as_secs() / 60) / 60 % 24;
                        let days = (d.as_secs() / 60) / 60 / 24;
                        if days > 0 {
                            tera::to_value(format!(
                                "{}d {}h {}m {}s",
                                days, hours, minutes, seconds
                            ))
                            .unwrap()
                        } else if hours > 0 {
                            tera::to_value(format!("{}h {}m {}s", hours, minutes, seconds)).unwrap()
                        } else if minutes > 0 {
                            tera::to_value(format!("{}m {}s", minutes, seconds)).unwrap()
                        } else {
                            tera::to_value(format!("{}s", seconds)).unwrap()
                        }
                    }),
                    Err(_) => Err("Can't parse 'seconds' as i32.".into()),
                },
                None => Err("No parameter 'seconds' passed to seconds_to_duration()".into()),
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_uint() {
        assert_eq!(parse_uint("0").unwrap(), 0);
        assert_eq!(parse_uint("32").unwrap(), 32);

        assert!(parse_uint("").is_err());
        assert!(parse_uint(" ").is_err());
        assert!(parse_uint("-").is_err());
        assert!(parse_uint("+").is_err());
        assert!(parse_uint("*").is_err());
        assert!(parse_uint(".").is_err());
        assert!(parse_uint("0 ").is_err());
        assert!(parse_uint(" 0 ").is_err());
        assert!(parse_uint(" 32").is_err());
        assert!(parse_uint("-1").is_err());
        assert!(parse_uint("99999999999999999999999").is_err());
    }

    #[test]
    fn test_parse_block_hash_str() {
        assert_eq!(
            parse_block_hash_str(
                "0000000000000000000000000000000000000000000000000000000000000000"
            )
            .unwrap(),
            &[0; 32]
        );
        assert!(parse_block_hash_str(
            "0000001111111111111111111111111111111111111111111111111111111111"
        )
        .is_ok());
        assert_eq!(
            parse_block_hash_str(
                "1111111111111111111111111111111111111111111111111111111111111111"
            )
            .unwrap(),
            &[17; 32]
        );
        // Blocks on regtest (and sometimes on signet and testnet) don't have
        // six leading zeros. Their hashes have to be accepted as well.
        let regtest_genesis = "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206";
        assert_eq!(
            parse_block_hash_str(regtest_genesis).unwrap(),
            hex::decode(regtest_genesis).unwrap()
        );
        // upper-case hex is accepted as well
        assert!(parse_block_hash_str(
            "0F9188F13CB7B2C71F2A335E3A4FC328BF5BEB436012AFCA590B1A11466E2206"
        )
        .is_ok());

        assert!(parse_block_hash_str("").is_err());
        assert!(parse_block_hash_str("0000").is_err());
        // only 63 0's
        assert!(parse_block_hash_str(
            "000000000000000000000000000000000000000000000000000000000000000"
        )
        .is_err());
        // 65 0's
        assert!(parse_block_hash_str(
            "00000000000000000000000000000000000000000000000000000000000000000"
        )
        .is_err());
        // 64 characters, but not hex
        assert!(parse_block_hash_str(
            "00000000000000000000000000000iiiiiiiiii0000000000000000000000001"
        )
        .is_err());
    }

    #[test]
    fn test_parse_txid_str() {
        assert_eq!(
            parse_txid_str("0000000000000000000000000000000000000000000000000000000000000000")
                .unwrap(),
            &[0; 32]
        );
        assert_eq!(
            parse_txid_str("1111111111111111111111111111111111111111111111111111111111111111")
                .unwrap(),
            &[17; 32]
        );

        assert!(parse_txid_str("").is_err());
        assert!(parse_txid_str("0000").is_err());
        // only 63 0's
        assert!(
            parse_txid_str("000000000000000000000000000000000000000000000000000000000000000")
                .is_err()
        );
        assert!(
            parse_txid_str("00000000000000000000000000000iiiiiiiiii0000000000000000000000001")
                .is_err()
        );
    }
}
