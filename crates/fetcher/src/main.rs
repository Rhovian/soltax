use soltax_common::EnhancedTransaction;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tokio::time::sleep;

const HELIUS_BASE: &str = "https://api-mainnet.helius-rpc.com/v0/addresses";
const PAGE_LIMIT: u32 = 100;
const MAX_RETRIES: u32 = 5;
const PAGE_DELAY: Duration = Duration::from_millis(150);

fn year_bounds(year: i32) -> (i64, i64) {
    let start = year_to_ts(year, 1, 1);
    let end = year_to_ts(year + 1, 1, 1) - 1;
    (start, end)
}

fn year_to_ts(y: i32, m: u32, d: u32) -> i64 {
    let y2 = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let m2 = if m <= 2 { m as i64 + 9 } else { m as i64 - 3 };
    let days = 365 * y2 + y2 / 4 - y2 / 100 + y2 / 400 + (m2 * 153 + 2) / 5 + d as i64 - 719469;
    days * 86400
}

fn output_file(year: i32) -> String {
    format!("data/transactions_{year}.json")
}

fn load_existing(path: &str) -> Vec<EnhancedTransaction> {
    if !Path::new(path).exists() {
        return Vec::new();
    }
    let data = fs::read_to_string(path).expect("failed to read existing transactions file");
    serde_json::from_str(&data).expect("failed to parse existing transactions file")
}

fn save(txs: &[EnhancedTransaction], path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new(path).parent().unwrap();
    fs::create_dir_all(dir)?;
    let file = fs::File::create(path)?;
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

    let year: i32 = env::args()
        .nth(1)
        .map(|s| s.parse().expect("usage: soltax-fetcher <year>"))
        .unwrap_or(2025);

    let (start_ts, end_ts) = year_bounds(year);
    let out_path = output_file(year);

    eprintln!("fetching {year} transactions (ts {start_ts}..{end_ts})");
    eprintln!("output: {out_path}");

    let mut existing = load_existing(&out_path);
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

        let mut hit_before_year = false;
        let count_before = new_txs.len();

        for tx in txs {
            before_sig = Some(tx.signature.clone());

            if let Some(ts) = tx.timestamp {
                if ts < start_ts {
                    hit_before_year = true;
                    break;
                }
                if ts <= end_ts && !known_sigs.contains(&tx.signature) {
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

        if hit_before_year {
            break;
        }

        sleep(PAGE_DELAY).await;
    }

    eprintln!("fetched {} new transactions", new_txs.len());

    existing.extend(new_txs);
    existing.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    save(&existing, &out_path)?;
    eprintln!("saved {} total transactions to {out_path}", existing.len());

    Ok(())
}
