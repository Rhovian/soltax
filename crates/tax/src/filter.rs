use soltax_common::EnhancedTransaction;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const EXCLUDED_SIGS_FILE: &str = "data/excluded_signatures.json";

fn load_excluded_signatures() -> HashSet<String> {
    let path = Path::new(EXCLUDED_SIGS_FILE);
    if !path.exists() {
        return HashSet::new();
    }
    let data = fs::read_to_string(path).unwrap_or_default();
    // Support both formats: array of strings or object with reasons
    if let Ok(arr) = serde_json::from_str::<Vec<String>>(&data) {
        return arr.into_iter().collect();
    }
    if let Ok(obj) = serde_json::from_str::<std::collections::HashMap<String, String>>(&data) {
        return obj.into_keys().collect();
    }
    HashSet::new()
}

pub fn apply(txs: Vec<EnhancedTransaction>, wallet: &str) -> Vec<EnhancedTransaction> {
    let excluded = load_excluded_signatures();
    txs.into_iter()
        .filter(|tx| !excluded.contains(&tx.signature))
        .filter(|tx| !is_spam(tx, wallet))
        .collect()
}

fn is_spam(tx: &EnhancedTransaction, wallet: &str) -> bool {
    // Drop BUBBLEGUM (compressed NFT spam)
    if tx.source.as_deref() == Some("BUBBLEGUM") {
        return true;
    }

    // Drop transactions with zero value flow for this wallet
    let has_native = tx.native_transfers.iter().any(|nt| {
        nt.amount > 0
            && (nt.from_user_account.as_deref() == Some(wallet)
                || nt.to_user_account.as_deref() == Some(wallet))
    });
    let has_token = tx.token_transfers.iter().any(|tt| {
        tt.token_amount > 0.0
            && (tt.from_user_account.as_deref() == Some(wallet)
                || tt.to_user_account.as_deref() == Some(wallet))
    });
    if !has_native && !has_token {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tx(tx_type: &str, source: &str) -> EnhancedTransaction {
        EnhancedTransaction {
            signature: "test".into(),
            timestamp: Some(1735689600),
            description: String::new(),
            tx_type: Some(tx_type.into()),
            source: Some(source.into()),
            fee: Some(5000),
            fee_payer: Some("wallet".into()),
            native_transfers: vec![],
            token_transfers: vec![],
        }
    }

    fn with_native(mut tx: EnhancedTransaction, from: &str, to: &str, amount: u64) -> EnhancedTransaction {
        tx.native_transfers.push(soltax_common::NativeTransfer {
            from_user_account: Some(from.into()),
            to_user_account: Some(to.into()),
            amount,
        });
        tx
    }

    #[test]
    fn keeps_normal_tx() {
        let tx = with_native(make_tx("SWAP", "JUPITER"), "wallet", "other", 1000);
        let filtered = apply(vec![tx], "wallet");
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn drops_bubblegum() {
        let tx = with_native(make_tx("COMPRESSED_NFT_MINT", "BUBBLEGUM"), "wallet", "other", 1000);
        let filtered = apply(vec![tx], "wallet");
        assert_eq!(filtered.len(), 0);
    }

    #[test]
    fn drops_zero_value() {
        let tx = make_tx("UNKNOWN", "SYSTEM_PROGRAM"); // no transfers
        let filtered = apply(vec![tx], "wallet");
        assert_eq!(filtered.len(), 0);
    }
}
