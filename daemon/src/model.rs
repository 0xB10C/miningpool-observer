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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{fake_tx, outpoint, tx_info, SEQUENCE_FINAL};
    use miningpool_observer_shared::bitcoincore_rpc::bitcoin::ScriptBuf;

    #[test]
    fn test_txinfo_feerate() {
        let tx = fake_tx(
            &[outpoint(1, 0)],
            &[(1_000, ScriptBuf::new())],
            0,
            SEQUENCE_FINAL,
        );
        let vsize = tx.vsize() as u64;

        // a fee of 10 sat per vByte
        assert_eq!(tx_info(&tx, 0, 10 * vsize).feerate(), 10.0);
        // a fee of half a sat per vByte
        assert_eq!(tx_info(&tx, 0, vsize / 2).feerate(), 0.5);
        assert_eq!(tx_info(&tx, 0, 0).feerate(), 0.0);
    }

    #[test]
    fn test_txinfo_prev_output_txids_deduplicates() {
        // two inputs spending two outputs of the same parent transaction
        let tx = fake_tx(
            &[outpoint(1, 0), outpoint(1, 7)],
            &[(1_000, ScriptBuf::new())],
            0,
            SEQUENCE_FINAL,
        );
        let prev_txids = tx_info(&tx, 0, 0).prev_output_txids();
        assert_eq!(prev_txids.len(), 1);
        assert!(prev_txids.contains(&outpoint(1, 0).txid));

        // two inputs spending outputs of two different parent transactions
        let tx = fake_tx(
            &[outpoint(1, 0), outpoint(2, 0)],
            &[(1_000, ScriptBuf::new())],
            0,
            SEQUENCE_FINAL,
        );
        assert_eq!(tx_info(&tx, 0, 0).prev_output_txids().len(), 2);
    }

    #[test]
    fn test_txpackage_feerate_is_not_the_average_of_the_feerates() {
        // a small transaction paying a high fee
        let small_tx = fake_tx(
            &[outpoint(1, 0)],
            &[(1_000, ScriptBuf::new())],
            0,
            SEQUENCE_FINAL,
        );
        // and a larger transaction paying no fee at all
        let large_tx = fake_tx(
            &[outpoint(2, 0)],
            &[
                (1_000, ScriptBuf::new()),
                (1_000, ScriptBuf::new()),
                (1_000, ScriptBuf::new()),
                (1_000, ScriptBuf::new()),
            ],
            0,
            SEQUENCE_FINAL,
        );
        let small_vsize = small_tx.vsize() as f32;
        let large_vsize = large_tx.vsize() as f32;
        assert!(small_vsize < large_vsize);

        let package = TxPackage {
            txns: vec![tx_info(&small_tx, 0, 10_000), tx_info(&large_tx, 1, 0)],
        };

        assert_eq!(package.feerate(), 10_000f32 / (small_vsize + large_vsize));
        // the feerate of the package is not the average of the two feerates
        assert_ne!(package.feerate(), (10_000f32 / small_vsize) / 2.0);
    }

    #[test]
    fn test_txpackage_weight_is_the_sum_of_the_transaction_weights() {
        let tx1 = fake_tx(
            &[outpoint(1, 0)],
            &[(1_000, ScriptBuf::new())],
            0,
            SEQUENCE_FINAL,
        );
        let tx2 = fake_tx(
            &[outpoint(2, 0)],
            &[(1_000, ScriptBuf::new()), (2_000, ScriptBuf::new())],
            0,
            SEQUENCE_FINAL,
        );

        let package = TxPackage {
            txns: vec![tx_info(&tx1, 0, 1), tx_info(&tx2, 1, 2)],
        };

        assert_eq!(
            package.weight(),
            tx1.weight().to_wu() as usize + tx2.weight().to_wu() as usize
        );
    }
}
