use soltax_common::EnhancedTransaction;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tokio::time::sleep;

const HELIUS_BASE: &str = "https://api-mainnet.helius-rpc.com/v0/addresses";
const PAGE_LIMIT: u32 = 100;
const OUTPUT_FILE: &str = "data/transactions_2025.json";
const MAX_RETRIES: u32 = 5;
const PAGE_DELAY: Duration = Duration::from_millis(150);

// 2025-01-01T00:00:00Z
const START_TS: i64 = 1735689600;
// 2025-12-31T23:59:59Z
const END_TS: i64 = 1767225599;

fn load_existing() -> Vec<EnhancedTransaction> {
    let path = Path::new(OUTPUT_FILE);
    if !path.exists() {
        return Vec::new();
    }
    let data = fs::read_to_string(path).expect("failed to read existing transactions file");
    serde_json::from_str(&data).expect("failed to parse existing transactions file")
}

fn save(txs: &[EnhancedTransaction]) -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new(OUTPUT_FILE).parent().unwrap();
    fs::create_dir_all(dir)?;
    let file = fs::File::create(OUTPUT_FILE)?;
    serde_json::to_writer_pretty(file, txs)?;
    Ok(())
}

async fn fetch_page(
    client: &reqwest::Client,
    address: &str,
    api_key: &str,
    before_sig: Option<&str>,
) -> Result<Vec<EnhancedTransaction>, Box<dyn std::error::Error>> {
    let mut url = format!(
        "{}/{}/transactions?api-key={}&limit={}",
        HELIUS_BASE, address, api_key, PAGE_LIMIT
    );
    if let Some(sig) = before_sig {
        url.push_str(&format!("&before-signature={}", sig));
    }

    for attempt in 0..MAX_RETRIES {
        let resp = client.get(&url).send().await?;
        let status = resp.status();

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let backoff = Duration::from_secs(2u64.pow(attempt));
            eprintln!("rate limited, backing off {}s", backoff.as_secs());
            sleep(backoff).await;
            continue;
        }

        if status.is_server_error() {
            let backoff = Duration::from_secs(2u64.pow(attempt));
            eprintln!("server error {}, retrying in {}s", status, backoff.as_secs());
            sleep(backoff).await;
            continue;
        }

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("API error {}: {}", status, body).into());
        }

        let txs: Vec<EnhancedTransaction> = resp.json().await?;
        return Ok(txs);
    }

    Err("max retries exceeded".into())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let api_key = env::var("HELIUS_API_KEY").expect("HELIUS_API_KEY must be set");
    let address = env::var("WALLET_ADDRESS").expect("WALLET_ADDRESS must be set");

    // Load existing and build a set of known signatures
    let mut existing = load_existing();
    let known_sigs: HashSet<String> = existing.iter().map(|tx| tx.signature.clone()).collect();
    eprintln!("loaded {} existing transactions", existing.len());

    let client = reqwest::Client::new();
    let mut new_txs: Vec<EnhancedTransaction> = Vec::new();
    let mut before_sig: Option<String> = None;
    let mut page = 0u32;

    loop {
        page += 1;
        let txs = fetch_page(&client, &address, &api_key, before_sig.as_deref()).await?;

        if txs.is_empty() {
            break;
        }

        let mut hit_before_2025 = false;
        let count_before = new_txs.len();

        for tx in txs {
            before_sig = Some(tx.signature.clone());

            if let Some(ts) = tx.timestamp {
                if ts < START_TS {
                    hit_before_2025 = true;
                    break;
                }
                if ts <= END_TS && !known_sigs.contains(&tx.signature) {
                    new_txs.push(tx);
                }
            }
        }

        eprintln!(
            "page {}: +{} new (total new {})",
            page,
            new_txs.len() - count_before,
            new_txs.len()
        );

        if hit_before_2025 {
            break;
        }

        sleep(PAGE_DELAY).await;
    }

    eprintln!("fetched {} new transactions", new_txs.len());

    // Merge new into existing
    existing.extend(new_txs);

    // Sort by timestamp descending (newest first)
    existing.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    save(&existing)?;
    eprintln!(
        "saved {} total transactions to {}",
        existing.len(),
        OUTPUT_FILE
    );

    Ok(())
}
