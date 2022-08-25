use std::convert::TryFrom;

use serde::Serialize;

const COIN: u64 = 100_000_000;
pub const THRESHOLD_TRANSACTION_CONSIDERED_LARGE: u64 = 2500; // vByte
pub const THRESHOLD_FEERATE_CONSIDERED_HIGH: f32 = 1000.0; // sat/vByte
pub const THRESHOLD_OUTPUT_CONSIDERED_DUST: u64 = 1000; // sat
pub const THRESHOLD_VALUE_CONSIDERED_HIGH: u64 = 100 * COIN; // sat (100 BTC)
pub const THRESHOLD_TRANSACTION_CONSIDERED_YOUNG: u64 = 90; // seconds

type BootstrapColor = &'static str;
// const BLUE: BootstrapColor = "primary";
pub const GRAY: BootstrapColor = "secondary";
pub const YELLOW: BootstrapColor = "warning";
pub const RED: BootstrapColor = "danger";
pub const GREEN: BootstrapColor = "success";
pub const CYAN: BootstrapColor = "info";
pub const BLACK: BootstrapColor = "dark";
pub const WHITE: BootstrapColor = "white";
// const LIGHT: BootstrapColor = "light";

#[derive(Serialize, Eq, PartialEq, Hash)]
pub struct Tag {
    pub name: String,
    pub description: Vec<String>,
    pub color: BootstrapColor,
    pub text_color: BootstrapColor,
}

pub enum TxTag {
    // the value is important for database backwards compatibilty
    // make sure to add new tag to the try_from fn below!

    // important / danger (1000-1999)
    FromSanctioned = 1099,
    ToSanctioned = 1100,
    Conflicting = 1110,

    // warning (2000-2999)
    Large = 2100,
    ZeroFee = 2110,
    HighFeerate = 2120,
    HighValue = 2130,

    // informational (3000-3999)
    // DEPRECATED = 3100, (can be reused)
    Young = 3110,

    // make sure to add new tag to the try_from fn below!

    // secondary (4000-4999)
    Coinbase = 4099,
    Coinjoin = 4100,
    SegWit = 4110,
    Taproot = 4111,
    Multisig = 4120,
    RbfSignaling = 4130,
    OpReturn = 4140,
    CounterParty = 4141,
    LockByHeight = 4150,
    LockByTimestamp = 4160,
    Consolidation = 4170,
    DustOutput = 4180,
}

impl TryFrom<i32> for TxTag {
    type Error = ();

    fn try_from(v: i32) -> Result<Self, Self::Error> {
        match v {
            x if x == TxTag::FromSanctioned as i32 => Ok(TxTag::FromSanctioned),
            x if x == TxTag::ToSanctioned as i32 => Ok(TxTag::ToSanctioned),
            x if x == TxTag::Coinbase as i32 => Ok(TxTag::Coinbase),
            x if x == TxTag::Large as i32 => Ok(TxTag::Large),
            x if x == TxTag::ZeroFee as i32 => Ok(TxTag::ZeroFee),
            x if x == TxTag::HighFeerate as i32 => Ok(TxTag::HighFeerate),
            x if x == TxTag::SegWit as i32 => Ok(TxTag::SegWit),
            x if x == TxTag::Taproot as i32 => Ok(TxTag::Taproot),
            x if x == TxTag::Multisig as i32 => Ok(TxTag::Multisig),
            x if x == TxTag::RbfSignaling as i32 => Ok(TxTag::RbfSignaling),
            x if x == TxTag::OpReturn as i32 => Ok(TxTag::OpReturn),
            x if x == TxTag::CounterParty as i32 => Ok(TxTag::CounterParty),
            x if x == TxTag::Coinjoin as i32 => Ok(TxTag::Coinjoin),
            x if x == TxTag::LockByHeight as i32 => Ok(TxTag::LockByHeight),
            x if x == TxTag::LockByTimestamp as i32 => Ok(TxTag::LockByTimestamp),
            x if x == TxTag::Consolidation as i32 => Ok(TxTag::Consolidation),
            x if x == TxTag::DustOutput as i32 => Ok(TxTag::DustOutput),
            x if x == TxTag::HighValue as i32 => Ok(TxTag::HighValue),
            x if x == TxTag::Conflicting as i32 => Ok(TxTag::Conflicting),
            x if x == TxTag::Young as i32 => Ok(TxTag::Young),
            // FIXME: add new tags here
            _ => Err(()),
        }
    }
}

impl TxTag {
    pub const TX_TAGS: &'static [TxTag; 20] = &[
        // important / danger
        TxTag::FromSanctioned,
        TxTag::ToSanctioned,
        TxTag::Conflicting,
        // warning
        TxTag::Large,
        TxTag::ZeroFee,
        TxTag::HighFeerate,
        TxTag::HighValue,
        // informational
        TxTag::Young,
        // secondary
        TxTag::Coinbase,
        TxTag::SegWit,
        TxTag::Taproot,
        TxTag::Multisig,
        TxTag::RbfSignaling,
        TxTag::OpReturn,
        TxTag::CounterParty,
        TxTag::LockByHeight,
        TxTag::LockByTimestamp,
        TxTag::Coinjoin,
        TxTag::Consolidation,
        TxTag::DustOutput,
    ];

    pub fn value(&self) -> Tag {
        match self {
            TxTag::SegWit => {
                Tag {
                    name: "SegWit spending".to_string(),
                    description: vec!["The transaction has at least one SegWit input.".to_string()],
                    color: GRAY,
                    text_color: WHITE,
                }
            },
            TxTag::Taproot => {
                Tag {
                    name: "Taproot spending".to_string(),
                    description: vec!["The transaction has at least one Taproot input.".to_string()],
                    color: GRAY,
                    text_color: WHITE,
                }
            },
            TxTag::Multisig => {
                Tag {
                    name: "Mulitisig spending".to_string(),
                    description: vec!["The transaction has at least one input spending a MultiSig script.".to_string()],
                    color: GRAY,
                    text_color: WHITE,
                }
            },
            TxTag::RbfSignaling => {
                Tag {
                    name: "RBF signaling".to_string(),
                    description: vec![
                        "The transaction signal explicit Replace-By-Fee replaceability.".to_string(),
                        "This means the sender can replace the transaction as long as it is unconfirmed.".to_string(),
                        r#"
                        <ul>
                            <li><a target="_blank" rel="noopener" href='https://bitcoinops.org/en/topics/replace-by-fee/'>Bitcoin Optech: Replace-by-fee (RBF)</a></li>
                        </ul>
                        "#.to_string(),
                        ],
                    color: GRAY,
                    text_color: WHITE,
                }
            },
            TxTag::OpReturn => {
                Tag {
                    name: "OP_RETURN".to_string(),
                    description: vec![
                        "The transaction has an OP_RETURN output.".to_string(),
                        "OP_RETURN outputs embed data into the blockchain.".to_string(),
                        ],
                    color: GRAY,
                    text_color: WHITE,
                }
            },
            TxTag::CounterParty => {
                Tag {
                    name: "CounterParty".to_string(),
                    description: vec![
                        "The transaction has at least one CounterParty output.".to_string(),
                        "CounterParty is a protocol building on-top of Bitcoin.".to_string(),
                    ],
                    color: GRAY,
                    text_color: WHITE,
                }
            },
            TxTag::LockByHeight => {
                Tag {
                    name: "Height-Locked".to_string(),
                    description: vec!["The transaction is time-locked to only be valid after a certain absolute block height.".to_string()],
                    color: GRAY,
                    text_color: WHITE,
                }
            },
            TxTag::LockByTimestamp => {
                Tag {
                    name: "Timestamp-Locked".to_string(),
                    description: vec!["The transaction is time-locked to only be valid after a certain absolute timestamp.".to_string()],
                    color: GRAY,
                    text_color: WHITE,
                }
            },
            TxTag::Coinbase => {
                Tag {
                    name: "Coinbase".to_string(),
                    description: vec!["The transaction is a coinbase transaction paying the mining pool.".to_string()],
                    color: GRAY,
                    text_color: WHITE,
                }
            },
            TxTag::FromSanctioned => {
                Tag {
                    name: "From Sanctioned".to_string(),
                    description: vec![
                            "The transaction spends an UTXO from a sanctioned address.".to_string(),
                            "Some mining pools adhere to the sanctions and won't include transactions from sanctioned addresses in their blocks.".to_string(),
                        ],
                    color: RED,
                    text_color: WHITE,
                }
            },
            TxTag::ToSanctioned => {
                Tag {
                    name: "To Sanctioned".to_string(),
                    description: vec![
                            "The transaction pays to a sanctioned address.".to_string(),
                            "Some mining pools adhere to the sanctions and won't include transactions to sanctioned addresses in their blocks.".to_string(),
                        ],
                    color: RED,
                    text_color: WHITE,
                }
            },
            TxTag::Coinjoin => {
                Tag {
                    name: "Potential Coinjoin".to_string(),
                    description: vec![
                        "The transaction meets the characteristics of a coinjoin with multiple equal-value outputs.".to_string(),
                        r#"
                            <ul>
                            <li><a target="_blank" rel="noopener" href='https://en.bitcoin.it/Privacy#CoinJoin'>Bitcoin Wiki: Privacy - Coinjoin</a></li>
                            <li><a target="_blank" rel="noopener" href='https://bitcoinops.org/en/topics/coinjoin/'>Bitcoin Optech: Coinjoin</a></li>
                            </ul>
                        "#.to_string(),
                        ],
                    color: GRAY,
                    text_color: WHITE,
                }
            },
            TxTag::Large => {
                Tag {
                    name: "Large".to_string(),
                    description: vec![format!("The transaction is equal to or larger than {} vByte in vsize.", THRESHOLD_TRANSACTION_CONSIDERED_LARGE)],
                    color: YELLOW,
                    text_color: BLACK,
                }
            },
            TxTag::HighFeerate => {
                Tag {
                    name: "High-Feerate".to_string(),
                    description: vec![format!("The transaction has a feerate higher than {} sat/vByte.", THRESHOLD_FEERATE_CONSIDERED_HIGH)],
                    color: YELLOW,
                    text_color: BLACK,
                }
            },
            TxTag::ZeroFee => {
                Tag {
                    name: "Zero-Fee".to_string(),
                    description: vec![
                        "The transaction does not pay any fees and is not a coinbase transaction.".to_string(),
                        "Zero-fee transactions aren't relayed through the Bitcoin P2P network.".to_string(),
                        "The pool likely <a target=\"_blank\" rel=\"noopener\" href='https://btcinformation.org/en/developer-reference#prioritisetransaction'>prioritized</a> it to add it to the block.".to_string(),
                        ],
                    color: YELLOW,
                    text_color: BLACK,
                }
            },
            TxTag::Consolidation => {
                Tag {
                    name: "Consolidation".to_string(),
                    description: vec![
                        "The transaction meets the characteristics of a consolidation transaction.".to_string(),
                        r#"
                        <ul>
                            <li><a target="_blank" rel="noopener" href='https://en.bitcoin.it/wiki/Techniques_to_reduce_transaction_fees#Consolidation'>Bitcoin Wiki: Techniques to reduce transaction fees - Consolidation</a></li>
                        </ul>
                        "#.to_string(),
                        ],
                    color: GRAY,
                    text_color: WHITE,
                }
            },
            TxTag::DustOutput => {
                Tag {
                    name: "Has Dust Output".to_string(),
                    description: vec![
                        format!("The transaction has at least one output smaller than {} satoshi.", THRESHOLD_OUTPUT_CONSIDERED_DUST),
                        "It's often not economical to spend 'dust outputs' as the fees a spender needs to pay are a large part of the value of the output.".to_string(),
                        "Some miners might choose to filter transactions with (only) dust outputs.".to_string(),
                        "<b>Note:</b> The dust threshold is client-specific and not a network rule.".to_string(),
                        r#"
                        <ul>
                            <li><a target="_blank" rel="noopener" href='https://bitcoin.stackexchange.com/questions/10986/what-is-meant-by-bitcoin-dust'>Bitcoin StackExchange: What is meant by Bitcoin dust?</a></li>
                        </ul>
                        "#.to_string(),
                        ],
                    color: GRAY,
                    text_color: WHITE,
                }
            },
            TxTag::HighValue => {
                Tag {
                    name: "High-Value".to_string(),
                    description: vec![
                            format!("The transaction has a value higher than {} BTC.", THRESHOLD_VALUE_CONSIDERED_HIGH / COIN),
                        ],
                    color: YELLOW,
                    text_color: BLACK,
                }
            },
            TxTag::Conflicting => {
                Tag {
                    name: "Conflicting".to_string(),
                    description: vec![
                            "The transaction conflicts with a transaction from the other set.".to_string(),
                            "If this transaction is in a template, then it conflicts with a transaction in the block.".to_string(),
                            "If this transaction is in a block, then it conflicts with a transaction in the template.".to_string(),
                            "A conflicting tranasction <i>could</i> indicate an attempted zero-confirmation double-spend attack.".to_string(),
                            "For example, a mallicius party could have sent two conflicting transactions to a merchant and a mining pool.".to_string(),
                            "<b>Note:</b> This should not be confused with a protocol-level double-spend where conflicting transactions are accepted into the chain".to_string(),
                        ],
                    color: RED,
                    text_color: WHITE,
                }
            },
            TxTag::Young => {
                Tag {
                    name: "Young".to_string(),
                    description: vec![
                            format!("The transaction hasn't been in the mempool for longer than {} seconds.", THRESHOLD_TRANSACTION_CONSIDERED_YOUNG),
                            "If this transaction wasn't included in a block, it could be that the transaction hadn't propagated to the pool when the block was constructed.".to_string(),
                        ],
                    color: CYAN,
                    text_color: WHITE,
                }
            },
        }
    }
}

pub enum BlockTag {
    // the value is important for database backwards compatibilty
    // make sure to add new tag to the try_from fn below!

    // important / danger (1000-1999)
    // StartHere = 1100,

    // warning (2000-2999)
    // StartHere = 2100,

    // informational (3000-3999)
    TaprootSignaling = 3100,
    // secondary (4000-4999)
    // StartHere = 4100,
}

impl TryFrom<i32> for BlockTag {
    type Error = ();

    fn try_from(v: i32) -> Result<Self, Self::Error> {
        match v {
            x if x == BlockTag::TaprootSignaling as i32 => Ok(BlockTag::TaprootSignaling),
            // FIXME: add new tags here
            _ => Err(()),
        }
    }
}

impl BlockTag {
    pub const BLOCK_TAGS: &'static [BlockTag; 1] = &[
        // important / danger
        //
        // warning
        //
        // informational
        BlockTag::TaprootSignaling,
        // secondary
        //
    ];

    pub fn value(&self) -> Tag {
        match self {
            BlockTag::TaprootSignaling => Tag {
                name: "Taproot Signaling".to_string(),
                description: vec!["The block signals for Taproot.".to_string()],
                color: CYAN,
                text_color: WHITE,
            },
        }
    }
}
