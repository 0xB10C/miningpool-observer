use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use bitcoin::hash_types::Txid;
use bitcoin::{Amount, Transaction};

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
        let vsize: f32 = self.tx.get_weight() as f32 / 4.0_f32;
        self.fee.as_sat() as f32 / vsize
    }
}

pub struct TxPackage {
    pub txns: Vec<TxInfo>,
}

impl TxPackage {
    pub fn feerate(&self) -> f32 {
        let vsize_sum: f64 = self
            .txns
            .iter()
            .map(|i| (i.tx.get_weight() as f64 / 4.0_f64).ceil())
            .sum();
        let fee_sum: f64 = self.txns.iter().map(|i| i.fee.as_sat() as f64).sum();
        (fee_sum / vsize_sum) as f32
    }

    pub fn weight(&self) -> usize {
        self.txns.iter().map(|i| i.tx.get_weight()).sum::<usize>()
    }
}

#[derive(PartialEq, Eq, Clone)]
pub struct TxPackageForPackageConstruction {
    pub txns: RefCell<Vec<TxInfo>>,
}

impl TxPackageForPackageConstruction {
    pub fn min_tx_pos(&self) -> i32 {
        self.txns
            .borrow()
            .iter()
            .map(|tx_info| tx_info.pos)
            .min()
            .unwrap_or(-1)
    }

    pub fn txids(&self) -> HashSet<Txid> {
        self.txns.borrow().iter().map(|t| t.txid).collect()
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
