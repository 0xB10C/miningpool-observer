//! Helpers for building the test fixtures used by the unit tests.

use std::convert::TryInto;
use std::str::FromStr;

use miningpool_observer_shared::bitcoincore_rpc::bitcoin::{
    hashes::Hash, locktime::absolute::LockTime, script::PushBytesBuf, transaction::Version,
    Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
};

use miningpool_observer_shared::model as shared_model;

use crate::model::TxInfo;

/// Sequence number that neither signals RBF nor enables the absolute lock time.
pub const SEQUENCE_FINAL: u32 = 0xffff_ffff;

/// Builds a deterministic outpoint. All outpoints built from a different `n`
/// are different.
pub fn outpoint(n: u8, vout: u32) -> OutPoint {
    OutPoint {
        txid: Txid::from_byte_array([n; 32]),
        vout,
    }
}

/// Builds a transaction spending the passed outpoints and paying the passed
/// (amount in sat, script_pubkey) outputs. All inputs share the same sequence
/// number.
///
/// The inputs carry a dummy script_sig. An input with an empty script_sig and
/// an empty witness would be detected as a Pay-to-Anchor input, which counts
/// as spending SegWit. With the dummy script_sig the inputs are detected as
/// Pay-to-Script-Hash inputs without any sigops.
pub fn fake_tx(
    inputs: &[OutPoint],
    outputs: &[(u64, ScriptBuf)],
    locktime: u32,
    sequence: u32,
) -> Transaction {
    Transaction {
        version: Version::TWO,
        lock_time: LockTime::from_consensus(locktime),
        input: inputs
            .iter()
            .map(|previous_output| TxIn {
                previous_output: *previous_output,
                script_sig: ScriptBuf::from_bytes(vec![0x01, 0x51]),
                sequence: Sequence(sequence),
                witness: Witness::new(),
            })
            .collect(),
        output: outputs
            .iter()
            .map(|(value, script_pubkey)| TxOut {
                value: Amount::from_sat(*value),
                script_pubkey: script_pubkey.clone(),
            })
            .collect(),
    }
}

/// Wraps a transaction into a [TxInfo] with the passed position and fee.
pub fn tx_info(tx: &Transaction, pos: i32, fee_sat: u64) -> TxInfo {
    TxInfo {
        txid: tx.compute_txid(),
        tx: tx.clone(),
        pos,
        fee: Amount::from_sat(fee_sat),
    }
}

/// Mainnet Pay-to-Witness-Public-Key-Hash address used in tests.
pub const ADDRESS_P2WPKH: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
/// Mainnet Pay-to-Taproot address used in tests.
pub const ADDRESS_P2TR: &str = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr";

/// Returns the script_pubkey of a mainnet address.
pub fn script_for_address(address: &str) -> ScriptBuf {
    Address::from_str(address)
        .expect("test address should be parsable")
        .assume_checked()
        .script_pubkey()
}

/// Mainnet Pay-to-Public-Key-Hash address used in tests.
pub const ADDRESS_P2PKH: &str = "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2";

/// Returns an OP_RETURN script with a `payload_len` byte long payload.
pub fn op_return_script(payload_len: usize) -> ScriptBuf {
    let payload: PushBytesBuf = vec![0x42u8; payload_len]
        .try_into()
        .expect("OP_RETURN payload should not be too long");
    ScriptBuf::new_op_return(payload)
}

/// Builds a sanctioned UTXO for the passed outpoint. The database stores the
/// txid in display (reversed) byte order.
pub fn sanctioned_utxo(
    outpoint: OutPoint,
    script_pubkey: &ScriptBuf,
    amount: i64,
) -> shared_model::SanctionedUtxo {
    shared_model::SanctionedUtxo {
        txid: outpoint
            .txid
            .to_byte_array()
            .iter()
            .rev()
            .cloned()
            .collect(),
        vout: outpoint.vout as i32,
        script_pubkey: script_pubkey.to_bytes(),
        amount,
        height: 0,
    }
}
