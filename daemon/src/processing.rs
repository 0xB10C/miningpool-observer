use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::metrics;
use crate::model::{BlockTxData, SharedPoolIDData, TemplateTxData, TxInfo, TxPackage};

use miningpool_observer_shared::bitcoincore_rpc::json::{
    GetBlockTemplateResult, GetBlockTxFeesResult,
};
use miningpool_observer_shared::chrono;
use miningpool_observer_shared::{model as shared_model, tags};

use bitcoin_pool_identification::{IdentificationMethod, PoolIdentification};
use rawtx_rs::tx::TxInfo as RawTxInfo;
use rawtx_rs::tx::TxInfoError as RawTxInfoError;
use rawtx_rs::tx::{
    is_opreturn_counterparty, is_p2ms_counterparty, is_p2sh_counterparty, TransactionSigops,
};
use rawtx_rs::{input::InputInscriptionDetection, input::InputType, output::OutputType};

use miningpool_observer_shared::bitcoincore_rpc::bitcoin;
use miningpool_observer_shared::bitcoincore_rpc::bitcoin::{
    hash_types::Txid, hashes::Hash, network::Network, Address, Amount, Transaction,
};

pub const LOG_TARGET_PROCESSING: &str = "processing";

const VERSION_BIT_TAPROOT: u8 = 2;

fn in_and_outputs_to_strings(raw_tx_info: &RawTxInfo) -> (Vec<String>, Vec<String>) {
    let mut output_type_counts: HashMap<OutputType, u32> = HashMap::new();
    for ot in raw_tx_info.output_infos.iter().map(|i| i.out_type) {
        *output_type_counts.entry(ot).or_insert(0) += 1;
    }

    let mut input_type_counts: HashMap<InputType, u32> = HashMap::new();
    for it in raw_tx_info.input_infos.iter().map(|i| i.in_type) {
        *input_type_counts.entry(it).or_insert(0) += 1;
    }

    // The hash maps are iterated in an arbitrary order. The strings are sorted
    // to make the values we store and display deterministic.
    let mut inputs_strs: Vec<String> = input_type_counts
        .iter()
        .map(|(k, v)| format!("{}x {}", v, k))
        .collect();
    inputs_strs.sort();
    let mut outputs_strs: Vec<String> = output_type_counts
        .iter()
        .map(|(k, v)| format!("{}x {}", v, k))
        .collect();
    outputs_strs.sort();

    (inputs_strs, outputs_strs)
}

pub fn build_outpoint_to_sanctioned_utxo_map(
    sanctioned_utxos: &[shared_model::SanctionedUtxo],
) -> HashMap<(Vec<u8>, u32), &shared_model::SanctionedUtxo> {
    sanctioned_utxos
        .iter()
        .map(|utxo| {
            let mut reversed_txid = utxo.txid.clone();
            reversed_txid.reverse();
            ((reversed_txid, utxo.vout as u32), utxo)
        })
        .collect()
}

fn is_tx_sanctioned(
    tx: &Transaction,
    outpoint_to_sanctioned_utxo_map: &HashMap<(Vec<u8>, u32), &shared_model::SanctionedUtxo>,
    sanctioned_addresses: &HashSet<String>,
) -> bool {
    is_tx_to_sanctioned(tx, sanctioned_addresses)
        || is_tx_from_sanctioned(tx, outpoint_to_sanctioned_utxo_map)
}

fn is_tx_to_sanctioned(tx: &Transaction, sanctioned_addresses: &HashSet<String>) -> bool {
    for output in tx.output.iter() {
        if let Ok(address) = Address::from_script(&output.script_pubkey, Network::Bitcoin) {
            if sanctioned_addresses.contains(&address.to_string()) {
                return true;
            }
        }
    }
    false
}

fn is_tx_from_sanctioned(
    tx: &Transaction,
    outpoint_to_sanctioned_utxo_map: &HashMap<(Vec<u8>, u32), &shared_model::SanctionedUtxo>,
) -> bool {
    tx.input
        .iter()
        .map(|input| input.previous_output)
        .any(|outpoint| {
            outpoint_to_sanctioned_utxo_map
                .contains_key(&(outpoint.txid.to_byte_array().to_vec(), outpoint.vout))
        })
}

fn tx_sanctioned_addresses(
    tx: &Transaction,
    outpoint_to_sanctioned_utxo_map: &HashMap<(Vec<u8>, u32), &shared_model::SanctionedUtxo>,
    sanctioned_addresses: &HashSet<String>,
) -> Vec<String> {
    let mut addresses: Vec<String> = tx_sanctioned_addresses_to(tx, sanctioned_addresses)
        .iter()
        .cloned()
        .chain(tx_sanctioned_addresses_from(
            tx,
            outpoint_to_sanctioned_utxo_map,
        ))
        .collect();
    addresses.sort();
    addresses.dedup();
    addresses
}

fn tx_sanctioned_addresses_to(
    tx: &Transaction,
    sanctioned_addresses: &HashSet<String>,
) -> Vec<String> {
    let mut addresses: HashSet<String> = HashSet::new();
    for out in &tx.output {
        if let Ok(address) = Address::from_script(&out.script_pubkey, Network::Bitcoin) {
            if sanctioned_addresses.contains(&address.to_string()) {
                addresses.insert(address.to_string());
            }
        }
    }
    addresses.iter().cloned().collect()
}

fn tx_sanctioned_addresses_from(
    tx: &Transaction,
    outpoint_to_sanctioned_utxo_map: &HashMap<(Vec<u8>, u32), &shared_model::SanctionedUtxo>,
) -> Vec<String> {
    let mut addresses: HashSet<String> = HashSet::new();
    for input in &tx.input {
        if let Some(utxo) = outpoint_to_sanctioned_utxo_map.get(&(
            input.previous_output.txid.to_byte_array().to_vec(),
            input.previous_output.vout,
        )) {
            if let Ok(address) = Address::from_script(
                &bitcoin::ScriptBuf::from(utxo.script_pubkey.clone()),
                Network::Bitcoin,
            ) {
                addresses.insert(address.to_string());
            }
        }
    }
    addresses.iter().cloned().collect()
}

fn is_tx_opreturn(tx: &Transaction) -> bool {
    for out in &tx.output {
        if out.script_pubkey.is_op_return() {
            return true;
        }
    }
    false
}

pub fn retag_transaction(
    tx: &bitcoin::Transaction,
    tx_info: &TxInfo,
    sanctioned_addresses: &HashSet<String>,
) -> Vec<i32> {
    if let Ok(raw_tx_info) = RawTxInfo::new(tx) {
        let new_tags = get_transaction_tags(
            tx_info,
            &raw_tx_info,
            false,
            &HashMap::new(),
            sanctioned_addresses,
        );
        return new_tags;
    }
    return vec![];
}

fn get_transaction_tags(
    tx_info: &TxInfo,
    raw_tx_info: &RawTxInfo,
    is_conflicting_tx: bool,
    outpoint_to_sanctioned_utxo_map: &HashMap<(Vec<u8>, u32), &shared_model::SanctionedUtxo>,
    sanctioned_addresses: &HashSet<String>,
) -> Vec<i32> {
    let mut tags: Vec<i32> = vec![];

    // Order is important. More servere tags first.
    // Push order is the display order.

    if is_conflicting_tx {
        tags.push(tags::TxTag::Conflicting as i32);
    }

    if is_tx_to_sanctioned(&tx_info.tx, sanctioned_addresses) {
        tags.push(tags::TxTag::ToSanctioned as i32);
    }

    if is_tx_from_sanctioned(&tx_info.tx, outpoint_to_sanctioned_utxo_map) {
        tags.push(tags::TxTag::FromSanctioned as i32);
    }

    if tx_info.tx.is_coinbase() {
        tags.push(tags::TxTag::Coinbase as i32);
    }

    // warnings
    if tx_info.fee.to_sat() == 0 && !tx_info.tx.is_coinbase() {
        tags.push(tags::TxTag::ZeroFee as i32);
    }

    if tx_info.feerate() >= tags::THRESHOLD_FEERATE_CONSIDERED_HIGH {
        tags.push(tags::TxTag::HighFeerate as i32);
    }

    if raw_tx_info.vsize >= tags::THRESHOLD_TRANSACTION_CONSIDERED_LARGE {
        tags.push(tags::TxTag::Large as i32);
    }

    if raw_tx_info.has_non_opretrun_output_smaller_than(Amount::from_sat(
        tags::THRESHOLD_OUTPUT_CONSIDERED_DUST,
    )) {
        tags.push(tags::TxTag::DustOutput as i32);
    }

    if raw_tx_info.output_value_sum() > Amount::from_sat(tags::THRESHOLD_VALUE_CONSIDERED_HIGH) {
        tags.push(tags::TxTag::HighValue as i32);
    }

    if raw_tx_info.sigops() as u64 > tags::THRESHOLD_SIGOPS_CONSIDERED_MANY {
        tags.push(tags::TxTag::ManySigops as i32);
    }

    // normal
    if raw_tx_info.is_spending_segwit() {
        tags.push(tags::TxTag::SegWit as i32);
    }

    if raw_tx_info.is_spending_taproot() {
        tags.push(tags::TxTag::Taproot as i32);
    }

    if raw_tx_info.is_spending_multisig() {
        tags.push(tags::TxTag::Multisig as i32);
    }

    if is_tx_opreturn(&tx_info.tx) {
        tags.push(tags::TxTag::OpReturn as i32);
    }

    if is_p2ms_counterparty(&tx_info.tx)
        || is_opreturn_counterparty(&tx_info.tx)
        || is_p2sh_counterparty(&tx_info.tx)
    {
        tags.push(tags::TxTag::CounterParty as i32);
    }

    if tx_info
        .tx
        .input
        .iter()
        .any(|input| input.reveals_inscription().unwrap_or(false))
    {
        tags.push(tags::TxTag::Inscription as i32);
    }

    if raw_tx_info
        .input_infos
        .iter()
        .any(|input| input.in_type == InputType::P2pk || input.in_type == InputType::P2ms)
        || raw_tx_info.output_infos.iter().any(|output| {
            output.out_type == OutputType::P2pk || output.out_type == OutputType::P2ms
        })
    {
        tags.push(tags::TxTag::RareScriptType as i32);
    }

    if raw_tx_info.is_signaling_explicit_rbf_replicability() {
        tags.push(tags::TxTag::RbfSignaling as i32);
    }

    if raw_tx_info.potentially_coinjoin() {
        tags.push(tags::TxTag::Coinjoin as i32);
    }

    if raw_tx_info.potentially_consolidation() {
        tags.push(tags::TxTag::Consolidation as i32);
    }

    if raw_tx_info.locktime.is_block_height()
        && tx_info.tx.is_lock_time_enabled()
        && raw_tx_info.locktime.to_consensus_u32() > 0
    {
        tags.push(tags::TxTag::LockByHeight as i32);
    }

    // A time based lock time can not be compared to the block height based
    // LockTime::from_consensus(0), the comparison would always be false. It is
    // also always larger than zero, as the smallest possible value is
    // 500_000_000.
    if raw_tx_info.locktime.is_block_time() && tx_info.tx.is_lock_time_enabled() {
        tags.push(tags::TxTag::LockByTimestamp as i32);
    }

    tags
}

fn get_pool_info_or_default(
    result_option: Option<bitcoin_pool_identification::IdentificationResult>,
) -> (String, String, String) {
    if let Some(result) = result_option {
        (
            result.pool.name,
            result.pool.link,
            if result.identification_method == IdentificationMethod::Tag {
                "coinbase tag".to_string()
            } else {
                "coinbase output address".to_string()
            },
        )
    } else {
        ("Unknown".to_string(), String::default(), String::default())
    }
}

/// Builds a vector of transaction packages. These Packages are similar to what
/// Bitcoin Core (currently) puts into blocks.
pub fn build_packages(txns: &[TxInfo]) -> Vec<TxPackage> {
    let txids_in_txns: HashSet<Txid> = txns.iter().map(|ti| ti.tx.compute_txid()).collect();
    let mut packages: Vec<TxPackage> = vec![];

    // Bitcoin Core places transactions that belong to a package next to each other.
    // We consider locality when building packages.
    // Bitcoin Core currently uses the best ancestor set as a package. See
    // https://gist.github.com/murchandamus/5cb413fe9f26dbce57abfd344ebbfaf2#file-asb-vs-csb-png

    let mut package_txns: Vec<TxInfo> = vec![];
    // by iterating in reverse order, we know about the children first
    // This is helpful, as children reference their parents and we can directly link them
    for tx_info in txns.iter().rev() {
        if package_txns.is_empty() {
            package_txns.push(tx_info.clone());
        } else {
            let parent_txids: HashSet<Txid> =
                package_txns.iter().fold(HashSet::new(), |mut txids, tx| {
                    txids.extend(&tx.prev_output_txids());
                    txids
                });
            if parent_txids.contains(&tx_info.tx.compute_txid()) {
                // belongs to the same package if the tx is a parent of a tx in the package
                package_txns.push(tx_info.clone());
            } else if tx_info
                .prev_output_txids()
                .iter()
                .filter(|txid| txids_in_txns.contains(*txid))
                .any(|txid| parent_txids.contains(txid))
            {
                // belongs to the same package if the transaction shares a parent with a
                // transaction already in the package (if this parent is in the `txns` set of
                // transactions)
                package_txns.push(tx_info.clone());
            } else {
                packages.push(TxPackage {
                    txns: package_txns.clone(),
                });
                package_txns.clear();
                package_txns.push(tx_info.clone());
            }
        }
    }
    if !package_txns.is_empty() {
        packages.push(TxPackage {
            txns: package_txns.clone(),
        });
    }
    packages.reverse();

    let total_tx: usize = packages.iter().map(|p| p.txns.len()).sum();
    assert_eq!(txns.len(), total_tx);

    packages
}

pub fn build_transaction(
    reversed_txid: &[u8],
    tx_info: &TxInfo,
    is_conflicting_tx: bool,
    outpoint_to_sanctioned_utxo_map: &HashMap<(Vec<u8>, u32), &shared_model::SanctionedUtxo>,
    sanctioned_addresses: &HashSet<String>,
) -> Result<shared_model::Transaction, RawTxInfoError> {
    let raw_tx_info = RawTxInfo::new(&tx_info.tx)?;
    let (inputs_strs, outputs_strs) = in_and_outputs_to_strings(&raw_tx_info);
    let tags = get_transaction_tags(
        tx_info,
        &raw_tx_info,
        is_conflicting_tx,
        outpoint_to_sanctioned_utxo_map,
        sanctioned_addresses,
    );
    return Ok(shared_model::Transaction {
        txid: reversed_txid.to_vec(),
        sanctioned: is_tx_sanctioned(
            &tx_info.tx,
            outpoint_to_sanctioned_utxo_map,
            sanctioned_addresses,
        ),
        fee: tx_info.fee.to_sat() as i64,
        vsize: tx_info.tx.vsize() as i32,
        output_sum: tx_info
            .tx
            .output
            .iter()
            .map(|o| o.value.to_sat())
            .sum::<u64>() as i64,
        tags,
        input_count: tx_info.tx.input.len() as i32,
        inputs: inputs_strs,
        output_count: tx_info.tx.output.len() as i32,
        outputs: outputs_strs,
        sigops: raw_tx_info.sigops() as i64,
    });
}

pub fn build_transactions_only_in_block(
    block_id: i64,
    txids_only_in_block: &HashSet<&Txid>,
    block_txid_to_txinfo_map: &HashMap<Txid, TxInfo>,
    transactions: &mut HashMap<Vec<u8>, shared_model::Transaction>,
    outpoint_to_sanctioned_utxo_map: &HashMap<(Vec<u8>, u32), &shared_model::SanctionedUtxo>,
    sanctioned_addresses: &HashSet<String>,
) -> Vec<shared_model::TransactionOnlyInBlock> {
    let mut transactions_only_in_block: Vec<shared_model::TransactionOnlyInBlock> = vec![];
    for txid in txids_only_in_block {
        let tx_info = match block_txid_to_txinfo_map.get(*txid) {
            Some(i) => i,
            None => {
                log_processing_error(&format!("Could not find {} in txids_only_in_block in build_transactions_only_in_block().", txid));
                panic!("Could not find {} in txids_only_in_block.", txid);
            }
        };
        let mut txid_to_reverse = tx_info.txid.to_byte_array().to_vec();
        txid_to_reverse.reverse();

        transactions_only_in_block.push(shared_model::TransactionOnlyInBlock {
            block_id,
            position: tx_info.pos,
            transaction_txid: txid_to_reverse.clone(),
        });

        match build_transaction(
            &txid_to_reverse,
            tx_info,
            false,
            outpoint_to_sanctioned_utxo_map,
            sanctioned_addresses,
        ) {
            Ok(t) => add_to_transactions(&t, transactions),
            Err(e) => {
                log_processing_error(&format!(
                    "Could not build transaction {} in build_transactions_only_in_block: {}",
                    txid, e
                ));
            }
        };
    }
    transactions_only_in_block
}

pub fn build_sanctioned_transaction_infos(
    block_id: i64,
    block_tx_data: &BlockTxData,
    template_txids: &HashSet<Txid>,
    template_txid_to_txinfo_map: &HashMap<Txid, TxInfo>,
    txids_only_in_block: &HashSet<&Txid>,
    outpoint_to_sanctioned_utxo_map: &HashMap<(Vec<u8>, u32), &shared_model::SanctionedUtxo>,
    sanctioned_addresses: &HashSet<String>,
    transactions: &mut HashMap<Vec<u8>, shared_model::Transaction>,
) -> Vec<shared_model::SanctionedTransactionInfo> {
    let mut block_sanctioned_transactions = vec![];

    // handle transactions in the template and maybe in the block
    for template_txid in template_txids.iter() {
        let tx_info = match template_txid_to_txinfo_map.get(&template_txid.clone()) {
            Some(i) => i,
            None => {
                log_processing_error(&format!("Could not find {} in template_txid_to_txinfo_map in build_sanctioned_transaction_infos().", template_txid));
                panic!(
                    "Could not find {} in template_txid_to_txinfo_map.",
                    template_txid
                );
            }
        };
        if is_tx_sanctioned(
            &tx_info.tx,
            outpoint_to_sanctioned_utxo_map,
            sanctioned_addresses,
        ) {
            let mut txid_to_reverse = tx_info.txid.to_byte_array().to_vec();
            txid_to_reverse.reverse();

            match build_transaction(
                &txid_to_reverse,
                tx_info,
                false,
                outpoint_to_sanctioned_utxo_map,
                sanctioned_addresses,
            ) {
                Ok(t) => {
                    add_to_transactions(&t, transactions);
                    block_sanctioned_transactions.push(shared_model::SanctionedTransactionInfo {
                        block_id,
                        transaction_txid: txid_to_reverse,
                        in_block: block_tx_data.txids.contains(&tx_info.txid),
                        in_template: true,
                        addresses: tx_sanctioned_addresses(
                            &tx_info.tx,
                            outpoint_to_sanctioned_utxo_map,
                            sanctioned_addresses,
                        ),
                    });
                }
                Err(e) => {
                    log_processing_error(&format!("Could not build transaction {} in build_sanctioned_transaction_infos (in the template and maybe in the block): {}", tx_info.txid, e));
                }
            };
        }
    }

    // handle transactions in only in the block
    for block_txid in txids_only_in_block.iter() {
        let tx_info = match block_tx_data.txid_to_txinfo_map.get(*block_txid) {
            Some(i) => i,
            None => {
                log_processing_error(&format!("Could not find {} in block_txid_to_txinfo_map in build_sanctioned_transaction_infos().", block_txid));
                panic!("Could not find {} in block_txid_to_txinfo_map.", block_txid);
            }
        };
        if is_tx_sanctioned(
            &tx_info.tx,
            outpoint_to_sanctioned_utxo_map,
            sanctioned_addresses,
        ) {
            let mut txid_to_reverse = tx_info.txid.to_byte_array().to_vec();
            txid_to_reverse.reverse();

            match build_transaction(
                &txid_to_reverse,
                tx_info,
                false,
                outpoint_to_sanctioned_utxo_map,
                sanctioned_addresses,
            ) {
                Ok(t) => {
                    add_to_transactions(&t, transactions);
                    block_sanctioned_transactions.push(shared_model::SanctionedTransactionInfo {
                        block_id,
                        transaction_txid: txid_to_reverse,
                        in_block: true,
                        in_template: false,
                        addresses: tx_sanctioned_addresses(
                            &tx_info.tx,
                            outpoint_to_sanctioned_utxo_map,
                            sanctioned_addresses,
                        ),
                    });
                }
                Err(e) => {
                    log_processing_error(&format!( "Could not build transaction {} in build_sanctioned_transaction_infos (only in the block): {}", tx_info.txid, e));
                }
            };
        }
    }

    block_sanctioned_transactions
}

pub fn build_newly_created_sanctioned_utxos(
    block: &bitcoin::Block,
    sanctioned_addresses: &HashSet<String>,
) -> Vec<shared_model::SanctionedUtxo> {
    let mut new_sanctioned_utxos: Vec<shared_model::SanctionedUtxo> = vec![];
    for to_sanctioned_tx in block
        .txdata
        .iter()
        .filter(|tx| is_tx_to_sanctioned(tx, sanctioned_addresses))
    {
        for (vout, output) in to_sanctioned_tx.output.iter().enumerate() {
            if let Ok(address) = Address::from_script(&output.script_pubkey, Network::Bitcoin) {
                if sanctioned_addresses.contains(&address.to_string()) {
                    new_sanctioned_utxos.push(shared_model::SanctionedUtxo {
                        amount: output.value.to_sat() as i64,
                        script_pubkey: output.script_pubkey.to_bytes(),
                        height: block.bip34_block_height().unwrap_or_default() as i32,
                        txid: to_sanctioned_tx
                            .compute_txid()
                            .to_byte_array()
                            .to_vec()
                            .iter()
                            .rev()
                            .cloned()
                            .collect(),
                        vout: vout as i32,
                    })
                }
            }
        }
    }
    new_sanctioned_utxos
}

pub fn build_block_tx_data(
    block: &bitcoin::Block,
    block_tx_fees: &GetBlockTxFeesResult,
) -> BlockTxData {
    let mut txids: HashSet<Txid> = HashSet::new();
    let mut txid_to_txinfo_map: HashMap<Txid, TxInfo> = HashMap::new();
    let mut txinfos: Vec<TxInfo> = vec![];
    let txid_to_fee_map: HashMap<Txid, Amount> = block_tx_fees
        .tx
        .iter()
        .map(|tx| (tx.txid, tx.fee))
        .collect();

    for (i, tx) in block.txdata.iter().enumerate() {
        let txid = tx.compute_txid();
        txids.insert(txid);
        let tx_info = TxInfo {
            txid,
            tx: tx.clone(),
            pos: i as i32,
            fee: txid_to_fee_map[&txid],
        };
        txid_to_txinfo_map.insert(txid, tx_info.clone());
        txinfos.push(tx_info);
    }
    BlockTxData {
        txids,
        txid_to_txinfo_map,
        txinfos,
    }
}

pub fn build_template_tx_data(template: &GetBlockTemplateResult) -> TemplateTxData {
    let mut txids: HashSet<Txid> = HashSet::new();
    let mut txid_to_txinfo_map: HashMap<Txid, TxInfo> = HashMap::new();
    let mut txinfos: Vec<TxInfo> = vec![];
    for (i, tx) in template.transactions.iter().enumerate() {
        let deser_tx: Transaction = match tx.transaction() {
            Ok(t) => t,
            Err(e) => {
                log::error!(
                    "Could not deserialize template transaction index={} tx={:?}: {}",
                    i,
                    tx,
                    e
                );
                continue;
            }
        };
        // The txid is computed from the deserialized transaction. Using the
        // txid from the RPC result here could make the map and the set of
        // txids disagree.
        let txid = deser_tx.compute_txid();
        txids.insert(txid);
        let tx_info = TxInfo {
            txid,
            tx: deser_tx,
            pos: i as i32,
            fee: tx.fee,
        };
        txid_to_txinfo_map.insert(txid, tx_info.clone());
        txinfos.push(tx_info);
    }
    TemplateTxData {
        txids,
        txid_to_txinfo_map,
        txinfos,
    }
}

pub fn build_block(
    block: &bitcoin::Block,
    template: &GetBlockTemplateResult,
    template_txid_to_txinfo_map: &HashMap<Txid, TxInfo>,
    template_pkg_weights: &[i64],
    template_pkg_feerates: &[f32],
    block_pkg_weights: &[i64],
    block_pkg_feerates: &[f32],
    missing_tx: i32,
    sanctioned_missing_tx: i32,
    extra_tx: i32,
    shared_tx: i32,
    block_fees: &Amount,
    template_fees: &Amount,
    outpoint_to_sanctioned_utxo_map: &HashMap<(Vec<u8>, u32), &shared_model::SanctionedUtxo>,
    sanctioned_addresses: &HashSet<String>,
    pools: SharedPoolIDData,
    network: Network,
) -> shared_model::NewBlock {
    let (pool_name, pool_link, pool_id_method) =
        get_pool_info_or_default(block.identify_pool(network, &pools.lock().unwrap()));
    let mut block_hash = block.block_hash().to_byte_array().to_vec();
    block_hash.reverse();
    let mut prev_block_hash = block.header.prev_blockhash.to_byte_array().to_vec();
    prev_block_hash.reverse();

    let block_sigops: i64 = block
        .txdata
        .iter()
        .map(|tx| tx.sigops().unwrap_or_default() as i64)
        .sum();
    let mut block_tags: Vec<i32> = vec![];
    if block_sigops as u64 > tags::THRESHOLD_SIGOP_LIMIT_CLOSE {
        block_tags.push(tags::BlockTag::SigopsLimitClose as i32);
    }
    if block
        .header
        .version
        .is_signalling_soft_fork(VERSION_BIT_TAPROOT)
    {
        block_tags.push(tags::BlockTag::TaprootSignaling as i32);
    }

    shared_model::NewBlock {
        hash: block_hash,
        prev_hash: prev_block_hash,
        height: template.height as i32,
        tags: block_tags,
        extra_tx,
        missing_tx,
        shared_tx,
        sanctioned_missing_tx,
        equality: 0.0, // TODO: can be implemented later, if needed
        block_seen_time: chrono::Utc::now().naive_utc(),
        block_time: chrono::DateTime::from_timestamp(block.header.time as i64, 0)
            .expect("block timestamp out of range")
            .naive_utc(),
        block_tx: block.txdata.len() as i32,
        block_weight: block.weight().to_wu() as i32,
        block_sanctioned: block
            .txdata
            .iter()
            .filter(|tx| {
                is_tx_sanctioned(tx, outpoint_to_sanctioned_utxo_map, sanctioned_addresses)
            })
            .count() as i32,
        block_cb_value: block
            .txdata
            .first()
            .unwrap() // blocks must have at least one transaction
            .output
            .iter()
            .map(|o| o.value.to_sat())
            .sum::<u64>() as i64,
        block_cb_fees: block_fees.to_sat() as i64,
        block_pkg_weights: block_pkg_weights.to_vec(),
        block_pkg_feerates: block_pkg_feerates.to_vec(),
        pool_name,
        pool_link,
        pool_id_method,
        template_cb_value: template.coinbase_value.to_sat() as i64,
        template_time: chrono::DateTime::from_timestamp(template.current_time as i64, 0)
            .expect("template timestamp out of range")
            .naive_utc(),
        template_tx: template.transactions.len() as i32,
        template_sanctioned: template_txid_to_txinfo_map
            .iter()
            .filter(|(_, tx_info)| {
                is_tx_sanctioned(
                    &tx_info.tx,
                    outpoint_to_sanctioned_utxo_map,
                    sanctioned_addresses,
                )
            })
            .count() as i32,
        template_weight: (template
            .transactions
            .iter()
            .map(|tx| tx.weight)
            .sum::<usize>()) as i32,
        template_cb_fees: template_fees.to_sat() as i64,
        template_pkg_weights: template_pkg_weights.to_vec(),
        template_pkg_feerates: template_pkg_feerates.to_vec(),
        template_sigops: template
            .transactions
            .iter()
            .map(|tx| tx.sigops as u64)
            .sum::<u64>() as i64,
        block_sigops: block
            .txdata
            .iter()
            .map(|tx| tx.sigops().unwrap_or_default() as i64)
            .sum(),
    }
}

pub fn build_transactions_only_in_template(
    block_id: i64,
    txids_only_in_template: &HashSet<&Txid>,
    template_txid_to_txinfo_map: &HashMap<Txid, TxInfo>,
    template_txid_to_mempool_age: &HashMap<Txid, i32>,
    transactions: &mut HashMap<Vec<u8>, shared_model::Transaction>,
    outpoint_to_sanctioned_utxo_map: &HashMap<(Vec<u8>, u32), &shared_model::SanctionedUtxo>,
    sanctioned_addresses: &HashSet<String>,
) -> Vec<shared_model::TransactionOnlyInTemplate> {
    let mut transactions_only_in_template: Vec<shared_model::TransactionOnlyInTemplate> = vec![];
    for txid in txids_only_in_template {
        let tx_info = match template_txid_to_txinfo_map.get(*txid) {
            Some(i) => i,
            None => {
                log_processing_error(&format!("Could not find {} in template_txid_to_txinfo_map in build_transactions_only_in_template().", txid));
                panic!("Could not find {} in template_txid_to_txinfo_map.", txid);
            }
        };
        let mut txid_to_reverse = tx_info.txid.to_byte_array().to_vec();
        txid_to_reverse.reverse();

        transactions_only_in_template.push(shared_model::TransactionOnlyInTemplate {
            block_id,
            position: tx_info.pos,
            mempool_age_seconds: *template_txid_to_mempool_age.get(*txid).unwrap_or(&-1),
            transaction_txid: txid_to_reverse.clone(),
        });

        match build_transaction(
            &txid_to_reverse,
            tx_info,
            false,
            outpoint_to_sanctioned_utxo_map,
            sanctioned_addresses,
        ) {
            Ok(t) => add_to_transactions(&t, transactions),
            Err(e) => {
                log_processing_error(&format!(
                    "Could not build transaction {} in build_transactions_only_in_template: {}",
                    tx_info.txid, e
                ));
            }
        }
    }
    transactions_only_in_template
}

fn add_to_transactions(
    new_tx: &shared_model::Transaction,
    transactions: &mut HashMap<Vec<u8>, shared_model::Transaction>,
) {
    if let Some(existing_tx) = transactions.get_mut(&new_tx.txid) {
        if existing_tx.tags != new_tx.tags {
            let mut both_tags: Vec<i32> = existing_tx
                .tags
                .iter()
                .chain(new_tx.tags.iter())
                .cloned()
                .collect();
            both_tags.sort_unstable();
            both_tags.dedup();
            existing_tx.tags = both_tags;
        }
    } else {
        transactions.insert(new_tx.txid.clone(), new_tx.clone());
    }
}

pub fn build_conflicting_transactions(
    block_id: i64,
    txids_only_in_template: &HashSet<&Txid>,
    template_txid_to_txinfo_map: &HashMap<Txid, TxInfo>,
    txids_only_in_block: &HashSet<&Txid>,
    block_txid_to_txinfo_map: &HashMap<Txid, TxInfo>,
    transactions: &mut HashMap<Vec<u8>, shared_model::Transaction>,
    outpoint_to_sanctioned_utxo_map: &HashMap<(Vec<u8>, u32), &shared_model::SanctionedUtxo>,
    sanctioned_addresses: &HashSet<String>,
) -> Vec<shared_model::ConflictingTransaction> {
    // for each conflict:
    // we want to find two sets of transactions, one from the template (set T)
    // and one from the block (set B) where at least one outpoint is in both B and T
    // (this is a conflict)

    let mut template_outpoints_to_txinfo: HashMap<bitcoin::OutPoint, &TxInfo> = HashMap::new();
    let mut block_outpoints_to_txinfo: HashMap<bitcoin::OutPoint, &TxInfo> = HashMap::new();

    let mut template_outpoints: HashSet<bitcoin::OutPoint> = HashSet::new();
    let mut block_outpoints: HashSet<bitcoin::OutPoint> = HashSet::new();

    for txid in txids_only_in_template {
        let tx_info = match template_txid_to_txinfo_map.get(*txid) {
            Some(i) => i,
            None => {
                log_processing_error(&format!("Could not find {} in template_txid_to_txinfo_map in build_conflicting_transactions().", txid));
                panic!("Could not find {} in template_txid_to_txinfo_map.", txid);
            }
        };
        for input in tx_info.tx.input.iter() {
            template_outpoints.insert(input.previous_output);
            template_outpoints_to_txinfo.insert(input.previous_output, tx_info);
        }
    }

    for txid in txids_only_in_block {
        let tx_info = match block_txid_to_txinfo_map.get(*txid) {
            Some(i) => i,
            None => {
                log_processing_error(&format!("Could not find {} in block_txid_to_txinfo_map in build_conflicting_transactions().", txid));
                panic!("Could not find {} in block_txid_to_txinfo_map.", txid);
            }
        };
        for input in tx_info.tx.input.iter() {
            block_outpoints.insert(input.previous_output);
            block_outpoints_to_txinfo.insert(input.previous_output, tx_info);
        }
    }

    // outpoints that are in both sets (interestect) conflict
    let conflicting_outpoints: Vec<&bitcoin::OutPoint> =
        template_outpoints.intersection(&block_outpoints).collect();

    if conflicting_outpoints.is_empty() {
        return vec![];
    }

    #[derive(PartialEq)]
    struct Conflict {
        template_transactions: RefCell<Vec<TxInfo>>,
        block_transactions: RefCell<Vec<TxInfo>>,
        conflicting_outpoints: RefCell<Vec<bitcoin::OutPoint>>,
    }

    let mut conflicts: Vec<Conflict> = vec![];

    for co in conflicting_outpoints {
        let template_transaction = match template_outpoints_to_txinfo.get(co) {
            Some(template_tx) => template_tx,
            None => {
                log_processing_error(&format!("Could not find {:?} in template_outpoints_to_txinfo in build_conflicting_transactions().", co));
                panic!("Could not find {:?} in template_outpoints_to_txinfo.", co);
            }
        };
        let block_transaction = match block_outpoints_to_txinfo.get(co) {
            Some(block_tx) => block_tx,
            None => {
                log_processing_error(&format!("Could not find {:?} in block_outpoints_to_txinfo in build_conflicting_transactions().", co));
                panic!("Could not find {:?} in block_outpoints_to_txinfo.", co);
            }
        };

        // check in which Conflicts the transactions are already present.

        let mut existing_conficts_with_these_transactions: Vec<&Conflict> = vec![];
        for c in conflicts.iter() {
            if c.template_transactions
                .borrow()
                .contains(template_transaction)
                && !existing_conficts_with_these_transactions.contains(&c)
            {
                existing_conficts_with_these_transactions.push(c);
            }
            if c.block_transactions.borrow().contains(block_transaction)
                && !existing_conficts_with_these_transactions.contains(&c)
            {
                existing_conficts_with_these_transactions.push(c);
            }
        }

        match existing_conficts_with_these_transactions.len() {
            0 => {
                conflicts.push(Conflict {
                    template_transactions: RefCell::new(vec![(*template_transaction).clone()]),
                    block_transactions: RefCell::new(vec![(*block_transaction).clone()]),
                    conflicting_outpoints: RefCell::new(vec![*co]),
                });
            }
            // Merge the conflicts the transactions already appear in, the two
            // transactions and the outpoint into the first of the conflicts.
            _ => {
                let c = existing_conficts_with_these_transactions.first().unwrap(); // We can unwrap here as we know there is at least one

                let mut new_template_transactions: Vec<TxInfo> =
                    vec![(*template_transaction).clone()];
                let mut new_block_transactions: Vec<TxInfo> = vec![(*block_transaction).clone()];
                let mut new_conflicting_outpoints: Vec<bitcoin::OutPoint> = vec![*co];
                for c in existing_conficts_with_these_transactions.iter() {
                    new_template_transactions.append(&mut c.template_transactions.borrow_mut());
                    new_block_transactions.append(&mut c.block_transactions.borrow_mut());
                    new_conflicting_outpoints.append(&mut c.conflicting_outpoints.borrow_mut());
                }
                new_template_transactions.sort_unstable_by_key(|tx| tx.txid);
                new_template_transactions.dedup_by_key(|tx| tx.txid);
                c.template_transactions.replace(new_template_transactions);

                new_block_transactions.sort_unstable_by_key(|tx| tx.txid);
                new_block_transactions.dedup_by_key(|tx| tx.txid);
                c.block_transactions.replace(new_block_transactions);

                new_conflicting_outpoints.sort_unstable();
                new_conflicting_outpoints.dedup();
                c.conflicting_outpoints.replace(new_conflicting_outpoints);
            }
        }
        // Merging moves the transactions of the merged conflicts into one of
        // them and leaves the others empty. Drop the empty ones.
        conflicts.retain(|c| !c.template_transactions.borrow().is_empty());
    }

    let mut conflicting_transactions: Vec<shared_model::ConflictingTransaction> = vec![];

    for conflict in conflicts {
        conflicting_transactions.push(shared_model::ConflictingTransaction {
            block_id,
            template_txids: conflict
                .template_transactions
                .borrow()
                .iter()
                .map(|tx| tx.txid.to_byte_array().iter().rev().copied().collect())
                .collect(),
            block_txids: conflict
                .block_transactions
                .borrow()
                .iter()
                .map(|tx| tx.txid.to_byte_array().iter().rev().copied().collect())
                .collect(),
            conflicting_outpoints_txids: conflict
                .conflicting_outpoints
                .borrow()
                .iter()
                .map(|outpoint| {
                    outpoint
                        .txid
                        .to_byte_array()
                        .iter()
                        .rev()
                        .copied()
                        .collect()
                })
                .collect(),
            conflicting_outpoints_vouts: conflict
                .conflicting_outpoints
                .borrow()
                .iter()
                .map(|outpoint| outpoint.vout as i32)
                .collect(),
        });

        for t_tx in conflict.template_transactions.borrow().iter() {
            let mut txid_to_reverse = t_tx.txid.to_byte_array().to_vec();
            txid_to_reverse.reverse();
            match build_transaction(
                &txid_to_reverse,
                t_tx,
                true,
                outpoint_to_sanctioned_utxo_map,
                sanctioned_addresses,
            ) {
                Ok(t) => add_to_transactions(&t, transactions),
                Err(e) => {
                    log_processing_error(&format!(
                        "Could not build transaction {} in build_conflicting_transactions(1): {}",
                        t_tx.txid, e
                    ));
                }
            }
        }

        for t_tx in conflict.block_transactions.borrow().iter() {
            let mut txid_to_reverse = t_tx.txid.to_byte_array().to_vec();
            txid_to_reverse.reverse();
            match build_transaction(
                &txid_to_reverse,
                t_tx,
                true,
                outpoint_to_sanctioned_utxo_map,
                sanctioned_addresses,
            ) {
                Ok(t) => add_to_transactions(&t, transactions),
                Err(e) => {
                    log_processing_error(&format!(
                        "Could not build transaction {} in build_conflicting_transactions(2): {}",
                        t_tx.txid, e
                    ));
                }
            }
        }
    }

    conflicting_transactions
}

pub fn get_sanctioned_missing_tx_count(
    txids_only_in_template: &HashSet<&Txid>,
    data: &TemplateTxData,
    outpoint_to_sanctioned_utxo_map: &HashMap<(Vec<u8>, u32), &shared_model::SanctionedUtxo>,
    sanctioned_addresses: &HashSet<String>,
) -> usize {
    txids_only_in_template
        .iter()
        .map(|txid| data.txid_to_txinfo_map.get(*txid).unwrap())
        .filter(|tx| {
            is_tx_sanctioned(
                &tx.tx,
                outpoint_to_sanctioned_utxo_map,
                sanctioned_addresses,
            )
        })
        .count()
}

pub fn build_debug_template_selection_infos(
    block_id: i64,
    templates: &VecDeque<GetBlockTemplateResult>,
    block_txids: HashSet<Txid>,
    selected_template_time: u64,
) -> Vec<shared_model::DebugTemplateSelectionInfo> {
    templates
        .iter()
        .map(|t| {
            let template_txids: HashSet<Txid> = t.transactions.iter().map(|t| t.txid).collect();
            shared_model::DebugTemplateSelectionInfo {
                block_id,
                template_time: chrono::DateTime::from_timestamp(t.current_time as i64, 0)
                    .expect("template timestamp out of range")
                    .naive_utc(),
                count_missing: template_txids.difference(&block_txids).count() as i32,
                count_shared: block_txids.intersection(&template_txids).count() as i32,
                count_extra: block_txids.difference(&template_txids).count() as i32,
                selected: t.current_time == selected_template_time,
            }
        })
        .collect::<Vec<shared_model::DebugTemplateSelectionInfo>>()
}

pub fn log_processing_error(msg: &str) {
    log::error!(target: LOG_TARGET_PROCESSING, "{}", msg);
    metrics::ERROR_PROCESSING.inc();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TxInfo;
    use crate::test_util::{
        coinbase_tx, deser_tx, fake_tx, op_return_script, outpoint, reversed_txid, sanctioned_utxo,
        script_for_address, spend_of, template, template_tx, tx_info, tx_with_vsize, ADDRESS_P2PKH,
        ADDRESS_P2TR, ADDRESS_P2WPKH, SEQUENCE_FINAL, SEQUENCE_LOCKTIME_ONLY, SEQUENCE_RBF,
    };
    use bitcoin::locktime::absolute::LockTime;
    use bitcoin::BlockHash;
    use bitcoin::{
        consensus, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
    };
    use hex;
    use miningpool_observer_shared::tags::TxTag;
    use rawtx_rs::bitcoin::transaction::Version;
    use std::str::FromStr;

    /// mainnet 4aba5c7fe256a9997aa93cab91f1c0405074b30b34b6a5c0bbde425322d40ee7
    /// spending a P2WPKH output
    const TX_P2WPKH: &str = "01000000000101ce73651d1ef6e687a66a76adbaf16741a205e7230bff0ae7259d36c478ec1a339a00000000ffffffff018a399c0000000000160014a14296a183d6e4c2f1696713a9db833e66fcb2d50247304402204da5741dab6897b961d996c0bdd0fed51f33afa54459769a81019d04a35fb1a802206f47143811a512a73b9d1112f5f0202740c554bf853fe18e2c72e58381c5452b012103e37b04cf32aa417e3ef25c0fe19faefcdbb6e6b2eaca38837dabc4c146cdc95000000000";

    /// signet 75a1a2488770ba0506b9899b1d03dc232f5b22f00dffc3ca3ea6640a53de8403
    /// spending P2TR outputs with key-path spends
    const TX_P2TR_KEYPATH: &str = "0200000000010232b1c6063448c8089d4e7a500399555daec6bc1f3fef281016518c1de3d154cb0000000000feffffff32b1c6063448c8089d4e7a500399555daec6bc1f3fef281016518c1de3d154cb0100000000feffffff02301b0f00000000002251204b03959143386c56a1646c9d1002314c6acecd79ee0f0976fb9fa0f1f0837b1be31f0000000000001600140af8bb24c9504e8740076bb07b755237d4af6e67014159f6076cc04503a9bc72f137aa4af523ab05f5805b5fcb9e0b0b0f258a86afd30c7298e11203f6f27a1408385ec5b9fc1d16be738d628a4aac62eef1cfdac10b0102473044022044d39c6b67334c5e1aa0be056b2200221f8ec84e7319ba1b1cbe6df4dc2f6b1d02207289dd7e3d362355f22ff1df76965391ccd9c4d529a216947d2434f16dfd9b7901210338008b55bf51d06440c64129665a56c2eb828fdad50cca74191d29f92475e962b5680000";

    /// mainnet 949591ad468cef5c41656c0a502d9500671ee421fadb590fbc6373000039b693
    /// spending a 2-of-3 P2MS output
    const TX_P2MS_INPUT: &str = "010000000110a5fee9786a9d2d72c25525e52dd70cbd9035d5152fac83b62d3aa7e2301d58000000009300483045022100af204ef91b8dba5884df50f87219ccef22014c21dd05aa44470d4ed800b7f6e40220428fe058684db1bb2bfb6061bff67048592c574effc217f0d150daedcf36787601483045022100e8547aa2c2a2761a5a28806d3ae0d1bbf0aeff782f9081dfea67b86cacb321340220771a166929469c34959daf726a2ac0c253f9aff391e58a3c7cb46d8b7e0fdc4801ffffffff0180a21900000000001976a914971802edf585cdbc4e57017d6e5142515c1e502888ac00000000";

    /// mainnet ac1d9ed701af32ea52fabd0834acfb1ba4e3584cf0553551f1b61b3d7fb05ee7
    /// paying to a P2MS output
    const TX_P2MS_OUTPUT: &str = "0100000001ffc0d6d6b592cd2b4160300a278ea5e250b5055b5536dcfb2da5dcc46022765a00000000694630430220575ddd235a989befbf98f43b008666e56af07be89e47e09d18690c75846fb587021f00830605aa09febc51132001e0dbcad860e54d4657b55aaf961b527a935b8a01210281feb90c058c3436f8bc361930ae99fcfb530a699cdad141d7244bfcad521a1fffffffff03204e0000000000002551210281feb90c058c3436f8bc361930ae99fcfb530a699cdad141d7244bfcad521a1f51ae204e0000000000001976a914a988f8039a203cf86136e0d32b9d77eafa5a6bef88ac46f4d501000000001976a914161d7a3d0ee15c793ab300433192f949d8f3566588ac00000000";

    /// mainnet f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16
    /// spending and paying to P2PK outputs
    const TX_P2PK: &str = "0100000001c997a5e56e104102fa209c6a852dd90660a20b2d9c352423edce25857fcd3704000000004847304402204e45e16932b8af514961a1d3a1a25fdf3f4f7732e9d624c6c61548ab5fb8cd410220181522ec8eca07de4860a4acdd12909d831cc56cbbac4622082221a8768d1d0901ffffffff0200ca9a3b00000000434104ae1a62fe09c5f51b13905f07f06b99a2f7159b2225f374cd378d71302fa28414e7aab37397f554a7df5f142c21c1b7303b8a0626f1baded5c72a704f7e6cd84cac00286bee0000000043410411db93e1dcdb8a016b49840f8c53bc1eb68a382e97b1482ecad7b148a6909a5cb2e0eaddfb84ccf9744464f82e160bfa9b8b64f9d4c03f999b8643f656b412a3ac00000000";

    /// mainnet e08c3d808317731ef6040799646de2f567590ff890c8fe920a12e36502d8ceb0
    /// a CounterParty transaction with an OP_RETURN output
    const TX_COUNTERPARTY: &str = "0100000001c05b048e0964c3db4a87c33e00503ccdc3d7bff75892639029ba311ce7829745010000006b483045022100df78463aa570274bb2be52ee7410265f5165c0f9f00fb67285582a3e365711ed022074eb1c368569164bb1ad4bca9833b8398e1bb90409e58ac92a98d5ecd117e7e4012103a6e6e5baa76d34499b6b895bccf5a7b5f0d6b264637dd1cddad72cea2c4a1499ffffffff0200000000000000004b6a49f7032ee8d7f20e860d177068a124158cebaa066a209a916e2e6fa26637468bfa7742f6c58724fb1d36aa209aa394245b94afbe385df237e9c4821c6e9d8b9c234f8de5c68760561d03ebd0eb01000000001976a9142e9943921a473dee1e04a579c1762ff6e9ac34e488ac00000000";

    /// mainnet ba4f42037f92c2782ee3dd8c75ce0ce80d8a04b8d36e6ed6a36452f512e66dfd
    /// an ordinals inscription reveal transaction
    const TX_INSCRIPTION: &str = "02000000000101da5159649742d35e069f74724bb72cbb116415d72e11771a17420e2201d4ecf30000000000fdffffff0122020000000000002251201e6960b35d5da5c6c9ce0a18d989518bf546b4968a4ad046d9d664f5585f50440340ad42d4bec479e22866603b11048a64a3e5d366046d494278336798a51fd5fe5166221949e3025b25b505433ed871e20399fd82ffc867b70de8acef8e86f91f857e20318de3d918b6ca5ce115f5b00b3ed0b9c85ca7ebe8f0ccbe9de0e05e45de9293ac0063036f7264010118746578742f706c61696e3b636861727365743d7574662d3800387b2270223a226272632d3230222c226f70223a227472616e73666572222c227469636b223a224d4d5353222c22616d74223a22323030227d6821c1318de3d918b6ca5ce115f5b00b3ed0b9c85ca7ebe8f0ccbe9de0e05e45de929300000000";

    fn sanctioned_address_set(addresses: &[&str]) -> HashSet<String> {
        addresses.iter().map(|a| a.to_string()).collect()
    }

    /// Returns the tags of a transaction that is not conflicting and neither
    /// pays to nor spends from a sanctioned address.
    fn tags_of(tx: &Transaction, fee_sat: u64) -> Vec<i32> {
        let raw_tx_info = RawTxInfo::new(tx).unwrap();
        get_transaction_tags(
            &tx_info(tx, 0, fee_sat),
            &raw_tx_info,
            false,
            &HashMap::new(),
            &HashSet::new(),
        )
    }

    /// A transaction without any tags.
    fn untagged_tx() -> Transaction {
        fake_tx(
            &[outpoint(1, 0)],
            &[(1_000, script_for_address(ADDRESS_P2WPKH))],
            0,
            SEQUENCE_FINAL,
        )
    }

    #[test]
    fn test_get_transaction_tags_without_any_tag() {
        assert_eq!(tags_of(&untagged_tx(), 1_000), Vec::<i32>::new());
    }

    #[test]
    fn test_get_transaction_tags_conflicting() {
        let tx = untagged_tx();
        let raw_tx_info = RawTxInfo::new(&tx).unwrap();
        let tags = get_transaction_tags(
            &tx_info(&tx, 0, 1_000),
            &raw_tx_info,
            true,
            &HashMap::new(),
            &HashSet::new(),
        );
        assert_eq!(tags, vec![TxTag::Conflicting as i32]);
    }

    #[test]
    fn test_get_transaction_tags_to_and_from_sanctioned() {
        let script = script_for_address(ADDRESS_P2WPKH);
        let utxos = vec![sanctioned_utxo(outpoint(5, 1), &script, 1_000)];
        let map = build_outpoint_to_sanctioned_utxo_map(&utxos);
        let sanctioned = sanctioned_address_set(&[ADDRESS_P2WPKH]);

        let to_sanctioned = fake_tx(
            &[outpoint(1, 0)],
            &[(1_000, script.clone())],
            0,
            SEQUENCE_FINAL,
        );
        let raw_tx_info = RawTxInfo::new(&to_sanctioned).unwrap();
        assert_eq!(
            get_transaction_tags(
                &tx_info(&to_sanctioned, 0, 1_000),
                &raw_tx_info,
                false,
                &HashMap::new(),
                &sanctioned,
            ),
            vec![TxTag::ToSanctioned as i32]
        );

        let from_sanctioned = fake_tx(
            &[outpoint(5, 1)],
            &[(1_000, script_for_address(ADDRESS_P2TR))],
            0,
            SEQUENCE_FINAL,
        );
        let raw_tx_info = RawTxInfo::new(&from_sanctioned).unwrap();
        assert_eq!(
            get_transaction_tags(
                &tx_info(&from_sanctioned, 0, 1_000),
                &raw_tx_info,
                false,
                &map,
                &sanctioned,
            ),
            vec![TxTag::FromSanctioned as i32]
        );
    }

    #[test]
    fn test_get_transaction_tags_coinbase_is_not_zero_fee() {
        let coinbase = coinbase_tx(
            800_000,
            b"/test/",
            &[(50_000, script_for_address(ADDRESS_P2WPKH))],
        );
        // A coinbase transaction has no fee, but must not be tagged as a
        // zero fee transaction.
        assert_eq!(tags_of(&coinbase, 0), vec![TxTag::Coinbase as i32]);
    }

    #[test]
    fn test_get_transaction_tags_zero_fee() {
        assert_eq!(tags_of(&untagged_tx(), 0), vec![TxTag::ZeroFee as i32]);
        assert_eq!(tags_of(&untagged_tx(), 1), Vec::<i32>::new());
    }

    #[test]
    fn test_get_transaction_tags_high_feerate() {
        let tx = untagged_tx();
        let vsize = tx.vsize() as u64;
        let at_threshold = (tags::THRESHOLD_FEERATE_CONSIDERED_HIGH as u64) * vsize;

        // the threshold feerate itself is already considered high
        assert_eq!(tags_of(&tx, at_threshold), vec![TxTag::HighFeerate as i32]);
        assert_eq!(tags_of(&tx, at_threshold - 1), Vec::<i32>::new());
    }

    #[test]
    fn test_get_transaction_tags_large() {
        let large = tx_with_vsize(tags::THRESHOLD_TRANSACTION_CONSIDERED_LARGE as usize);
        assert_eq!(
            tags_of(&large, 1_000),
            vec![TxTag::Large as i32, TxTag::OpReturn as i32]
        );

        let not_large = tx_with_vsize(tags::THRESHOLD_TRANSACTION_CONSIDERED_LARGE as usize - 1);
        assert_eq!(tags_of(&not_large, 1_000), vec![TxTag::OpReturn as i32]);
    }

    #[test]
    fn test_get_transaction_tags_dust_output() {
        let script = script_for_address(ADDRESS_P2WPKH);

        let dust = fake_tx(
            &[outpoint(1, 0)],
            &[(tags::THRESHOLD_OUTPUT_CONSIDERED_DUST - 1, script.clone())],
            0,
            SEQUENCE_FINAL,
        );
        assert_eq!(tags_of(&dust, 1_000), vec![TxTag::DustOutput as i32]);

        let no_dust = fake_tx(
            &[outpoint(1, 0)],
            &[(tags::THRESHOLD_OUTPUT_CONSIDERED_DUST, script.clone())],
            0,
            SEQUENCE_FINAL,
        );
        assert_eq!(tags_of(&no_dust, 1_000), Vec::<i32>::new());

        // an OP_RETURN output with a value of 0 is not a dust output
        let opreturn = fake_tx(
            &[outpoint(1, 0)],
            &[
                (tags::THRESHOLD_OUTPUT_CONSIDERED_DUST, script),
                (0, op_return_script(20)),
            ],
            0,
            SEQUENCE_FINAL,
        );
        assert_eq!(tags_of(&opreturn, 1_000), vec![TxTag::OpReturn as i32]);
    }

    #[test]
    fn test_get_transaction_tags_high_value() {
        let script = script_for_address(ADDRESS_P2WPKH);

        // the threshold value itself is not yet considered high
        let at_threshold = fake_tx(
            &[outpoint(1, 0)],
            &[(tags::THRESHOLD_VALUE_CONSIDERED_HIGH, script.clone())],
            0,
            SEQUENCE_FINAL,
        );
        assert_eq!(tags_of(&at_threshold, 1_000), Vec::<i32>::new());

        let above_threshold = fake_tx(
            &[outpoint(1, 0)],
            &[(tags::THRESHOLD_VALUE_CONSIDERED_HIGH + 1, script)],
            0,
            SEQUENCE_FINAL,
        );
        assert_eq!(
            tags_of(&above_threshold, 1_000),
            vec![TxTag::HighValue as i32]
        );
    }

    #[test]
    fn test_get_transaction_tags_many_sigops() {
        // Each P2PKH output counts as four sigops.
        let p2pkh_outputs = |count: usize| -> Vec<(u64, ScriptBuf)> {
            (0..count)
                .map(|_| (1_000u64, script_for_address(ADDRESS_P2PKH)))
                .collect()
        };

        let at_threshold = fake_tx(
            &[outpoint(1, 0)],
            &p2pkh_outputs(tags::THRESHOLD_SIGOPS_CONSIDERED_MANY as usize / 4),
            0,
            SEQUENCE_FINAL,
        );
        assert_eq!(
            RawTxInfo::new(&at_threshold).unwrap().sigops() as u64,
            tags::THRESHOLD_SIGOPS_CONSIDERED_MANY
        );
        // the threshold itself is not yet considered many sigops
        assert!(!tags_of(&at_threshold, 1_000).contains(&(TxTag::ManySigops as i32)));

        let above_threshold = fake_tx(
            &[outpoint(1, 0)],
            &p2pkh_outputs(tags::THRESHOLD_SIGOPS_CONSIDERED_MANY as usize / 4 + 1),
            0,
            SEQUENCE_FINAL,
        );
        assert!(tags_of(&above_threshold, 1_000).contains(&(TxTag::ManySigops as i32)));
    }

    #[test]
    fn test_get_transaction_tags_segwit() {
        let tx = deser_tx(TX_P2WPKH);
        assert_eq!(tags_of(&tx, 1_000), vec![TxTag::SegWit as i32]);
    }

    #[test]
    fn test_get_transaction_tags_taproot() {
        let tx = deser_tx(TX_P2TR_KEYPATH);
        assert_eq!(
            tags_of(&tx, 1_000),
            vec![
                TxTag::SegWit as i32,
                TxTag::Taproot as i32,
                TxTag::LockByHeight as i32,
            ]
        );
    }

    #[test]
    fn test_get_transaction_tags_multisig() {
        let tx = deser_tx(TX_P2MS_INPUT);
        assert_eq!(
            tags_of(&tx, 1_000),
            vec![TxTag::Multisig as i32, TxTag::RareScriptType as i32]
        );
    }

    #[test]
    fn test_get_transaction_tags_opreturn() {
        let tx = fake_tx(
            &[outpoint(1, 0)],
            &[
                (1_000, script_for_address(ADDRESS_P2WPKH)),
                (0, op_return_script(20)),
            ],
            0,
            SEQUENCE_FINAL,
        );
        assert_eq!(tags_of(&tx, 1_000), vec![TxTag::OpReturn as i32]);
    }

    #[test]
    fn test_get_transaction_tags_counterparty() {
        let tx = deser_tx(TX_COUNTERPARTY);
        let tags = tags_of(&tx, 1_000);
        assert!(tags.contains(&(TxTag::CounterParty as i32)));
        assert!(tags.contains(&(TxTag::OpReturn as i32)));
    }

    #[test]
    fn test_get_transaction_tags_inscription() {
        let tx = deser_tx(TX_INSCRIPTION);
        let tags = tags_of(&tx, 1_000);
        assert!(tags.contains(&(TxTag::Inscription as i32)));
        assert!(tags.contains(&(TxTag::Taproot as i32)));
    }

    #[test]
    fn test_get_transaction_tags_rare_script_type() {
        // a transaction spending and paying to P2PK outputs
        let p2pk = deser_tx(TX_P2PK);
        assert!(tags_of(&p2pk, 1_000).contains(&(TxTag::RareScriptType as i32)));

        // a transaction paying to a P2MS output
        let p2ms = deser_tx(TX_P2MS_OUTPUT);
        assert!(tags_of(&p2ms, 1_000).contains(&(TxTag::RareScriptType as i32)));
    }

    #[test]
    fn test_get_transaction_tags_rbf_signaling() {
        let script = script_for_address(ADDRESS_P2WPKH);

        let signaling = fake_tx(
            &[outpoint(1, 0)],
            &[(1_000, script.clone())],
            0,
            SEQUENCE_RBF,
        );
        assert_eq!(tags_of(&signaling, 1_000), vec![TxTag::RbfSignaling as i32]);

        // the highest sequence number that does not signal RBF
        let not_signaling = fake_tx(
            &[outpoint(1, 0)],
            &[(1_000, script)],
            0,
            SEQUENCE_LOCKTIME_ONLY,
        );
        assert_eq!(tags_of(&not_signaling, 1_000), Vec::<i32>::new());
    }

    #[test]
    fn test_get_transaction_tags_coinjoin() {
        let script = script_for_address(ADDRESS_P2WPKH);
        // two inputs and three equal-value outputs
        let tx = fake_tx(
            &[outpoint(1, 0), outpoint(2, 0)],
            &[
                (100_000, script.clone()),
                (100_000, script.clone()),
                (100_000, script.clone()),
                (123_456, script),
            ],
            0,
            SEQUENCE_FINAL,
        );
        assert_eq!(tags_of(&tx, 1_000), vec![TxTag::Coinjoin as i32]);
    }

    #[test]
    fn test_get_transaction_tags_consolidation() {
        let inputs: Vec<OutPoint> = (0..10).map(|i| outpoint(i, 0)).collect();
        let tx = fake_tx(
            &inputs,
            &[(100_000, script_for_address(ADDRESS_P2WPKH))],
            0,
            SEQUENCE_FINAL,
        );
        assert_eq!(tags_of(&tx, 1_000), vec![TxTag::Consolidation as i32]);

        // nine inputs are not yet a consolidation
        let tx = fake_tx(
            &inputs[..9],
            &[(100_000, script_for_address(ADDRESS_P2WPKH))],
            0,
            SEQUENCE_FINAL,
        );
        assert_eq!(tags_of(&tx, 1_000), Vec::<i32>::new());
    }

    #[test]
    fn test_get_transaction_tags_lock_by_height() {
        let script = script_for_address(ADDRESS_P2WPKH);

        let locked = fake_tx(
            &[outpoint(1, 0)],
            &[(1_000, script.clone())],
            500_000,
            SEQUENCE_LOCKTIME_ONLY,
        );
        assert_eq!(tags_of(&locked, 1_000), vec![TxTag::LockByHeight as i32]);

        // the lock time is disabled when all inputs have a final sequence
        let lock_time_disabled = fake_tx(
            &[outpoint(1, 0)],
            &[(1_000, script.clone())],
            500_000,
            SEQUENCE_FINAL,
        );
        assert_eq!(tags_of(&lock_time_disabled, 1_000), Vec::<i32>::new());

        // a lock time of 0 does not lock the transaction
        let no_lock_time = fake_tx(
            &[outpoint(1, 0)],
            &[(1_000, script)],
            0,
            SEQUENCE_LOCKTIME_ONLY,
        );
        assert_eq!(tags_of(&no_lock_time, 1_000), Vec::<i32>::new());
    }

    #[test]
    fn test_get_transaction_tags_lock_by_timestamp() {
        let script = script_for_address(ADDRESS_P2WPKH);

        let locked = fake_tx(
            &[outpoint(1, 0)],
            &[(1_000, script.clone())],
            1_600_000_000,
            SEQUENCE_LOCKTIME_ONLY,
        );
        assert_eq!(tags_of(&locked, 1_000), vec![TxTag::LockByTimestamp as i32]);

        let lock_time_disabled = fake_tx(
            &[outpoint(1, 0)],
            &[(1_000, script)],
            1_600_000_000,
            SEQUENCE_FINAL,
        );
        assert_eq!(tags_of(&lock_time_disabled, 1_000), Vec::<i32>::new());
    }

    #[test]
    fn test_get_transaction_tags_are_ordered_by_severity() {
        let sanctioned_script = script_for_address(ADDRESS_P2WPKH);
        let utxos = vec![sanctioned_utxo(outpoint(5, 1), &sanctioned_script, 1_000)];
        let map = build_outpoint_to_sanctioned_utxo_map(&utxos);
        let sanctioned = sanctioned_address_set(&[ADDRESS_P2WPKH]);

        // a transaction that triggers many of the tags at once
        let mut tx = fake_tx(
            &[outpoint(5, 1), outpoint(6, 0)],
            &[
                (
                    tags::THRESHOLD_OUTPUT_CONSIDERED_DUST - 1,
                    sanctioned_script,
                ),
                (0, op_return_script(2_400)),
            ],
            500_000,
            SEQUENCE_RBF,
        );
        // pad the transaction until it is considered large
        while (tx.vsize() as u64) < tags::THRESHOLD_TRANSACTION_CONSIDERED_LARGE {
            tx.output[1].script_pubkey = op_return_script(tx.output[1].script_pubkey.len() + 100);
        }

        let raw_tx_info = RawTxInfo::new(&tx).unwrap();
        let tags = get_transaction_tags(&tx_info(&tx, 0, 0), &raw_tx_info, true, &map, &sanctioned);

        assert_eq!(
            tags,
            vec![
                TxTag::Conflicting as i32,
                TxTag::ToSanctioned as i32,
                TxTag::FromSanctioned as i32,
                TxTag::ZeroFee as i32,
                TxTag::Large as i32,
                TxTag::DustOutput as i32,
                TxTag::OpReturn as i32,
                TxTag::RbfSignaling as i32,
                TxTag::LockByHeight as i32,
            ]
        );
    }

    #[test]
    fn test_build_outpoint_to_sanctioned_utxo_map() {
        // mainnet f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16
        const TXID: &str = "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16";
        let txid = Txid::from_str(TXID).unwrap();
        // The database stores the txid in display (reversed) byte order.
        let display_order_txid = hex::decode(TXID).unwrap();

        let utxos = vec![shared_model::SanctionedUtxo {
            txid: display_order_txid.clone(),
            vout: 1,
            script_pubkey: script_for_address(ADDRESS_P2WPKH).to_bytes(),
            amount: 1337,
            height: 170,
        }];

        let map = build_outpoint_to_sanctioned_utxo_map(&utxos);
        assert_eq!(map.len(), 1);

        // The key must be the internal byte order, so that it can be looked up
        // with the bytes of an OutPoint txid.
        let utxo = map
            .get(&(txid.to_byte_array().to_vec(), 1))
            .expect("the UTXO should be found by its internal byte order txid");
        assert_eq!(utxo.amount, 1337);
        assert_eq!(utxo.height, 170);

        assert!(!map.contains_key(&(display_order_txid, 1)));
        // the vout is part of the key
        assert!(!map.contains_key(&(txid.to_byte_array().to_vec(), 0)));
    }

    #[test]
    fn test_is_tx_to_sanctioned() {
        let sanctioned = sanctioned_address_set(&[ADDRESS_P2WPKH]);

        let to_sanctioned = fake_tx(
            &[outpoint(1, 0)],
            &[
                (1_000, script_for_address(ADDRESS_P2TR)),
                (1_000, script_for_address(ADDRESS_P2WPKH)),
            ],
            0,
            SEQUENCE_FINAL,
        );
        assert!(is_tx_to_sanctioned(&to_sanctioned, &sanctioned));
        // with an empty list of sanctioned addresses, no transaction is sanctioned
        assert!(!is_tx_to_sanctioned(&to_sanctioned, &HashSet::new()));

        let not_to_sanctioned = fake_tx(
            &[outpoint(1, 0)],
            &[
                (1_000, script_for_address(ADDRESS_P2TR)),
                // an output that can't be represented as an address
                (0, op_return_script(8)),
            ],
            0,
            SEQUENCE_FINAL,
        );
        assert!(!is_tx_to_sanctioned(&not_to_sanctioned, &sanctioned));
    }

    #[test]
    fn test_is_tx_from_sanctioned() {
        let script = script_for_address(ADDRESS_P2WPKH);
        let utxos = vec![sanctioned_utxo(outpoint(5, 1), &script, 1_000)];
        let map = build_outpoint_to_sanctioned_utxo_map(&utxos);

        let from_sanctioned = fake_tx(
            &[outpoint(4, 0), outpoint(5, 1)],
            &[(1_000, script.clone())],
            0,
            SEQUENCE_FINAL,
        );
        assert!(is_tx_from_sanctioned(&from_sanctioned, &map));
        assert!(!is_tx_from_sanctioned(&from_sanctioned, &HashMap::new()));

        // the same txid, but a different vout
        let not_from_sanctioned = fake_tx(&[outpoint(5, 0)], &[(1_000, script)], 0, SEQUENCE_FINAL);
        assert!(!is_tx_from_sanctioned(&not_from_sanctioned, &map));
    }

    #[test]
    fn test_is_tx_sanctioned() {
        let sanctioned_script = script_for_address(ADDRESS_P2PKH);
        let utxos = vec![sanctioned_utxo(outpoint(5, 1), &sanctioned_script, 1_000)];
        let map = build_outpoint_to_sanctioned_utxo_map(&utxos);
        let sanctioned = sanctioned_address_set(&[ADDRESS_P2WPKH]);

        let to_sanctioned = fake_tx(
            &[outpoint(4, 0)],
            &[(1_000, script_for_address(ADDRESS_P2WPKH))],
            0,
            SEQUENCE_FINAL,
        );
        assert!(is_tx_sanctioned(&to_sanctioned, &map, &sanctioned));

        let from_sanctioned = fake_tx(
            &[outpoint(5, 1)],
            &[(1_000, script_for_address(ADDRESS_P2TR))],
            0,
            SEQUENCE_FINAL,
        );
        assert!(is_tx_sanctioned(&from_sanctioned, &map, &sanctioned));

        let unrelated = fake_tx(
            &[outpoint(4, 0)],
            &[(1_000, script_for_address(ADDRESS_P2TR))],
            0,
            SEQUENCE_FINAL,
        );
        assert!(!is_tx_sanctioned(&unrelated, &map, &sanctioned));
    }

    #[test]
    fn test_tx_sanctioned_addresses() {
        let from_script = script_for_address(ADDRESS_P2PKH);
        let utxos = vec![sanctioned_utxo(outpoint(5, 1), &from_script, 1_000)];
        let map = build_outpoint_to_sanctioned_utxo_map(&utxos);
        let sanctioned = sanctioned_address_set(&[ADDRESS_P2WPKH]);

        let tx = fake_tx(
            &[outpoint(5, 1), outpoint(4, 0)],
            &[
                // the same sanctioned address twice
                (1_000, script_for_address(ADDRESS_P2WPKH)),
                (2_000, script_for_address(ADDRESS_P2WPKH)),
                // and an address that is not sanctioned
                (3_000, script_for_address(ADDRESS_P2TR)),
            ],
            0,
            SEQUENCE_FINAL,
        );

        // sorted, deduplicated and containing both the "to" and the "from" address
        assert_eq!(
            tx_sanctioned_addresses(&tx, &map, &sanctioned),
            vec![ADDRESS_P2PKH.to_string(), ADDRESS_P2WPKH.to_string()]
        );

        let unrelated = fake_tx(
            &[outpoint(4, 0)],
            &[(1_000, script_for_address(ADDRESS_P2TR))],
            0,
            SEQUENCE_FINAL,
        );
        assert!(tx_sanctioned_addresses(&unrelated, &map, &sanctioned).is_empty());
    }

    #[test]
    fn test_is_tx_opreturn() {
        let with_opreturn = fake_tx(
            &[outpoint(1, 0)],
            &[
                (1_000, script_for_address(ADDRESS_P2WPKH)),
                (0, op_return_script(20)),
            ],
            0,
            SEQUENCE_FINAL,
        );
        assert!(is_tx_opreturn(&with_opreturn));

        let without_opreturn = fake_tx(
            &[outpoint(1, 0)],
            &[(1_000, script_for_address(ADDRESS_P2WPKH))],
            0,
            SEQUENCE_FINAL,
        );
        assert!(!is_tx_opreturn(&without_opreturn));

        let without_outputs = fake_tx(&[outpoint(1, 0)], &[], 0, SEQUENCE_FINAL);
        assert!(!is_tx_opreturn(&without_outputs));
    }

    #[test]
    fn test_in_and_outputs_to_strings() {
        let tx = fake_tx(
            &[outpoint(1, 0)],
            &[
                (1_000, script_for_address(ADDRESS_P2WPKH)),
                (1_000, script_for_address(ADDRESS_P2TR)),
                (1_000, script_for_address(ADDRESS_P2WPKH)),
            ],
            0,
            SEQUENCE_FINAL,
        );

        // The counts are collected in hash maps, which are iterated in an
        // arbitrary order. Repeat the check a few times to notice when the
        // result is not sorted.
        for _ in 0..20 {
            let raw_tx_info = RawTxInfo::new(&tx).unwrap();
            let (inputs, outputs) = in_and_outputs_to_strings(&raw_tx_info);
            assert_eq!(inputs, vec!["1x P2SH".to_string()]);
            assert_eq!(
                outputs,
                vec!["1x P2TR".to_string(), "2x P2WPKH v0".to_string()]
            );
        }
    }

    /// Builds a transaction spending `parents` (vout 0 of each) plus, when
    /// there is no parent, an outpoint unknown to the test.
    fn child_of(parents: &[&Transaction], unrelated: u8) -> Transaction {
        let inputs: Vec<OutPoint> = if parents.is_empty() {
            vec![outpoint(unrelated, 0)]
        } else {
            parents.iter().map(|p| spend_of(p, 0)).collect()
        };
        fake_tx(
            &inputs,
            &[(10_000, script_for_address(ADDRESS_P2WPKH))],
            0,
            SEQUENCE_FINAL,
        )
    }

    /// Builds a txid to TxInfo map and the set of its txids.
    fn txinfo_map(infos: Vec<TxInfo>) -> HashMap<Txid, TxInfo> {
        infos.into_iter().map(|i| (i.txid, i)).collect()
    }

    #[test]
    fn test_build_transactions_only_in_block() {
        let tx_a = child_of(&[], 100);
        let tx_b = child_of(&[], 101);
        let map = txinfo_map(vec![tx_info(&tx_a, 3, 100), tx_info(&tx_b, 5, 200)]);
        let txids: HashSet<&Txid> = map.keys().collect();

        let mut transactions: HashMap<Vec<u8>, shared_model::Transaction> = HashMap::new();
        let only_in_block = build_transactions_only_in_block(
            42,
            &txids,
            &map,
            &mut transactions,
            &HashMap::new(),
            &HashSet::new(),
        );

        assert_eq!(only_in_block.len(), 2);
        // every transaction is also added to the transactions map, keyed by
        // the reversed txid
        assert_eq!(transactions.len(), 2);
        assert!(transactions.contains_key(&reversed_txid(&tx_a)));
        assert!(transactions.contains_key(&reversed_txid(&tx_b)));

        let a = only_in_block
            .iter()
            .find(|t| t.transaction_txid == reversed_txid(&tx_a))
            .unwrap();
        assert_eq!(a.block_id, 42);
        // the position is taken from the TxInfo
        assert_eq!(a.position, 3);

        let b = only_in_block
            .iter()
            .find(|t| t.transaction_txid == reversed_txid(&tx_b))
            .unwrap();
        assert_eq!(b.position, 5);
    }

    #[test]
    fn test_build_transactions_only_in_template() {
        let tx_a = child_of(&[], 100);
        let tx_b = child_of(&[], 101);
        let map = txinfo_map(vec![tx_info(&tx_a, 3, 100), tx_info(&tx_b, 5, 200)]);
        let txids: HashSet<&Txid> = map.keys().collect();

        // the mempool age is only known for one of the two transactions
        let mut mempool_ages: HashMap<Txid, i32> = HashMap::new();
        mempool_ages.insert(tx_a.compute_txid(), 1337);

        let mut transactions: HashMap<Vec<u8>, shared_model::Transaction> = HashMap::new();
        let only_in_template = build_transactions_only_in_template(
            42,
            &txids,
            &map,
            &mempool_ages,
            &mut transactions,
            &HashMap::new(),
            &HashSet::new(),
        );

        assert_eq!(only_in_template.len(), 2);
        assert_eq!(transactions.len(), 2);
        assert!(transactions.contains_key(&reversed_txid(&tx_a)));
        assert!(transactions.contains_key(&reversed_txid(&tx_b)));

        let a = only_in_template
            .iter()
            .find(|t| t.transaction_txid == reversed_txid(&tx_a))
            .unwrap();
        assert_eq!(a.block_id, 42);
        assert_eq!(a.position, 3);
        assert_eq!(a.mempool_age_seconds, 1337);

        let b = only_in_template
            .iter()
            .find(|t| t.transaction_txid == reversed_txid(&tx_b))
            .unwrap();
        assert_eq!(b.position, 5);
        // an unknown mempool age defaults to -1
        assert_eq!(b.mempool_age_seconds, -1);
    }

    #[test]
    fn test_build_transaction() {
        let sanctioned = sanctioned_address_set(&[ADDRESS_P2WPKH]);
        let tx = fake_tx(
            &[outpoint(1, 0)],
            &[
                (1_000, script_for_address(ADDRESS_P2WPKH)),
                (2_000, script_for_address(ADDRESS_P2TR)),
                (3_000, script_for_address(ADDRESS_P2PKH)),
            ],
            0,
            SEQUENCE_FINAL,
        );
        let info = tx_info(&tx, 7, 123);
        let mut reversed_txid = tx.compute_txid().to_byte_array().to_vec();
        reversed_txid.reverse();

        let built = build_transaction(
            &reversed_txid,
            &info,
            false,
            &HashMap::new(),
            &HashSet::new(),
        )
        .unwrap();

        assert_eq!(built.txid, reversed_txid);
        assert!(!built.sanctioned);
        assert_eq!(built.fee, 123);
        assert_eq!(built.vsize, tx.vsize() as i32);
        assert_eq!(built.output_sum, 6_000);
        assert_eq!(built.input_count, 1);
        assert_eq!(built.output_count, 3);
        // the single P2PKH output is the only output with sigops
        assert_eq!(built.sigops, 4);
        assert_eq!(built.inputs, vec!["1x P2SH".to_string()]);
        assert_eq!(
            built.outputs,
            vec![
                "1x P2PKH".to_string(),
                "1x P2TR".to_string(),
                "1x P2WPKH v0".to_string()
            ]
        );
        assert_eq!(built.tags, Vec::<i32>::new());

        // the same transaction, but now paying to a sanctioned address
        let built =
            build_transaction(&reversed_txid, &info, true, &HashMap::new(), &sanctioned).unwrap();
        assert!(built.sanctioned);
        assert_eq!(
            built.tags,
            vec![TxTag::Conflicting as i32, TxTag::ToSanctioned as i32]
        );
    }

    fn db_transaction(txid: Vec<u8>, tags: Vec<i32>, fee: i64) -> shared_model::Transaction {
        shared_model::Transaction {
            txid,
            sanctioned: false,
            vsize: 100,
            fee,
            output_sum: 1_000,
            tags,
            input_count: 1,
            inputs: vec!["1x P2SH".to_string()],
            output_count: 1,
            outputs: vec!["1x P2WPKH v0".to_string()],
            sigops: 0,
        }
    }

    #[test]
    fn test_add_to_transactions() {
        let txid = vec![0xb1, 0x0c];
        let mut transactions: HashMap<Vec<u8>, shared_model::Transaction> = HashMap::new();

        let first = db_transaction(
            txid.clone(),
            vec![TxTag::SegWit as i32, TxTag::Coinjoin as i32],
            111,
        );
        add_to_transactions(&first, &mut transactions);
        assert_eq!(transactions.len(), 1);
        assert_eq!(
            transactions[&txid].tags,
            vec![TxTag::SegWit as i32, TxTag::Coinjoin as i32]
        );

        // The same transaction with different tags merges the tags. The other
        // fields of the transaction stay untouched.
        let second = db_transaction(
            txid.clone(),
            vec![TxTag::OpReturn as i32, TxTag::Coinjoin as i32],
            222,
        );
        add_to_transactions(&second, &mut transactions);
        assert_eq!(transactions.len(), 1);
        assert_eq!(
            transactions[&txid].tags,
            vec![
                TxTag::Coinjoin as i32,
                TxTag::SegWit as i32,
                TxTag::OpReturn as i32,
            ]
        );
        assert_eq!(transactions[&txid].fee, 111);

        // Adding a transaction with the tags it already has changes nothing.
        let third = db_transaction(txid.clone(), transactions[&txid].tags.clone(), 333);
        add_to_transactions(&third, &mut transactions);
        assert_eq!(transactions.len(), 1);
        assert_eq!(
            transactions[&txid].tags,
            vec![
                TxTag::Coinjoin as i32,
                TxTag::SegWit as i32,
                TxTag::OpReturn as i32,
            ]
        );

        // a different transaction is added as a new entry
        add_to_transactions(&db_transaction(vec![0x42], vec![], 1), &mut transactions);
        assert_eq!(transactions.len(), 2);
    }

    /// Builds the conflicting transaction sets for a list of template and a
    /// list of block transactions.
    fn conflict_sets(
        template_txs: &[&Transaction],
        block_txs: &[&Transaction],
        transactions: &mut HashMap<Vec<u8>, shared_model::Transaction>,
    ) -> Vec<shared_model::ConflictingTransaction> {
        let template_map = txinfo_map(
            template_txs
                .iter()
                .enumerate()
                .map(|(i, tx)| tx_info(tx, i as i32, 1_000))
                .collect(),
        );
        let block_map = txinfo_map(
            block_txs
                .iter()
                .enumerate()
                .map(|(i, tx)| tx_info(tx, i as i32, 1_000))
                .collect(),
        );
        let template_txids: HashSet<&Txid> = template_map.keys().collect();
        let block_txids: HashSet<&Txid> = block_map.keys().collect();

        build_conflicting_transactions(
            42,
            &template_txids,
            &template_map,
            &block_txids,
            &block_map,
            transactions,
            &HashMap::new(),
            &HashSet::new(),
        )
    }

    /// Returns the conflicting outpoints of a set as sorted (txid, vout) pairs.
    fn conflicting_outpoints(set: &shared_model::ConflictingTransaction) -> Vec<(Vec<u8>, i32)> {
        let mut outpoints: Vec<(Vec<u8>, i32)> = set
            .conflicting_outpoints_txids
            .iter()
            .cloned()
            .zip(set.conflicting_outpoints_vouts.iter().cloned())
            .collect();
        outpoints.sort();
        outpoints
    }

    /// Returns the display byte order txid and vout of an outpoint.
    fn reversed_outpoint(outpoint: OutPoint) -> (Vec<u8>, i32) {
        (
            outpoint
                .txid
                .to_byte_array()
                .iter()
                .rev()
                .cloned()
                .collect(),
            outpoint.vout as i32,
        )
    }

    fn sorted(mut txids: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
        txids.sort();
        txids
    }

    /// A transaction spending the passed outpoints and paying `value` sat.
    fn spending(outpoints: &[OutPoint], value: u64) -> Transaction {
        fake_tx(
            outpoints,
            &[(value, script_for_address(ADDRESS_P2WPKH))],
            0,
            SEQUENCE_FINAL,
        )
    }

    #[test]
    fn test_build_conflicting_transactions_without_a_conflict() {
        let template_tx = spending(&[outpoint(1, 0)], 10_000);
        let block_tx = spending(&[outpoint(2, 0)], 20_000);

        let mut transactions: HashMap<Vec<u8>, shared_model::Transaction> = HashMap::new();
        let sets = conflict_sets(&[&template_tx], &[&block_tx], &mut transactions);

        assert!(sets.is_empty());
        assert!(transactions.is_empty());
    }

    #[test]
    fn test_build_conflicting_transactions_with_one_conflict() {
        let template_tx = spending(&[outpoint(1, 0)], 10_000);
        let block_tx = spending(&[outpoint(1, 0)], 20_000);

        let mut transactions: HashMap<Vec<u8>, shared_model::Transaction> = HashMap::new();
        let sets = conflict_sets(&[&template_tx], &[&block_tx], &mut transactions);

        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].block_id, 42);
        assert_eq!(sets[0].template_txids, vec![reversed_txid(&template_tx)]);
        assert_eq!(sets[0].block_txids, vec![reversed_txid(&block_tx)]);
        assert_eq!(
            conflicting_outpoints(&sets[0]),
            vec![reversed_outpoint(outpoint(1, 0))]
        );

        // both transactions are added to the transactions map and tagged as
        // conflicting
        assert_eq!(transactions.len(), 2);
        assert_eq!(
            transactions[&reversed_txid(&template_tx)].tags,
            vec![TxTag::Conflicting as i32]
        );
        assert_eq!(
            transactions[&reversed_txid(&block_tx)].tags,
            vec![TxTag::Conflicting as i32]
        );
    }

    #[test]
    fn test_build_conflicting_transactions_with_one_template_and_two_block_transactions() {
        // The template transaction spends two outpoints, which the block
        // spends with two separate transactions.
        let template_tx = spending(&[outpoint(1, 0), outpoint(2, 0)], 10_000);
        let block_tx1 = spending(&[outpoint(1, 0)], 20_000);
        let block_tx2 = spending(&[outpoint(2, 0)], 30_000);

        let mut transactions: HashMap<Vec<u8>, shared_model::Transaction> = HashMap::new();
        let sets = conflict_sets(
            &[&template_tx],
            &[&block_tx1, &block_tx2],
            &mut transactions,
        );

        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].template_txids, vec![reversed_txid(&template_tx)]);
        assert_eq!(
            sorted(sets[0].block_txids.clone()),
            sorted(vec![reversed_txid(&block_tx1), reversed_txid(&block_tx2)])
        );
        assert_eq!(conflicting_outpoints(&sets[0]), {
            let mut expected = vec![
                reversed_outpoint(outpoint(1, 0)),
                reversed_outpoint(outpoint(2, 0)),
            ];
            expected.sort();
            expected
        });
        assert_eq!(transactions.len(), 3);
    }

    #[test]
    fn test_build_conflicting_transactions_merges_overlapping_conflicts() {
        // T1 spends o1 and o3, T2 spends o2, B1 spends o1 and B2 spends o2
        // and o3. All of them belong to one conflict.
        //
        // The conflicting outpoints are iterated in an arbitrary order, so
        // repeat this a few times to cover the different orders.
        for _ in 0..20 {
            let template_tx1 = spending(&[outpoint(1, 0), outpoint(3, 0)], 10_000);
            let template_tx2 = spending(&[outpoint(2, 0)], 20_000);
            let block_tx1 = spending(&[outpoint(1, 0)], 30_000);
            let block_tx2 = spending(&[outpoint(2, 0), outpoint(3, 0)], 40_000);

            let mut transactions: HashMap<Vec<u8>, shared_model::Transaction> = HashMap::new();
            let sets = conflict_sets(
                &[&template_tx1, &template_tx2],
                &[&block_tx1, &block_tx2],
                &mut transactions,
            );

            assert_eq!(sets.len(), 1);
            assert_eq!(
                sorted(sets[0].template_txids.clone()),
                sorted(vec![
                    reversed_txid(&template_tx1),
                    reversed_txid(&template_tx2)
                ])
            );
            assert_eq!(
                sorted(sets[0].block_txids.clone()),
                sorted(vec![reversed_txid(&block_tx1), reversed_txid(&block_tx2)])
            );
            assert_eq!(conflicting_outpoints(&sets[0]), {
                let mut expected = vec![
                    reversed_outpoint(outpoint(1, 0)),
                    reversed_outpoint(outpoint(2, 0)),
                    reversed_outpoint(outpoint(3, 0)),
                ];
                expected.sort();
                expected
            });
            assert_eq!(transactions.len(), 4);
        }
    }

    #[test]
    fn test_build_conflicting_transactions_are_stored_in_a_stable_order() {
        // The conflicting outpoints are iterated in an arbitrary order, but
        // what ends up in the database must not depend on that order.
        let mut previous: Option<(Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<i32>)> = None;
        for _ in 0..20 {
            let template_tx = spending(&[outpoint(1, 0), outpoint(2, 0), outpoint(3, 0)], 10_000);
            let block_tx1 = spending(&[outpoint(1, 0)], 20_000);
            let block_tx2 = spending(&[outpoint(2, 0)], 30_000);
            let block_tx3 = spending(&[outpoint(3, 0)], 40_000);

            let mut transactions: HashMap<Vec<u8>, shared_model::Transaction> = HashMap::new();
            let sets = conflict_sets(
                &[&template_tx],
                &[&block_tx1, &block_tx2, &block_tx3],
                &mut transactions,
            );

            assert_eq!(sets.len(), 1);
            let current = (
                sets[0].block_txids.clone(),
                sets[0].conflicting_outpoints_txids.clone(),
                sets[0].conflicting_outpoints_vouts.clone(),
            );
            if let Some(previous) = &previous {
                assert_eq!(&current, previous);
            }
            previous = Some(current);
        }
    }

    #[test]
    fn test_build_conflicting_transactions_with_two_independent_conflicts() {
        let template_tx1 = spending(&[outpoint(1, 0)], 10_000);
        let template_tx2 = spending(&[outpoint(2, 0)], 20_000);
        let block_tx1 = spending(&[outpoint(1, 0)], 30_000);
        let block_tx2 = spending(&[outpoint(2, 0)], 40_000);

        let mut transactions: HashMap<Vec<u8>, shared_model::Transaction> = HashMap::new();
        let sets = conflict_sets(
            &[&template_tx1, &template_tx2],
            &[&block_tx1, &block_tx2],
            &mut transactions,
        );

        assert_eq!(sets.len(), 2);
        for set in sets.iter() {
            assert_eq!(set.template_txids.len(), 1);
            assert_eq!(set.block_txids.len(), 1);
            assert_eq!(set.conflicting_outpoints_txids.len(), 1);
        }
        assert_eq!(transactions.len(), 4);
    }

    #[test]
    fn test_build_template_tx_data() {
        let tx_a = child_of(&[], 100);
        let tx_b = child_of(&[], 101);
        let t = template(
            vec![template_tx(&tx_a, 111), template_tx(&tx_b, 222)],
            800_000,
            1_600_000_000,
            BlockHash::all_zeros(),
            300_000,
        );

        let data = build_template_tx_data(&t);

        assert_eq!(data.txinfos.len(), 2);
        assert_eq!(data.txids.len(), 2);
        assert_eq!(data.txid_to_txinfo_map.len(), 2);

        assert_eq!(data.txinfos[0].txid, tx_a.compute_txid());
        assert_eq!(data.txinfos[0].pos, 0);
        assert_eq!(data.txinfos[0].fee, Amount::from_sat(111));
        assert_eq!(data.txinfos[1].pos, 1);
        assert_eq!(data.txinfos[1].fee, Amount::from_sat(222));

        assert!(data.txids.contains(&tx_b.compute_txid()));
        assert!(data.txid_to_txinfo_map.contains_key(&tx_b.compute_txid()));
    }

    #[test]
    fn test_build_template_tx_data_uses_the_computed_txid() {
        let tx = child_of(&[], 100);
        let mut entry = template_tx(&tx, 111);
        // The txid from the RPC result is not used. The set of txids and the
        // map must be keyed by the same, computed, txid.
        entry.txid = Txid::from_byte_array([0xff; 32]);

        let t = template(
            vec![entry],
            800_000,
            1_600_000_000,
            BlockHash::all_zeros(),
            300_000,
        );
        let data = build_template_tx_data(&t);

        assert!(data.txid_to_txinfo_map.contains_key(&tx.compute_txid()));
        assert!(data.txids.contains(&tx.compute_txid()));
        assert!(!data
            .txid_to_txinfo_map
            .contains_key(&Txid::from_byte_array([0xff; 32])));
    }

    #[test]
    fn test_build_template_tx_data_skips_undeserializable_transactions() {
        let tx = child_of(&[], 100);
        let mut broken = template_tx(&tx, 111);
        broken.raw_tx = vec![0x00, 0x01, 0x02];

        let t = template(
            vec![broken, template_tx(&tx, 222)],
            800_000,
            1_600_000_000,
            BlockHash::all_zeros(),
            300_000,
        );
        let data = build_template_tx_data(&t);

        assert_eq!(data.txinfos.len(), 1);
        // the position is still the index in the template
        assert_eq!(data.txinfos[0].pos, 1);
        assert_eq!(data.txinfos[0].fee, Amount::from_sat(222));
    }

    #[test]
    fn test_build_packages_without_transactions() {
        assert!(build_packages(&[]).is_empty());
    }

    #[test]
    fn test_build_packages_with_a_single_transaction() {
        let tx = child_of(&[], 100);
        let packages = build_packages(&[tx_info(&tx, 0, 1_000)]);

        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].txns.len(), 1);
        assert_eq!(packages[0].weight(), tx.weight().to_wu() as usize);
    }

    #[test]
    fn test_build_packages_with_unrelated_transactions() {
        let tx_a = child_of(&[], 100);
        let tx_b = child_of(&[], 101);

        let packages = build_packages(&[tx_info(&tx_a, 0, 1_000), tx_info(&tx_b, 1, 2_000)]);

        assert_eq!(packages.len(), 2);
        // the packages are in the order the transactions were passed in
        assert_eq!(packages[0].txns[0].txid, tx_a.compute_txid());
        assert_eq!(packages[1].txns[0].txid, tx_b.compute_txid());
        assert_eq!(packages[0].weight(), tx_a.weight().to_wu() as usize);
        assert_eq!(packages[1].weight(), tx_b.weight().to_wu() as usize);
    }

    #[test]
    fn test_build_packages_with_a_chain_of_transactions() {
        // A <- B <- C
        let tx_a = child_of(&[], 100);
        let tx_b = child_of(&[&tx_a], 0);
        let tx_c = child_of(&[&tx_b], 0);

        let packages = build_packages(&[
            tx_info(&tx_a, 0, 1_000),
            tx_info(&tx_b, 1, 1_000),
            tx_info(&tx_c, 2, 1_000),
        ]);

        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].txns.len(), 3);
        assert_eq!(
            packages[0].weight(),
            (tx_a.weight() + tx_b.weight() + tx_c.weight()).to_wu() as usize
        );
    }

    #[test]
    fn test_build_packages_with_sibling_transactions() {
        // B and C both spend an output of A
        let tx_a = child_of(&[], 100);
        let tx_b = fake_tx(
            &[spend_of(&tx_a, 0)],
            &[(10_000, script_for_address(ADDRESS_P2WPKH))],
            0,
            SEQUENCE_FINAL,
        );
        let tx_c = fake_tx(
            &[spend_of(&tx_a, 0)],
            &[(20_000, script_for_address(ADDRESS_P2WPKH))],
            0,
            SEQUENCE_FINAL,
        );

        let packages = build_packages(&[
            tx_info(&tx_a, 0, 1_000),
            tx_info(&tx_b, 1, 1_000),
            tx_info(&tx_c, 2, 1_000),
        ]);

        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].txns.len(), 3);
    }

    #[test]
    fn test_build_packages_with_a_parent_that_is_not_adjacent() {
        // Bitcoin Core places the transactions of a package next to each
        // other and build_packages assumes that. A child that is not next to
        // its parent ends up in its own package.
        let tx_a = child_of(&[], 100);
        let tx_d = child_of(&[], 101);
        let tx_b = child_of(&[&tx_a], 0);

        let packages = build_packages(&[
            tx_info(&tx_a, 0, 1_000),
            tx_info(&tx_d, 1, 1_000),
            tx_info(&tx_b, 2, 1_000),
        ]);

        assert_eq!(packages.len(), 3);
    }

    #[test]
    fn test_build_packages_package_feerate() {
        let tx_a = child_of(&[], 100);
        let tx_b = child_of(&[&tx_a], 0);

        let packages = build_packages(&[tx_info(&tx_a, 0, 1_000), tx_info(&tx_b, 1, 3_000)]);

        assert_eq!(packages.len(), 1);
        assert_eq!(
            packages[0].feerate(),
            4_000f32 / (tx_a.vsize() + tx_b.vsize()) as f32
        );
    }
    #[test]
    fn test_build_packages() {
        #[derive(Debug)]
        struct TxData {
            pub tx: Transaction,
            pub fee: Amount,
            pub weight: usize,
        }

        // Transaction A: mainnet 93e7e834d39507b62970e60bc759b0a569018c73ee6e128f5d29c36946361369
        // Parent of Transaction B
        let tx_a = TxData{
            tx: consensus::deserialize(&hex::decode("0200000000010130ffdf7aeecea96a99b7d85ac7012303c857f875d9c42466f874988573987b50000000001716001414644002d74912a7a15dd805cb556ea80979a79ffdffffff0240cb0d000000000017a914bedb237b964e6c796ff8c0b17d2970cb3672cd5687e274920d0000000017a91472945bddbf1debbd7e7fb61595db1e99e879d59a8702473044022062a607723e4bc48d4de0e7045934b7fb39e7159977881fae67c20b510128323d0220498c36ac95e661babe8ae0530deabe473f8d9af35d607fb52c15e5cfc34b3838012103f598c3b4546df0489ccef83d1d8df01b84391b2ad9c873c1d4fbf77467e192da3e680a00").unwrap()).unwrap(),
            fee: Amount::from_sat(664),
            weight: 661,
        };
        let tx_a_info = TxInfo {
            fee: tx_a.fee,
            pos: 0,
            txid: tx_a.tx.compute_txid(),
            tx: tx_a.tx,
        };

        // Transaction B: mainnet a0c8533add348dbd41021d6e381293647a76a260bf83a4c856fc87876401ee08
        // Child of Transaction A; Parent of Transaction C
        let tx_b = TxData{
            tx: consensus::deserialize(&hex::decode("020000000001016913364669c3295d8f126eee738c0169a5b059c70be67029b60795d334e8e79301000000171600143a452352d2fd631d3f979c79cf3c63e94d64f4abfdffffff02c8af0000000000001976a914b941c8c785e67f73635d9c83a3f2bd65753c311d88ac7ac2910d0000000017a9147fd0bfab1a3786516fc4e02624e6fb54f5d21d5d870247304402203b942af4b5ff7bd69d2636281e7f10fe3c1def035f2c98fd008045e15e92ee2a022029ce693f33869a0f9bd718fdede8d2ec7b9849fd19b01be7cb15f13e103e6dbc012102ca8d97fa9aa179c8c3014bde667b4f3661268287ced1094312ba1e867897ca273e680a00").unwrap()).unwrap(),
            fee: Amount::from_sat(672),
            weight: 669,
        };
        let tx_b_info = TxInfo {
            fee: tx_b.fee,
            pos: 1,
            txid: tx_b.tx.compute_txid(),
            tx: tx_b.tx,
        };

        // Transaction C: mainnet 20a45b2c31d8a63cb74dd853f4b4fe11271c77a6f5881a27278cc5771f0fcf52
        // Child of Transaction B
        let tx_c = TxData{
            tx: consensus::deserialize(&hex::decode("0200000000010108ee01648787fc56c8a483bf60a2767a649312386e1d0241bd8d34dd3a53c8a00100000017160014166e50f80b861d5fd3d000ab5b9ce948cb5e4c40fdffffff02b25b870d0000000017a9149b5188d943902cf6173cf69afcb7befe28b1ee868728640a00000000001976a914593380eb004b7378753680dc6326592cc490b35488ac0247304402205af35efb1902a5d71bb95d620e18fc7777a6bd7d70f2ff974258d1ba0c7b6697022051359fff9622a35da32276e532f4277950566c154974952282e75419f241294c012103c50ac2d147f034571916cb6500acfb4050efd428a8eec97168720a7c9576e545f0670a00").unwrap()).unwrap(),
            fee: Amount::from_sat(672),
            weight: 669,
        };
        let tx_c_info = TxInfo {
            fee: tx_c.fee,
            pos: 2,
            txid: tx_c.tx.compute_txid(),
            tx: tx_c.tx,
        };

        // Transaction D: mainnet a9b5a1e4eeec01bc73b5a30bc2991f8a57205f460dfc06cf6f4201d4d88a407c
        // Not related to A, B, or C
        let tx_d = TxData{
            tx: consensus::deserialize(&hex::decode("010000000001017732ae7bafe815d92db701c4a4ed05b0633fc0c2f359ad74029cfa482beac68f2e00000000feffffff01e5d107000000000017a914cebcfd263dea12d7cd2bb9644abf266d01444be2870247304402204c682bf7390cde3671bbde6e1af3013a9c6231bb9e74dfe9ecedab4b677338c80220543c99de42a651069527f56aeb2fd45ff8ed7fd24092a37cba0ad3cff2674e7a0121036e8ae0830fd289baff279aa2911e2fc2a9d2dd536d00fdc53047778455b73ea600000000").unwrap()).unwrap(),
            fee: Amount::from_sat(9324),
            weight: 441,
        };
        let tx_d_info = TxInfo {
            fee: tx_d.fee,
            pos: 2,
            txid: tx_d.tx.compute_txid(),
            tx: tx_d.tx,
        };

        // A: 93e7e834d39507b62970e60bc759b0a569018c73ee6e128f5d29c36946361369
        // B: a0c8533add348dbd41021d6e381293647a76a260bf83a4c856fc87876401ee08
        // C: 20a45b2c31d8a63cb74dd853f4b4fe11271c77a6f5881a27278cc5771f0fcf52
        // D: a9b5a1e4eeec01bc73b5a30bc2991f8a57205f460dfc06cf6f4201d4d88a407c
        //
        // 'X <- Y' means Y spends X; X is the partent of Y
        // A <- B <- C
        let mut txns = vec![tx_a_info.clone(), tx_b_info.clone(), tx_c_info, tx_d_info]; // order matters
        let packages = build_packages(&txns.clone());

        println!("There should be exactly two packages");
        assert_eq!(packages.len(), 2);

        println!("There should be three transactions in the package");
        let package = packages.first().unwrap();
        assert_eq!(package.txns.len(), 3);

        println!("The package weight should be the sum of the three transaction weights");
        assert_eq!(package.weight(), tx_a.weight + tx_b.weight + tx_c.weight);

        // fake tx that spends from A and B
        let tx_e = Transaction {
            lock_time: LockTime::from_consensus(0),
            version: Version::ONE,
            input: vec![
                TxIn {
                    previous_output: OutPoint {
                        txid: tx_a_info.txid,
                        vout: 0,
                    },
                    witness: Witness::new(),
                    script_sig: ScriptBuf::from_bytes(vec![]),
                    sequence: Sequence(1337),
                },
                TxIn {
                    previous_output: OutPoint {
                        txid: tx_b_info.txid,
                        vout: 0,
                    },
                    witness: Witness::new(),
                    script_sig: ScriptBuf::from_bytes(vec![]),
                    sequence: Sequence(1338),
                },
            ],
            output: vec![TxOut {
                script_pubkey: ScriptBuf::from_bytes(vec![]),
                value: Amount::ONE_BTC,
            }],
        };

        let tx_e_info = TxInfo {
            fee: Amount::from_sat(2),
            pos: 3,
            txid: tx_e.compute_txid(),
            tx: tx_e.clone(),
        };

        txns.insert(3, tx_e_info); // order matters

        let packages = build_packages(&txns);

        println!("There should be still exactly two packages");
        assert_eq!(packages.len(), 2);

        println!("There should be now four transactions in the package");
        let package = packages.first().unwrap();
        assert_eq!(package.txns.len(), 4);

        println!("The package weight should be the sum of the four transaction weights");
        assert_eq!(
            package.weight(),
            tx_a.weight + tx_b.weight + tx_c.weight + tx_e.weight().to_wu() as usize
        );
    }
}
