//! Helpers for building the test fixtures used by the unit tests.

use std::convert::TryInto;
use std::str::FromStr;

use miningpool_observer_shared::bitcoincore_rpc::bitcoin::{
    consensus, hashes::Hash, locktime::absolute::LockTime, script, script::PushBytesBuf,
    transaction::Version, Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
    Txid, Witness,
};

use miningpool_observer_shared::model as shared_model;

use crate::model::TxInfo;

/// Sequence number that neither signals RBF nor enables the absolute lock time.
pub const SEQUENCE_FINAL: u32 = 0xffff_ffff;
/// Highest sequence number that enables the absolute lock time but does not
/// signal RBF.
pub const SEQUENCE_LOCKTIME_ONLY: u32 = 0xffff_fffe;
/// Sequence number that enables the absolute lock time and signals RBF.
pub const SEQUENCE_RBF: u32 = 0xffff_fffd;

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

/// Builds a coinbase transaction. The script_sig contains the BIP34 block
/// height push followed by `tag`.
pub fn coinbase_tx(height: i64, tag: &[u8], outputs: &[(u64, ScriptBuf)]) -> Transaction {
    let tag_push: PushBytesBuf = tag
        .to_vec()
        .try_into()
        .expect("coinbase tag should not be too long");
    let script_sig = script::Builder::new()
        .push_int(height)
        .push_slice(tag_push)
        .into_script();

    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig,
            sequence: Sequence(SEQUENCE_FINAL),
            witness: Witness::new(),
        }],
        output: outputs
            .iter()
            .map(|(value, script_pubkey)| TxOut {
                value: Amount::from_sat(*value),
                script_pubkey: script_pubkey.clone(),
            })
            .collect(),
    }
}

/// Deserializes a hex encoded transaction.
pub fn deser_tx(raw_tx_hex: &str) -> Transaction {
    consensus::deserialize(&hex::decode(raw_tx_hex).expect("test transaction should be valid hex"))
        .expect("test transaction should be deserializable")
}

/// Builds a transaction with exactly `vsize` vBytes by padding it with an
/// OP_RETURN output.
pub fn tx_with_vsize(vsize: usize) -> Transaction {
    let padded = |payload_len: usize| {
        fake_tx(
            &[outpoint(1, 0)],
            &[
                (1_000, script_for_address(ADDRESS_P2WPKH)),
                (0, op_return_script(payload_len)),
            ],
            0,
            SEQUENCE_FINAL,
        )
    };

    // The OP_RETURN payload length and the transaction size grow by the same
    // amount, so this converges after one correction.
    let mut payload_len = vsize / 2;
    for _ in 0..5 {
        let tx = padded(payload_len);
        if tx.vsize() == vsize {
            return tx;
        }
        payload_len = (payload_len + vsize)
            .checked_sub(tx.vsize())
            .expect("the requested vsize should be reachable");
    }
    panic!("could not build a transaction with a vsize of {}", vsize);
}

/// Returns the outpoint of an output of a transaction.
pub fn spend_of(tx: &Transaction, vout: u32) -> OutPoint {
    OutPoint {
        txid: tx.compute_txid(),
        vout,
    }
}

/// Returns the txid of a transaction in display (reversed) byte order, which
/// is how the database stores it.
pub fn reversed_txid(tx: &Transaction) -> Vec<u8> {
    tx.compute_txid()
        .to_byte_array()
        .iter()
        .rev()
        .cloned()
        .collect()
}
