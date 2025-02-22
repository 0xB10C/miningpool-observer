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
    hash_types::Txid, hashes::Hash, locktime::absolute::LockTime, network::Network, Address,
    Amount, Transaction,
};

pub const LOG_TARGET_PROCESSING: &str = "processing";

const VERSION_BIT_TAPROOT: u8 = 2;

fn in_and_outputs_to_strings(raw_tx_info: &RawTxInfo) -> (Vec<String>, Vec<String>) {
    let mut output_type_counts: HashMap<OutputType, u32> = HashMap::new();
    for ot in raw_tx_info.output_infos.iter().map(|i| i.out_type) {
        if let Some(count) = output_type_counts.clone().get(&ot) {
            output_type_counts.insert(ot, *count + 1);
        } else {
            output_type_counts.insert(ot, 1);
        }
    }

    let mut input_type_counts: HashMap<InputType, u32> = HashMap::new();
    for it in raw_tx_info.input_infos.iter().map(|i| i.in_type) {
        if let Some(count) = input_type_counts.clone().get(&it) {
            input_type_counts.insert(it, *count + 1);
        } else {
            input_type_counts.insert(it, 1);
        }
    }

    let inputs_strs: Vec<String> = input_type_counts
        .iter()
        .map(|(k, v)| format!("{}x {}", v, k))
        .collect();
    let outputs_strs: Vec<String> = output_type_counts
        .iter()
        .map(|(k, v)| format!("{}x {}", v, k))
        .collect();

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

    if tx_info.tx.sigops().unwrap_or_default() as u64 > tags::THRESHOLD_SIGOPS_CONSIDERED_MANY {
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
        && raw_tx_info.locktime > LockTime::from_consensus(0)
    {
        tags.push(tags::TxTag::LockByHeight as i32);
    }

    if raw_tx_info.locktime.is_block_time()
        && tx_info.tx.is_lock_time_enabled()
        && raw_tx_info.locktime > LockTime::from_consensus(0)
    {
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
        sigops: tx_info.tx.sigops().unwrap_or_default() as i64,
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
        txids.insert(deser_tx.compute_txid());
        let tx_info = TxInfo {
            txid: deser_tx.compute_txid(),
            tx: deser_tx,
            pos: i as i32,
            fee: tx.fee,
        };
        txid_to_txinfo_map.insert(tx.txid, tx_info.clone());
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
            1 => {
                let c = existing_conficts_with_these_transactions.first().unwrap(); // We can unwrap here as we know there is a first (as there is exactly 1)

                if !c
                    .template_transactions
                    .borrow()
                    .contains(template_transaction)
                {
                    c.template_transactions
                        .borrow_mut()
                        .push((*template_transaction).clone());
                }

                if !c.block_transactions.borrow().contains(block_transaction) {
                    c.block_transactions
                        .borrow_mut()
                        .push((*block_transaction).clone());
                }

                if !c.conflicting_outpoints.borrow().contains(co) {
                    c.conflicting_outpoints.borrow_mut().push(*co);
                }
            }
            _ => {
                let c = existing_conficts_with_these_transactions.first().unwrap(); // We can unwrap here as we know there are multiple

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
        conflicts.retain(|c| !c.template_transactions.borrow().is_empty());
        conflicts.retain(|c| !c.template_transactions.borrow().is_empty());
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
    use bitcoin::{
        consensus, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
    };
    use hex;
    use rawtx_rs::bitcoin::transaction::Version;

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
