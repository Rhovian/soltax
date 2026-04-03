use soltax_common::{EnhancedTransaction, SOL_MINT, TaxEvent, TaxEventKind};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const YIELD_TOKENS_FILE: &str = "data/yield_tokens.json";
const NON_TAXABLE_SOURCES_FILE: &str = "data/non_taxable_sources.json";

fn load_string_list(path: &str) -> HashSet<String> {
    let p = Path::new(path);
    if !p.exists() {
        return HashSet::new();
    }
    let data = fs::read_to_string(p).unwrap_or_default();
    serde_json::from_str::<Vec<String>>(&data)
        .unwrap_or_default()
        .into_iter()
        .collect()
}

fn is_swap(tx: &EnhancedTransaction) -> bool {
    tx.tx_type.as_deref() == Some("SWAP")
}

const NON_TAXABLE_TX_TYPES: &[&str] = &[
    "FLASH_REPAY_RESERVE_LIQUIDITY",
    "REFRESH_OBLIGATION",
    "DEPOSIT",
    "WITHDRAW",
];

fn is_non_taxable_type(tx: &EnhancedTransaction) -> bool {
    tx.tx_type.as_deref().map(|t| NON_TAXABLE_TX_TYPES.contains(&t)).unwrap_or(false)
}

/// Extract tax-relevant events from a single transaction.
/// `wallet` is the user's address — only movements to/from this address matter.
pub fn extract(
    tx: &EnhancedTransaction,
    wallet: &str,
    yield_tokens: &HashSet<String>,
    non_taxable_sources: &HashSet<String>,
) -> Vec<TaxEvent> {
    let ts = match tx.timestamp {
        Some(ts) => ts,
        None => return vec![],
    };

    if is_non_taxable_type(tx) {
        return vec![];
    }

    let sig = &tx.signature;
    let mut events = Vec::new();

    let tx_is_swap = is_swap(tx);
    let source_is_non_taxable = tx.source.as_ref()
        .map(|s| non_taxable_sources.contains(s))
        .unwrap_or(false);

    // Native SOL transfers
    let mut sol_in: u64 = 0;
    let mut sol_out: u64 = 0;
    for nt in &tx.native_transfers {
        if nt.to_user_account.as_deref() == Some(wallet) {
            sol_in += nt.amount;
        }
        if nt.from_user_account.as_deref() == Some(wallet) {
            sol_out += nt.amount;
        }
    }

    // Fee is already included in sol_out (the fee payer sends lamports),
    // but we track it separately so we can attribute the cost.
    let fee_lamports = if tx.fee_payer.as_deref() == Some(wallet) {
        tx.fee.unwrap_or(0)
    } else {
        0
    };

    // Subtract fee from sol_out so we don't double-count it
    let sol_out_ex_fee = sol_out.saturating_sub(fee_lamports);

    if sol_in > 0 {
        events.push(TaxEvent {
            timestamp: ts,
            signature: sig.clone(),
            kind: TaxEventKind::Acquisition,
            mint: SOL_MINT.to_string(),
            amount: sol_in as f64 / 1_000_000_000.0,
        });
    }
    if sol_out_ex_fee > 0 {
        events.push(TaxEvent {
            timestamp: ts,
            signature: sig.clone(),
            kind: TaxEventKind::Disposal,
            mint: SOL_MINT.to_string(),
            amount: sol_out_ex_fee as f64 / 1_000_000_000.0,
        });
    }
    if fee_lamports > 0 {
        events.push(TaxEvent {
            timestamp: ts,
            signature: sig.clone(),
            kind: TaxEventKind::Fee,
            mint: SOL_MINT.to_string(),
            amount: fee_lamports as f64 / 1_000_000_000.0,
        });
    }

    // Token transfers — aggregate per mint to avoid double-counting
    // when a swap routes through multiple pools
    let mut token_in: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut token_out: std::collections::HashMap<String, f64> = std::collections::HashMap::new();

    for tt in &tx.token_transfers {
        let mint = match &tt.mint {
            Some(m) => m.clone(),
            None => continue,
        };
        // Skip wrapped SOL token transfers — already counted via native transfers
        if mint == SOL_MINT {
            continue;
        }
        if tt.token_amount <= 0.0 {
            continue;
        }

        if yield_tokens.contains(&mint) && !tx_is_swap {
            continue;
        }
        if source_is_non_taxable && !tx_is_swap {
            continue;
        }

        let is_in = tt.to_user_account.as_deref() == Some(wallet);
        let is_out = tt.from_user_account.as_deref() == Some(wallet);

        if is_in {
            *token_in.entry(mint).or_default() += tt.token_amount;
        } else if is_out {
            *token_out.entry(mint).or_default() += tt.token_amount;
        }
    }

    for (mint, amount) in &token_in {
        if *amount > 0.0 {
            events.push(TaxEvent {
                timestamp: ts,
                signature: sig.clone(),
                kind: TaxEventKind::Acquisition,
                mint: mint.clone(),
                amount: *amount,
            });
        }
    }
    for (mint, amount) in &token_out {
        if *amount > 0.0 {
            events.push(TaxEvent {
                timestamp: ts,
                signature: sig.clone(),
                kind: TaxEventKind::Disposal,
                mint: mint.clone(),
                amount: *amount,
            });
        }
    }

    events
}

/// Extract events from all transactions, sorted by timestamp ascending.
pub fn extract_all(txs: &[EnhancedTransaction], wallet: &str) -> Vec<TaxEvent> {
    let yield_tokens = load_string_list(YIELD_TOKENS_FILE);
    let non_taxable_sources = load_string_list(NON_TAXABLE_SOURCES_FILE);
    if !yield_tokens.is_empty() {
        eprintln!("{} yield tokens configured (deposit/withdraw ignored)", yield_tokens.len());
    }
    if !non_taxable_sources.is_empty() {
        eprintln!("{} non-taxable sources configured", non_taxable_sources.len());
    }
    let mut events: Vec<TaxEvent> = txs.iter().flat_map(|tx| extract(tx, wallet, &yield_tokens, &non_taxable_sources)).collect();
    events.sort_by_key(|e| e.timestamp);
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use soltax_common::{NativeTransfer, TokenTransfer};

    fn swap_tx() -> EnhancedTransaction {
        EnhancedTransaction {
            signature: "swap1".into(),
            timestamp: Some(1735700000),
            description: String::new(),
            tx_type: Some("SWAP".into()),
            source: Some("JUPITER".into()),
            fee: Some(5000),
            fee_payer: Some("wallet".into()),
            native_transfers: vec![
                NativeTransfer {
                    from_user_account: Some("wallet".into()),
                    to_user_account: Some("pool".into()),
                    amount: 1_000_000_000,
                },
            ],
            token_transfers: vec![
                TokenTransfer {
                    from_user_account: Some("pool".into()),
                    to_user_account: Some("wallet".into()),
                    from_token_account: None,
                    to_token_account: None,
                    token_amount: 100.0,
                    mint: Some("USDC_MINT".into()),
                },
            ],
        }
    }

    #[test]
    fn swap_produces_acquisition_disposal_and_fee() {
        let yt = HashSet::new();
        let ns = HashSet::new();
        let events = extract(&swap_tx(), "wallet", &yt, &ns);
        assert_eq!(events.len(), 3);

        let disposal = events.iter().find(|e| e.kind == TaxEventKind::Disposal).unwrap();
        assert_eq!(disposal.mint, SOL_MINT);
        assert!((disposal.amount - 0.999995).abs() < 0.000001);

        let acq = events.iter().find(|e| e.kind == TaxEventKind::Acquisition).unwrap();
        assert_eq!(acq.mint, "USDC_MINT");
        assert!((acq.amount - 100.0).abs() < 0.001);

        let fee = events.iter().find(|e| e.kind == TaxEventKind::Fee).unwrap();
        assert_eq!(fee.mint, SOL_MINT);
        assert!((fee.amount - 0.000005).abs() < 0.0000001);
    }

    #[test]
    fn yield_token_ignored_on_deposit() {
        let mut yt = HashSet::new();
        yt.insert("MSOL_MINT".to_string());
        let ns = HashSet::new();

        let tx = EnhancedTransaction {
            signature: "deposit1".into(),
            timestamp: Some(1735700000),
            description: String::new(),
            tx_type: Some("TRANSFER".into()),
            source: Some("KAMINO_LEND".into()),
            fee: Some(5000),
            fee_payer: Some("wallet".into()),
            native_transfers: vec![],
            token_transfers: vec![
                TokenTransfer {
                    from_user_account: Some("wallet".into()),
                    to_user_account: Some("vault".into()),
                    from_token_account: None,
                    to_token_account: None,
                    token_amount: 100.0,
                    mint: Some("MSOL_MINT".into()),
                },
            ],
        };

        let events = extract(&tx, "wallet", &yt, &ns);
        // Should only have the fee, no disposal of MSOL
        assert!(events.iter().all(|e| e.mint != "MSOL_MINT"));
    }

    #[test]
    fn yield_token_taxed_on_swap() {
        let mut yt = HashSet::new();
        yt.insert("MSOL_MINT".to_string());
        let ns = HashSet::new();

        let tx = EnhancedTransaction {
            signature: "swap2".into(),
            timestamp: Some(1735700000),
            description: String::new(),
            tx_type: Some("SWAP".into()),
            source: Some("JUPITER".into()),
            fee: Some(5000),
            fee_payer: Some("wallet".into()),
            native_transfers: vec![],
            token_transfers: vec![
                TokenTransfer {
                    from_user_account: Some("wallet".into()),
                    to_user_account: Some("pool".into()),
                    from_token_account: None,
                    to_token_account: None,
                    token_amount: 100.0,
                    mint: Some("MSOL_MINT".into()),
                },
                TokenTransfer {
                    from_user_account: Some("pool".into()),
                    to_user_account: Some("wallet".into()),
                    from_token_account: None,
                    to_token_account: None,
                    token_amount: 5000.0,
                    mint: Some("USDC".into()),
                },
            ],
        };

        let events = extract(&tx, "wallet", &yt, &ns);
        let msol_disposal = events.iter().find(|e| e.mint == "MSOL_MINT" && e.kind == TaxEventKind::Disposal);
        assert!(msol_disposal.is_some());
    }
}
