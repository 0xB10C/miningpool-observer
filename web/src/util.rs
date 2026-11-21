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
        assert!(parse_uint("-32").is_err());

        // leading zeros are fine
        assert_eq!(parse_uint("007").unwrap(), 7);
        assert_eq!(parse_uint("0000000000").unwrap(), 0);

        // the whole u32 range is supported, but nothing beyond it
        assert_eq!(parse_uint(&u32::MAX.to_string()).unwrap(), u32::MAX);
        assert!(parse_uint(&(u32::MAX as u64 + 1).to_string()).is_err());
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

    use tera::Function;

    fn call_with(f: &dyn Function, key: &str, value: tera::Value) -> tera::Result<tera::Value> {
        let mut args: HashMap<String, tera::Value> = HashMap::new();
        args.insert(key.to_string(), value);
        f.call(&args)
    }

    fn duration(seconds: i64) -> String {
        let f = seconds_to_duration();
        call_with(&f, "seconds", tera::to_value(seconds).unwrap())
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn test_seconds_to_duration() {
        assert_eq!(duration(0), "0s");
        assert_eq!(duration(1), "1s");
        assert_eq!(duration(59), "59s");
        assert_eq!(duration(60), "1m 0s");
        assert_eq!(duration(61), "1m 1s");
        assert_eq!(duration(3599), "59m 59s");
        assert_eq!(duration(3600), "1h 0m 0s");
        assert_eq!(duration(86399), "23h 59m 59s");
        assert_eq!(duration(86400), "1d 0h 0m 0s");
        assert_eq!(duration(90061), "1d 1h 1m 1s");
        assert_eq!(duration(i32::MAX as i64), "24855d 3h 14m 7s");
    }

    #[test]
    fn test_seconds_to_duration_negative_is_unknown() {
        assert_eq!(duration(-1), "Unknown");
        assert_eq!(duration(i32::MIN as i64), "Unknown");
    }

    #[test]
    fn test_seconds_to_duration_invalid_args() {
        let f = seconds_to_duration();

        // not an i32
        assert!(call_with(&f, "seconds", tera::to_value("60").unwrap()).is_err());
        assert!(call_with(&f, "seconds", tera::to_value(1.5).unwrap()).is_err());
        assert!(call_with(&f, "seconds", tera::to_value(i64::MAX).unwrap()).is_err());

        // no 'seconds' argument at all
        assert!(f.call(&HashMap::new()).is_err());
        assert!(call_with(&f, "second", tera::to_value(60).unwrap()).is_err());
    }

    #[test]
    fn test_tx_tag_id_to_tag() {
        let f = tx_tag_id_to_tag();

        let coinbase = call_with(
            &f,
            "id",
            tera::to_value(tags::TxTag::Coinbase as i32).unwrap(),
        )
        .unwrap();
        assert_eq!(coinbase["name"], tera::to_value("Coinbase").unwrap());
        assert_eq!(coinbase["color"], tera::to_value(tags::GRAY).unwrap());
        assert_eq!(coinbase["text_color"], tera::to_value(tags::WHITE).unwrap());
        assert!(!coinbase["description"].as_array().unwrap().is_empty());

        let young =
            call_with(&f, "id", tera::to_value(tags::TxTag::Young as i32).unwrap()).unwrap();
        assert_eq!(young["name"], tera::to_value("Young").unwrap());
        assert_eq!(young["color"], tera::to_value(tags::CYAN).unwrap());

        let sanctioned = call_with(
            &f,
            "id",
            tera::to_value(tags::TxTag::ToSanctioned as i32).unwrap(),
        )
        .unwrap();
        assert_eq!(sanctioned["name"], tera::to_value("To Sanctioned").unwrap());
        assert_eq!(sanctioned["color"], tera::to_value(tags::RED).unwrap());
        assert_eq!(
            sanctioned["description"].as_array().unwrap().len(),
            tags::TxTag::ToSanctioned.value().description.len()
        );
    }

    #[test]
    fn test_tx_tag_id_to_tag_invalid_args() {
        let f = tx_tag_id_to_tag();

        // 3100 is a deprecated tag id that can still be in the database
        let err = call_with(&f, "id", tera::to_value(3100).unwrap()).unwrap_err();
        assert!(err.to_string().contains("3100"), "{}", err);

        let err = call_with(&f, "id", tera::to_value(-1).unwrap()).unwrap_err();
        assert!(err.to_string().contains("-1"), "{}", err);

        // not an i32
        assert!(call_with(&f, "id", tera::to_value("4099").unwrap()).is_err());

        // no 'id' argument at all
        assert!(f.call(&HashMap::new()).is_err());
        assert!(call_with(&f, "ID", tera::to_value(4099).unwrap()).is_err());
    }

    #[test]
    fn test_block_tag_id_to_tag() {
        let f = block_tag_id_to_tag();

        let taproot = call_with(
            &f,
            "id",
            tera::to_value(tags::BlockTag::TaprootSignaling as i32).unwrap(),
        )
        .unwrap();
        assert_eq!(
            taproot["name"],
            tera::to_value("Taproot Signaling").unwrap()
        );
        assert_eq!(taproot["color"], tera::to_value(tags::CYAN).unwrap());
        assert_eq!(taproot["text_color"], tera::to_value(tags::WHITE).unwrap());
        assert!(!taproot["description"].as_array().unwrap().is_empty());

        let sigops = call_with(
            &f,
            "id",
            tera::to_value(tags::BlockTag::SigopsLimitClose as i32).unwrap(),
        )
        .unwrap();
        assert_eq!(
            sigops["name"],
            tera::to_value("Close to Sigop Limit").unwrap()
        );
        assert_eq!(sigops["color"], tera::to_value(tags::CYAN).unwrap());
    }

    #[test]
    fn test_block_tag_id_to_tag_invalid_args() {
        let f = block_tag_id_to_tag();

        let err = call_with(&f, "id", tera::to_value(1234).unwrap()).unwrap_err();
        assert!(err.to_string().contains("1234"), "{}", err);

        // a tx tag id is not a block tag id
        assert!(call_with(
            &f,
            "id",
            tera::to_value(tags::TxTag::Coinbase as i32).unwrap()
        )
        .is_err());

        // not an i32
        assert!(call_with(&f, "id", tera::to_value("3100").unwrap()).is_err());

        // no 'id' argument at all
        assert!(f.call(&HashMap::new()).is_err());
    }
}
