use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;
use tokio::time::sleep;

const PRICES_FILE: &str = "data/prices.json";
const LLAMA_BASE: &str = "https://coins.llama.fi";

/// Convert "YYYY-MM-DD" to a unix timestamp (noon UTC to avoid edge issues).
fn date_to_timestamp(date: &str) -> i64 {
    let parts: Vec<i64> = date.split('-').map(|p| p.parse().unwrap()).collect();
    let (y, m, d) = (parts[0], parts[1], parts[2]);

    // Days from civil date (simplified)
    let y2 = if m <= 2 { y - 1 } else { y };
    let m2 = if m <= 2 { m + 9 } else { m - 3 };
    let days = 365 * y2 + y2 / 4 - y2 / 100 + y2 / 400 + (m2 * 153 + 2) / 5 + d - 719469;

    // Noon UTC
    days * 86400 + 43200
}

async fn fetch_price(
    client: &reqwest::Client,
    mint: &str,
    timestamp: i64,
) -> Result<Option<f64>, Box<dyn std::error::Error>> {
    let coin_id = format!("solana:{mint}");
    let url = format!("{LLAMA_BASE}/prices/historical/{timestamp}/{coin_id}");

    let resp = client.get(&url).send().await?;
    let status = resp.status();

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err("rate limited".into());
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        eprintln!("  warning: {mint} -> {status}: {body}");
        return Ok(None);
    }

    let data: serde_json::Value = resp.json().await?;
    let price = data["coins"][&coin_id]["price"].as_f64();
    Ok(price)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    // Load existing prices
    let mut prices: BTreeMap<String, f64> = if fs::metadata(PRICES_FILE).is_ok() {
        let data = fs::read_to_string(PRICES_FILE)?;
        serde_json::from_str(&data)?
    } else {
        eprintln!("no {PRICES_FILE} found — run soltax-report first to generate it");
        return Ok(());
    };

    // Find entries that are still 0.0
    let missing: Vec<(String, String)> = prices
        .iter()
        .filter(|(_, v)| **v == 0.0)
        .map(|(k, _)| {
            let (mint, date) = k.split_once(':').unwrap();
            (mint.to_string(), date.to_string())
        })
        .collect();

    if missing.is_empty() {
        eprintln!("all prices already filled in!");
        return Ok(());
    }

    eprintln!("{} prices to fetch from DeFiLlama", missing.len());

    let client = reqwest::Client::new();
    let mut filled = 0u32;
    let mut not_found: Vec<(String, String)> = Vec::new();

    for (mint, date) in &missing {
        let ts = date_to_timestamp(date);
        let key = format!("{mint}:{date}");

        // Retry loop for rate limits
        let mut attempts = 0u32;
        let price = loop {
            match fetch_price(&client, mint, ts).await {
                Ok(p) => break p,
                Err(e) if e.to_string().contains("rate limited") && attempts < 5 => {
                    attempts += 1;
                    let backoff = Duration::from_secs(2u64.pow(attempts));
                    eprintln!("  rate limited, waiting {}s...", backoff.as_secs());
                    sleep(backoff).await;
                }
                Err(e) => {
                    eprintln!("  error {mint} {date}: {e}");
                    break None;
                }
            }
        };

        if let Some(p) = price {
            prices.insert(key, p);
            filled += 1;
            eprint!("  {mint}:{date} ${p:.6}\r");
        } else {
            not_found.push((mint.clone(), date.clone()));
        }

        // ~500ms between requests to be polite
        sleep(Duration::from_millis(500)).await;
    }

    eprintln!();

    // Save updated prices
    let json = serde_json::to_string_pretty(&prices)?;
    fs::write(PRICES_FILE, &json)?;

    eprintln!("filled {filled} prices");

    if !not_found.is_empty() {
        eprintln!(
            "\n{} prices not found on DeFiLlama (fill manually):",
            not_found.len()
        );
        for (mint, date) in &not_found {
            eprintln!("  {mint}:{date}");
        }
    }

    let remaining: usize = prices.values().filter(|v| **v == 0.0).count();
    if remaining > 0 {
        eprintln!("\n{remaining} prices still at 0.0 — fill those manually");
    } else {
        eprintln!("\nall prices filled! run soltax-report to generate tax report");
    }

    Ok(())
}
