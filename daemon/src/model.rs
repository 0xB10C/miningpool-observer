use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use bitcoin_pool_identification::Pool;
use miningpool_observer_shared::bitcoincore_rpc::bitcoin::{hash_types::Txid, Amount, Transaction};

pub type SharedPoolIDData = Arc<Mutex<Vec<Pool>>>;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TxInfo {
    pub txid: Txid,
    pub tx: Transaction,
    /// Position of the Transaction in the block or the template
    pub pos: i32,
    pub fee: Amount,
}

impl TxInfo {
    pub fn feerate(&self) -> f32 {
        self.fee.to_sat() as f32 / self.tx.vsize() as f32
    }

    pub fn prev_output_txids(&self) -> HashSet<Txid> {
        self.tx
            .input
            .iter()
            .map(|input| input.previous_output.txid)
            .collect()
    }
}

pub struct TxPackage {
    pub txns: Vec<TxInfo>,
}

impl TxPackage {
    pub fn feerate(&self) -> f32 {
        let vsize_sum: f64 = self.txns.iter().map(|i| i.tx.vsize() as f64).sum();
        let fee_sum: f64 = self.txns.iter().map(|i| i.fee.to_sat() as f64).sum();
        (fee_sum / vsize_sum) as f32
    }

    pub fn weight(&self) -> usize {
        self.txns
            .iter()
            .map(|i| i.tx.weight().to_wu() as usize)
            .sum::<usize>()
    }
}

pub struct BlockTxData {
    pub txids: HashSet<Txid>,
    pub txid_to_txinfo_map: HashMap<Txid, TxInfo>,
    pub txinfos: Vec<TxInfo>,
}

pub struct TemplateTxData {
    pub txids: HashSet<Txid>,
    pub txid_to_txinfo_map: HashMap<Txid, TxInfo>,
    pub txinfos: Vec<TxInfo>,
}
