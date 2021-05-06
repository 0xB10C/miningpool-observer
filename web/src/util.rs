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
    if hash_str.len() != 64 || !hash_str.starts_with("000000") {
        log::warn!(target: "web_handler", "parse_block_hash_str: invalid block height {}", hash_str);
        return Err(error::ErrorNotFound(ERROR_INVALID_BLOCK_HASH));
    }

    match hex::decode(&hash_str.to_string()) {
        Err(e) => {
            log::warn!(target: "web_handler", "parse_block_hash_str: invalid block height {}: {}", hash_str, e);
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

    match hex::decode(&txid_str.to_string()) {
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
                    Ok(v) => Ok(tera::to_value(tags::TxTag::try_from(v).unwrap().value()).unwrap()),
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
                        Ok(tera::to_value(tags::BlockTag::try_from(v).unwrap().value()).unwrap())
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
        assert!(parse_uint("-32").is_err());
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

        assert!(parse_block_hash_str("").is_err());
        assert!(parse_block_hash_str("0000").is_err());
        // only 63 0's
        assert!(parse_block_hash_str(
            "000000000000000000000000000000000000000000000000000000000000000"
        )
        .is_err());
        assert!(parse_block_hash_str(
            "1111111111111111111111111111111111111111111111111111111111111111"
        )
        .is_err());
        assert!(parse_block_hash_str(
            "00000000000000000000000000000iiiiiiiiii0000000000000000000000001"
        )
        .is_err());
        // one too few 0's at start
        assert!(parse_block_hash_str(
            "0000011111111111111111111111111111111111111111111111111111111111"
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
