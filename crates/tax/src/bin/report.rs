use soltax_common::{
    EnhancedTransaction, GainLoss, Lot, PriceMap, SOL_MINT, TaxEvent, is_sol_pegged,
    is_stablecoin, price_key, ts_to_date,
};
use soltax_tax::{events, fifo, filter};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

const TX_FILES: &[&str] = &[
    "data/transactions_2024.json",
    "data/transactions_2025.json",
];
const PRICES_FILE: &str = "data/prices.json";
const LOTS_FILE: &str = "data/initial_lots.json";
const TRACKED_FILE: &str = "data/tracked_tokens.json";
const OUTPUT_EVENTS: &str = "data/tax_events.json";
const OUTPUT_REPORT: &str = "data/gain_loss.json";
const OUTPUT_REMAINING: &str = "data/remaining_lots.json";
const MANUAL_GAINS_FILE: &str = "data/manual_gains.json";

/// Well-known tokens that are auto-marked as tracked.
const KNOWN_TOKENS: &[&str] = &[
    SOL_MINT,
    "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", // USDC
    "USDSwr9ApdHk5bvJKMjzff41FfuX8bSxdKcR81vTwcA",  // USDS
    "mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So",  // mSOL
    "J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn", // JitoSOL
    "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN",  // JUP
    "MNDEFzGvMt87ueuHvVU9VcTqsAP5b3fTGPsHuuPA5ey",  // MNDE
    "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263", // BONK
    "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R", // RAY
];

fn derive_wallet(txs: &[EnhancedTransaction]) -> String {
    let mut counts = HashMap::<String, usize>::new();
    for tx in txs {
        if let Some(fp) = &tx.fee_payer {
            *counts.entry(fp.clone()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(k, _)| k)
        .unwrap_or_default()
}

fn auto_fill_known_prices(prices: &mut PriceMap, needed: &HashSet<String>) {
    let mut stable_count = 0;
    let mut sol_pegged_count = 0;

    for key in needed {
        let (mint, date) = key.split_once(':').unwrap();
        let current = prices.get(key).copied().unwrap_or(0.0);
        if is_stablecoin(mint) && current == 0.0 {
            prices.insert(key.clone(), 1.0);
            stable_count += 1;
        }
        if is_sol_pegged(mint) && current == 0.0 {
            let sol_key = format!("{SOL_MINT}:{date}");
            if let Some(&sol_price) = prices.get(&sol_key) {
                prices.insert(key.clone(), sol_price);
                sol_pegged_count += 1;
            }
        }
    }
    if stable_count > 0 {
        eprintln!("auto-filled {stable_count} stablecoin prices at $1.00");
    }
    if sol_pegged_count > 0 {
        eprintln!("auto-filled {sol_pegged_count} SOL-pegged prices from SOL");
    }
}

/// Build or load tracked_tokens.json.
/// Returns the set of mints the user wants to track.
fn resolve_tracked_tokens(all_events: &[TaxEvent]) -> Option<HashSet<String>> {
    // Gather per-mint stats from events
    let mut mint_volume: HashMap<String, f64> = HashMap::new();
    let mut mint_event_count: HashMap<String, usize> = HashMap::new();
    for e in all_events {
        *mint_volume.entry(e.mint.clone()).or_default() += e.amount;
        *mint_event_count.entry(e.mint.clone()).or_default() += 1;
    }

    if Path::new(TRACKED_FILE).exists() {
        let data = fs::read_to_string(TRACKED_FILE).unwrap();
        let map: BTreeMap<String, bool> = serde_json::from_str(&data).unwrap();

        // Check for new mints not yet in the file
        let mut new_mints = Vec::new();
        for mint in mint_volume.keys() {
            if !map.contains_key(mint) {
                new_mints.push(mint.clone());
            }
        }
        if !new_mints.is_empty() {
            eprintln!(
                "{} new token(s) found not in {TRACKED_FILE} — re-run after updating:",
                new_mints.len()
            );
            let mut updated = map;
            for mint in &new_mints {
                let suggested = KNOWN_TOKENS.contains(&mint.as_str());
                eprintln!("  {mint} (volume: {:.4}, suggested: {suggested})", mint_volume[mint]);
                updated.insert(mint.clone(), suggested);
            }
            let json = serde_json::to_string_pretty(&updated).unwrap();
            fs::write(TRACKED_FILE, &json).unwrap();
            return None;
        }

        let tracked: HashSet<String> = map
            .into_iter()
            .filter(|(_, v)| *v)
            .map(|(k, _)| k)
            .collect();
        Some(tracked)
    } else {
        // Generate initial file with suggestions
        let mut map: BTreeMap<String, bool> = BTreeMap::new();
        let mut entries: Vec<(String, f64, usize)> = mint_volume
            .iter()
            .map(|(m, v)| (m.clone(), *v, *mint_event_count.get(m).unwrap_or(&0)))
            .collect();
        entries.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.partial_cmp(&a.1).unwrap()));

        eprintln!("\ngenerated {TRACKED_FILE} — review and set tokens to true/false:");
        for (mint, vol, count) in &entries {
            let is_known = KNOWN_TOKENS.contains(&mint.as_str()) || is_stablecoin(mint);
            // Auto-track: known tokens or tokens with multiple events
            let suggested = is_known || *count > 2;
            map.insert(mint.clone(), suggested);
            let label = if suggested { "TRACK" } else { "skip " };
            eprintln!("  [{label}] {mint}  ({count} events, volume: {vol:.4})");
        }

        let json = serde_json::to_string_pretty(&map).unwrap();
        fs::write(TRACKED_FILE, &json).unwrap();
        eprintln!("\nedit {TRACKED_FILE} then re-run");
        None
    }
}

fn main() {
    dotenvy::dotenv().ok();

    // Load transactions from all years
    let mut txs = Vec::new();
    for tx_file in TX_FILES {
        if !Path::new(tx_file).exists() {
            eprintln!("skipping {tx_file} (not found)");
            continue;
        }
        let tx_data = fs::read_to_string(tx_file).expect(&format!("failed to read {tx_file}"));
        let file_txs: Vec<EnhancedTransaction> =
            serde_json::from_str(&tx_data).expect(&format!("failed to parse {tx_file}"));
        eprintln!("loaded {} transactions from {tx_file}", file_txs.len());
        txs.extend(file_txs);
    }
    let wallet = std::env::var("WALLET_ADDRESS").unwrap_or_else(|_| derive_wallet(&txs));
    eprintln!("wallet: {wallet}");

    // Filter spam
    let filtered = filter::apply(txs, &wallet);
    eprintln!("{} transactions after filtering", filtered.len());

    // Extract all events
    let all_events = events::extract_all(&filtered, &wallet);
    eprintln!("{} tax events extracted", all_events.len());

    // Resolve tracked tokens (may generate file and exit)
    let tracked = match resolve_tracked_tokens(&all_events) {
        Some(t) => t,
        None => return,
    };

    // Filter events to only tracked tokens
    let tracked_events: Vec<&TaxEvent> = all_events.iter().filter(|e| tracked.contains(&e.mint)).collect();
    eprintln!(
        "{} events for {} tracked tokens ({} skipped)",
        tracked_events.len(),
        tracked.len(),
        all_events.len() - tracked_events.len()
    );

    // Write filtered events for inspection
    let events_json = serde_json::to_string_pretty(&tracked_events).unwrap();
    fs::write(OUTPUT_EVENTS, &events_json).unwrap();

    // Collect needed prices (daily)
    let needed: HashSet<String> = tracked_events
        .iter()
        .map(|e| price_key(&e.mint, e.timestamp))
        .collect();

    // Load existing prices and auto-fill stablecoins
    let mut prices: PriceMap = if Path::new(PRICES_FILE).exists() {
        let data = fs::read_to_string(PRICES_FILE).unwrap();
        serde_json::from_str(&data).unwrap()
    } else {
        PriceMap::new()
    };
    auto_fill_known_prices(&mut prices, &needed);

    // Find missing prices
    let mut mint_dates: HashMap<String, HashSet<String>> = HashMap::new();
    let mut mint_volume: HashMap<String, f64> = HashMap::new();
    for event in &tracked_events {
        let key = price_key(&event.mint, event.timestamp);
        if !prices.contains_key(&key) {
            mint_dates
                .entry(event.mint.clone())
                .or_default()
                .insert(ts_to_date(event.timestamp));
            *mint_volume.entry(event.mint.clone()).or_default() += event.amount;
        }
    }

    if !mint_dates.is_empty() {
        let total_missing: usize = mint_dates.values().map(|d| d.len()).sum();
        eprintln!(
            "\n{total_missing} daily prices still needed across {} tokens:",
            mint_dates.len()
        );

        let mut sorted: Vec<_> = mint_dates.iter().collect();
        sorted.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

        for (mint, dates) in &sorted {
            let vol = mint_volume.get(*mint).unwrap_or(&0.0);
            eprintln!("  {mint}  ({} days, volume: {vol:.4})", dates.len());
        }

        // Merge into prices file
        let mut template: BTreeMap<String, f64> =
            prices.iter().map(|(k, v)| (k.clone(), *v)).collect();
        for (mint, dates) in &mint_dates {
            for date in dates {
                let key = format!("{mint}:{date}");
                template.entry(key).or_insert(0.0);
            }
        }
        let json = serde_json::to_string_pretty(&template).unwrap();
        fs::write(PRICES_FILE, &json).unwrap();
        eprintln!("\nupdated {PRICES_FILE} — fill in 0.0 entries and re-run");
        return;
    }

    eprintln!("all prices available");

    // Load initial lots
    let initial_lots: Vec<Lot> = if Path::new(LOTS_FILE).exists() {
        let data = fs::read_to_string(LOTS_FILE).unwrap();
        serde_json::from_str(&data).unwrap()
    } else {
        eprintln!("no {} found, starting with empty lots", LOTS_FILE);
        Vec::new()
    };

    // Run FIFO — need owned events
    let owned_events: Vec<TaxEvent> = tracked_events.into_iter().cloned().collect();
    // Only report gain/loss for 2025 (Jan 1 2025 00:00 UTC)
    let report_from: i64 = 1735689600;
    let mut engine = fifo::FifoEngine::new(initial_lots);
    engine.process(&owned_events, &prices, Some(report_from));

    if !engine.missing_prices.is_empty() {
        eprintln!(
            "warning: {} events skipped due to missing prices",
            engine.missing_prices.len()
        );
    }

    // Get remaining lots before moving results
    let remaining_lots = engine.remaining_lots();

    // Merge manual gains
    let mut all_results = engine.results;
    if Path::new(MANUAL_GAINS_FILE).exists() {
        let data = fs::read_to_string(MANUAL_GAINS_FILE).unwrap();
        let manual: Vec<GainLoss> = serde_json::from_str(&data).unwrap();
        eprintln!("{} manual gain/loss entries loaded", manual.len());
        all_results.extend(manual);
    }

    // Write results
    let report = serde_json::to_string_pretty(&all_results).unwrap();
    fs::write(OUTPUT_REPORT, &report).unwrap();
    eprintln!(
        "wrote {} gain/loss records to {OUTPUT_REPORT}",
        all_results.len()
    );

    let remaining = serde_json::to_string_pretty(&remaining_lots).unwrap();
    fs::write(OUTPUT_REMAINING, &remaining).unwrap();

    // Summary (from all results including manual)
    let summary = {
        let mut s = fifo::TaxSummary {
            short_term_gain: 0.0, short_term_loss: 0.0,
            long_term_gain: 0.0, long_term_loss: 0.0, net: 0.0,
        };
        for gl in &all_results {
            match gl.holding_period {
                soltax_common::HoldingPeriod::ShortTerm => {
                    if gl.gain_loss_usd >= 0.0 { s.short_term_gain += gl.gain_loss_usd; }
                    else { s.short_term_loss += gl.gain_loss_usd; }
                }
                soltax_common::HoldingPeriod::LongTerm => {
                    if gl.gain_loss_usd >= 0.0 { s.long_term_gain += gl.gain_loss_usd; }
                    else { s.long_term_loss += gl.gain_loss_usd; }
                }
            }
        }
        s.net = s.short_term_gain + s.short_term_loss + s.long_term_gain + s.long_term_loss;
        s
    };
    eprintln!("\n=== 2025 Tax Summary ===");
    eprintln!("Short-term gains:  ${:.2}", summary.short_term_gain);
    eprintln!("Short-term losses: ${:.2}", summary.short_term_loss);
    eprintln!("Long-term gains:   ${:.2}", summary.long_term_gain);
    eprintln!("Long-term losses:  ${:.2}", summary.long_term_loss);
    eprintln!("Net:               ${:.2}", summary.net);

    // Export Form 8949 CSV for TurboTax
    let csv_path = "data/form_8949.csv";
    let mut csv = String::new();
    csv.push_str("Description,Date Acquired,Date Sold,Proceeds,Cost Basis,Gain or Loss,Term\n");

    let mut sorted_results = all_results.clone();
    sorted_results.sort_by_key(|r| r.timestamp);

    for gl in &sorted_results {
        if gl.gain_loss_usd.abs() < 0.01 {
            continue;
        }
        let date_sold = ts_to_date(gl.timestamp);
        let date_acquired = if gl.cost_basis_usd == 0.0 && gl.holding_period == soltax_common::HoldingPeriod::LongTerm {
            "Various".to_string()
        } else {
            // Find the lot acquisition date from initial lots or use "Various"
            "Various".to_string()
        };
        let term = match gl.holding_period {
            soltax_common::HoldingPeriod::ShortTerm => "Short",
            soltax_common::HoldingPeriod::LongTerm => "Long",
        };
        let mint_short = if gl.mint.len() > 8 {
            format!("{}..{}", &gl.mint[..4], &gl.mint[gl.mint.len()-4..])
        } else {
            gl.mint.clone()
        };
        let desc = format!("{:.4} {} ({})", gl.amount, mint_short, &gl.signature[..8]);
        csv.push_str(&format!(
            "\"{}\",{},{},{:.2},{:.2},{:.2},{}\n",
            desc, date_acquired, date_sold, gl.proceeds_usd, gl.cost_basis_usd, gl.gain_loss_usd, term
        ));
    }

    fs::write(csv_path, &csv).unwrap();
    eprintln!("wrote Form 8949 CSV to {csv_path}");
}
