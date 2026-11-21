//! Helpers for building the test fixtures used by the unit tests.

use std::collections::HashMap;
use std::convert::TryInto;
use std::str::FromStr;

use miningpool_observer_shared::bitcoincore_rpc::bitcoin::{
    block::Header, block::Version as BlockVersion, consensus, hashes::Hash,
    locktime::absolute::LockTime, script, script::PushBytesBuf, transaction::Version, Address,
    Amount, Block, BlockHash, CompactTarget, OutPoint, ScriptBuf, Sequence, Transaction, TxIn,
    TxMerkleNode, TxOut, Txid, Witness,
};
use miningpool_observer_shared::bitcoincore_rpc::json::{
    GetBlockTemplateResult, GetBlockTemplateResultTransaction, GetBlockTxFeesResult,
    GetBlockTxFeesResultTxidFee,
};
use miningpool_observer_shared::chrono;
use miningpool_observer_shared::diesel::connection::Connection;
use miningpool_observer_shared::diesel::pg::PgConnection;
use miningpool_observer_shared::model as shared_model;

use rawtx_rs::tx::TransactionSigops;

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
/// Mainnet Pay-to-Public-Key-Hash address used in tests.
pub const ADDRESS_P2PKH: &str = "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2";

/// Returns the script_pubkey of a mainnet address.
pub fn script_for_address(address: &str) -> ScriptBuf {
    Address::from_str(address)
        .expect("test address should be parsable")
        .assume_checked()
        .script_pubkey()
}

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

/// Builds a block template transaction entry for a transaction.
pub fn template_tx(tx: &Transaction, fee_sat: u64) -> GetBlockTemplateResultTransaction {
    GetBlockTemplateResultTransaction {
        txid: tx.compute_txid(),
        wtxid: tx.compute_wtxid(),
        raw_tx: consensus::serialize(tx),
        fee: Amount::from_sat(fee_sat),
        sigops: tx.sigops().unwrap_or_default() as u32,
        weight: tx.weight().to_wu() as usize,
        depends: vec![],
    }
}

/// Builds a block template with sane defaults for the fields the daemon does
/// not look at.
pub fn template(
    transactions: Vec<GetBlockTemplateResultTransaction>,
    height: u64,
    current_time: u64,
    previous_block_hash: BlockHash,
    coinbase_value_sat: u64,
) -> GetBlockTemplateResult {
    GetBlockTemplateResult {
        bits: vec![0x17, 0x0f, 0x48, 0xe4],
        previous_block_hash,
        current_time,
        height,
        sigop_limit: 80_000,
        size_limit: 4_000_000,
        weight_limit: 4_000_000,
        version: 0x2000_0000,
        rules: vec![],
        capabilities: vec![],
        version_bits_available: HashMap::new(),
        version_bits_required: 0,
        longpollid: String::new(),
        transactions,
        signet_challenge: ScriptBuf::new(),
        default_witness_commitment: ScriptBuf::new(),
        coinbaseaux: HashMap::new(),
        coinbase_value: Amount::from_sat(coinbase_value_sat),
        target: vec![0u8; 32],
        min_time: current_time,
        mutable: vec![],
        nonce_range: vec![0, 0, 0, 0],
    }
}

/// Builds the result of the getblocktxfees RPC for a list of (txid, fee) pairs.
pub fn block_tx_fees(fees: &[(Txid, u64)]) -> GetBlockTxFeesResult {
    GetBlockTxFeesResult {
        tx: fees
            .iter()
            .map(|(txid, fee)| GetBlockTxFeesResultTxidFee {
                txid: *txid,
                fee: Amount::from_sat(*fee),
            })
            .collect(),
    }
}

/// Builds a block with the passed transactions.
pub fn block(
    txdata: Vec<Transaction>,
    version: i32,
    time: u32,
    prev_blockhash: BlockHash,
) -> Block {
    Block {
        header: Header {
            version: BlockVersion::from_consensus(version),
            prev_blockhash,
            merkle_root: TxMerkleNode::all_zeros(),
            time,
            bits: CompactTarget::from_consensus(0x170f_48e4),
            nonce: 0,
        },
        txdata,
    }
}

static MIGRATE_TEST_DATABASE: std::sync::Once = std::sync::Once::new();

/// Returns a database connection for the database integration tests, or None
/// when no test database is configured. The connection is inside a test
/// transaction, so nothing a test writes is committed.
pub fn test_conn() -> Option<PgConnection> {
    let url = match std::env::var("TEST_DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            println!("skipped: TEST_DATABASE_URL not set");
            return None;
        }
    };

    // Only one of the tests running in parallel may migrate the database.
    MIGRATE_TEST_DATABASE.call_once(|| {
        let mut conn =
            PgConnection::establish(&url).expect("could not connect to the test database");
        crate::db::run_migrations(&mut conn).expect("could not migrate the test database");
    });

    let mut conn = PgConnection::establish(&url).expect("could not connect to the test database");
    conn.begin_test_transaction()
        .expect("could not start a test transaction");
    Some(conn)
}

/// Builds a block for the database tests. Blocks built with a different
/// `hash_byte` have a different hash.
pub fn new_block_fixture(hash_byte: u8, height: i32) -> shared_model::NewBlock {
    let time = chrono::DateTime::from_timestamp(1_600_000_000, 0)
        .expect("timestamp should be in range")
        .naive_utc();
    shared_model::NewBlock {
        hash: vec![hash_byte; 32],
        prev_hash: vec![hash_byte.wrapping_sub(1); 32],
        height,
        tags: vec![],
        missing_tx: 1,
        extra_tx: 2,
        shared_tx: 3,
        sanctioned_missing_tx: 0,
        equality: 0.0,
        block_time: time,
        block_seen_time: time,
        block_tx: 4,
        block_sanctioned: 0,
        block_cb_value: 625_000_000,
        block_cb_fees: 1_000,
        block_weight: 3_999_000,
        block_pkg_weights: vec![1, 2],
        block_pkg_feerates: vec![1.0, 2.0],
        pool_name: "Unknown".to_string(),
        pool_link: String::new(),
        pool_id_method: String::new(),
        template_tx: 5,
        template_time: time,
        template_sanctioned: 0,
        template_cb_value: 625_000_000,
        template_cb_fees: 1_000,
        template_weight: 3_998_000,
        template_pkg_weights: vec![3],
        template_pkg_feerates: vec![3.0],
        template_sigops: 100,
        block_sigops: 110,
    }
}

/// Builds a transaction for the database tests. Transactions built with a
/// different `txid_byte` have a different txid.
pub fn db_transaction_fixture(txid_byte: u8, tags: Vec<i32>) -> shared_model::Transaction {
    shared_model::Transaction {
        txid: vec![txid_byte; 32],
        sanctioned: false,
        vsize: 250,
        fee: 1_000,
        output_sum: 100_000,
        tags,
        input_count: 1,
        inputs: vec!["1x P2WPKH".to_string()],
        output_count: 2,
        outputs: vec!["2x P2WPKH v0".to_string()],
        sigops: 4,
    }
}
