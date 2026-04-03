use soltax_common::{EnhancedTransaction, SOL_MINT, TaxEvent, TaxEventKind};

/// Extract tax-relevant events from a single transaction.
/// `wallet` is the user's address — only movements to/from this address matter.
pub fn extract(tx: &EnhancedTransaction, wallet: &str) -> Vec<TaxEvent> {
    let ts = match tx.timestamp {
        Some(ts) => ts,
        None => return vec![],
    };
    let sig = &tx.signature;
    let mut events = Vec::new();

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

    // Token transfers
    for tt in &tx.token_transfers {
        let mint = match &tt.mint {
            Some(m) => m.clone(),
            None => continue,
        };
        if tt.token_amount <= 0.0 {
            continue;
        }
        let is_in = tt.to_user_account.as_deref() == Some(wallet);
        let is_out = tt.from_user_account.as_deref() == Some(wallet);

        if is_in {
            events.push(TaxEvent {
                timestamp: ts,
                signature: sig.clone(),
                kind: TaxEventKind::Acquisition,
                mint,
                amount: tt.token_amount,
            });
        } else if is_out {
            events.push(TaxEvent {
                timestamp: ts,
                signature: sig.clone(),
                kind: TaxEventKind::Disposal,
                mint,
                amount: tt.token_amount,
            });
        }
    }

    events
}

/// Extract events from all transactions, sorted by timestamp ascending.
pub fn extract_all(txs: &[EnhancedTransaction], wallet: &str) -> Vec<TaxEvent> {
    let mut events: Vec<TaxEvent> = txs.iter().flat_map(|tx| extract(tx, wallet)).collect();
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
                    amount: 1_000_000_000, // 1 SOL out
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
        let events = extract(&swap_tx(), "wallet");
        assert_eq!(events.len(), 3);

        let disposal = events.iter().find(|e| e.kind == TaxEventKind::Disposal).unwrap();
        assert_eq!(disposal.mint, SOL_MINT);
        // 1 SOL minus 5000 lamport fee
        assert!((disposal.amount - 0.999995).abs() < 0.000001);

        let acq = events.iter().find(|e| e.kind == TaxEventKind::Acquisition).unwrap();
        assert_eq!(acq.mint, "USDC_MINT");
        assert!((acq.amount - 100.0).abs() < 0.001);

        let fee = events.iter().find(|e| e.kind == TaxEventKind::Fee).unwrap();
        assert_eq!(fee.mint, SOL_MINT);
        assert!((fee.amount - 0.000005).abs() < 0.0000001);
    }
}
