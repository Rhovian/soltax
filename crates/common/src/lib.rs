use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// SOL mint placeholder used as the "token" for native SOL movements.
pub const SOL_MINT: &str = "So11111111111111111111111111111111111111112";

pub const STABLECOINS: &[&str] = &[
    "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", // USDC
    "USDSwr9ApdHk5bvJKMjzff41FfuX8bSxdKcR81vTwcA",  // USDS
    "Es9vMFrzaCERmKFr8Y2id9xeasWt2d6WGMkGHVDHawmL", // USDT (Wormhole)
    "5YMkXAYccHSGnHn9nob9xEvv6Pvka9DZWH7nTbotTu9E", // USD-pegged
    "HnnGv3HrSqjRpgdFmx7vQGjntNEoex1SU4e9Lxcxuihz", // USD-pegged
    "6FrrzDk5mQARGc1TDYoyVnSyRdds1t4PbtohCD6p3tgG", // USD-pegged
];

/// Tokens that track SOL price (LST wrappers, leveraged SOL, etc).
pub const SOL_PEGGED: &[&str] = &[
    "4sWNB8zGWHkh6UnmwiEtzNxL4XrN7uK9tosbESbJFfVs", // SOL-pegged
    "hy1oXYgrBW6PVcJ4s6s2FKavRdwgWTXdfE69AxT7kPT",  // SOL-pegged
];

pub fn is_sol_pegged(mint: &str) -> bool {
    SOL_PEGGED.contains(&mint)
}

/// Tokens that track ETH price.
pub const ETH_PEGGED: &[&str] = &[
    "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs", // ETH-pegged
];

pub fn is_eth_pegged(mint: &str) -> bool {
    ETH_PEGGED.contains(&mint)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhancedTransaction {
    pub signature: String,
    pub timestamp: Option<i64>,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "type")]
    pub tx_type: Option<String>,
    pub source: Option<String>,
    pub fee: Option<u64>,
    pub fee_payer: Option<String>,
    #[serde(default)]
    pub native_transfers: Vec<NativeTransfer>,
    #[serde(default)]
    pub token_transfers: Vec<TokenTransfer>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeTransfer {
    pub from_user_account: Option<String>,
    pub to_user_account: Option<String>,
    pub amount: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenTransfer {
    pub from_user_account: Option<String>,
    pub to_user_account: Option<String>,
    pub from_token_account: Option<String>,
    pub to_token_account: Option<String>,
    pub token_amount: f64,
    pub mint: Option<String>,
}

// --- Tax types ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaxEventKind {
    Acquisition,
    Disposal,
    Fee,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxEvent {
    pub timestamp: i64,
    pub signature: String,
    pub kind: TaxEventKind,
    pub mint: String,
    pub amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lot {
    pub mint: String,
    pub amount: f64,
    pub cost_basis_usd: f64,
    pub acquired_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HoldingPeriod {
    ShortTerm,
    LongTerm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GainLoss {
    pub signature: String,
    pub timestamp: i64,
    pub mint: String,
    pub amount: f64,
    pub proceeds_usd: f64,
    pub cost_basis_usd: f64,
    pub gain_loss_usd: f64,
    pub holding_period: HoldingPeriod,
}

/// Maps "mint:YYYY-MM-DD" -> USD price per unit.
pub type PriceMap = HashMap<String, f64>;

/// Convert a unix timestamp to a "YYYY-MM-DD" date string (UTC).
pub fn ts_to_date(ts: i64) -> String {
    let secs_per_day: i64 = 86400;
    let days = ts / secs_per_day;
    // days since 1970-01-01
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn civil_from_days(mut days: i64) -> (i32, u32, u32) {
    days += 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

pub fn price_key(mint: &str, timestamp: i64) -> String {
    format!("{}:{}", mint, ts_to_date(timestamp))
}

pub fn is_stablecoin(mint: &str) -> bool {
    STABLECOINS.contains(&mint)
}
