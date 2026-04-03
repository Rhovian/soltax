use leptos::prelude::*;
use soltax_common::{EnhancedTransaction, GainLoss};
use std::collections::HashMap;

fn main() {
    leptos::mount::mount_to_body(App);
}

fn derive_wallet(txs: &[EnhancedTransaction]) -> String {
    let mut counts = std::collections::HashMap::<String, usize>::new();
    for tx in txs {
        if let Some(fp) = &tx.fee_payer {
            *counts.entry(fp.clone()).or_default() += 1;
        }
    }
    counts.into_iter().max_by_key(|(_, c)| *c).map(|(k, _)| k).unwrap_or_default()
}

async fn load_transactions() -> Result<Vec<EnhancedTransaction>, String> {
    let resp = gloo_net::http::Request::get("/data/transactions_2025.json")
        .send()
        .await
        .map_err(|e| format!("fetch failed: {e}"))?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json::<Vec<EnhancedTransaction>>()
        .await
        .map_err(|e| format!("parse failed: {e}"))
}

async fn load_gain_loss() -> Result<Vec<GainLoss>, String> {
    let resp = gloo_net::http::Request::get("/data/gain_loss.json")
        .send()
        .await
        .map_err(|e| format!("fetch failed: {e}"))?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json::<Vec<GainLoss>>()
        .await
        .map_err(|e| format!("parse failed: {e}"))
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Transactions,
    TaxReport,
}

#[component]
fn App() -> impl IntoView {
    let txs_resource = LocalResource::new(|| load_transactions());
    let gl_resource = LocalResource::new(|| load_gain_loss());
    let (tab, set_tab) = signal(Tab::TaxReport);

    let tab_style = |t: Tab| {
        move || {
            let active = tab.get() == t;
            if active {
                "padding: 0.5rem 1rem; cursor: pointer; border-bottom: 2px solid #333; font-weight: bold; background: none; border-top: none; border-left: none; border-right: none; font-size: 1rem;"
            } else {
                "padding: 0.5rem 1rem; cursor: pointer; border-bottom: 2px solid transparent; background: none; border-top: none; border-left: none; border-right: none; font-size: 1rem; color: #666;"
            }
        }
    };

    view! {
        <div style="font-family: system-ui, sans-serif; max-width: 1400px; margin: 0 auto; padding: 2rem;">
            <h1>"soltax"</h1>

            <div style="margin-bottom: 1rem; border-bottom: 1px solid #ddd;">
                <button style=tab_style(Tab::TaxReport) on:click=move |_| set_tab.set(Tab::TaxReport)>"Tax Report"</button>
                <button style=tab_style(Tab::Transactions) on:click=move |_| set_tab.set(Tab::Transactions)>"Transactions"</button>
            </div>

            <Show when=move || tab.get() == Tab::TaxReport>
                <Suspense fallback=move || view! { <p>"Loading gain/loss data..."</p> }>
                    {move || gl_resource.get().map(|result| match result.as_ref() {
                        Ok(records) => {
                            let (gl_data, _) = signal(records.clone());
                            view! { <TaxReportView records=gl_data /> }.into_any()
                        }
                        Err(e) => {
                            let msg = e.clone();
                            view! { <p style="color: red;">{format!("No gain/loss data: {msg} — run soltax-report first")}</p> }.into_any()
                        }
                    })}
                </Suspense>
            </Show>

            <Show when=move || tab.get() == Tab::Transactions>
                <Suspense fallback=move || view! { <p>"Loading transactions..."</p> }>
                    {move || txs_resource.get().map(|result| match result.as_ref() {
                        Ok(txs) => {
                            let wallet = derive_wallet(txs);
                            let filtered = soltax_tax::filter::apply(txs.clone(), &wallet);
                            let total = txs.len();
                            let shown = filtered.len();
                            let (transactions, _) = signal(filtered);
                            view! {
                                <p>{format!("{shown} transactions ({} filtered out of {total})", total - shown)}</p>
                                <TransactionTable transactions=transactions />
                            }.into_any()
                        }
                        Err(e) => {
                            let msg = e.clone();
                            view! { <p style="color: red;">{msg}</p> }.into_any()
                        }
                    })}
                </Suspense>
            </Show>
        </div>
    }
}

// --- Tax Report ---

#[derive(Clone)]
struct MintSummary {
    mint: String,
    gain: f64,
    loss: f64,
    net: f64,
    proceeds: f64,
    basis: f64,
    zero_basis_count: usize,
    zero_basis_proceeds: f64,
    count: usize,
}

#[component]
fn TaxReportView(records: ReadSignal<Vec<GainLoss>>) -> impl IntoView {
    let (selected_mint, set_selected_mint) = signal(Option::<String>::None);

    let summaries = move || {
        let recs = records.get();
        let mut map: HashMap<String, MintSummary> = HashMap::new();
        let mut total_gain = 0.0f64;
        let mut total_loss = 0.0f64;

        for r in &recs {
            let s = map.entry(r.mint.clone()).or_insert(MintSummary {
                mint: r.mint.clone(),
                gain: 0.0, loss: 0.0, net: 0.0,
                proceeds: 0.0, basis: 0.0,
                zero_basis_count: 0, zero_basis_proceeds: 0.0,
                count: 0,
            });
            s.proceeds += r.proceeds_usd;
            s.basis += r.cost_basis_usd;
            s.count += 1;
            if r.gain_loss_usd >= 0.0 {
                s.gain += r.gain_loss_usd;
                total_gain += r.gain_loss_usd;
            } else {
                s.loss += r.gain_loss_usd;
                total_loss += r.gain_loss_usd;
            }
            if r.cost_basis_usd == 0.0 && r.proceeds_usd > 0.01 {
                s.zero_basis_count += 1;
                s.zero_basis_proceeds += r.proceeds_usd;
            }
        }
        for s in map.values_mut() {
            s.net = s.gain + s.loss;
        }
        let mut sorted: Vec<MintSummary> = map.into_values().collect();
        sorted.sort_by(|a, b| b.net.abs().partial_cmp(&a.net.abs()).unwrap());
        (sorted, total_gain, total_loss)
    };

    let mint_records = move || {
        let sel = selected_mint.get();
        let recs = records.get();
        match sel {
            Some(mint) => recs.into_iter().filter(|r| r.mint == mint).collect::<Vec<_>>(),
            None => vec![],
        }
    };

    view! {
        {move || {
            let (sums, total_gain, total_loss) = summaries();
            let net = total_gain + total_loss;
            let net_color = if net >= 0.0 { "#16a34a" } else { "#dc2626" };
            view! {
                <div style="display: flex; gap: 2rem; margin-bottom: 1.5rem;">
                    <div style="padding: 1rem; border: 1px solid #ddd; border-radius: 4px; min-width: 150px;">
                        <div style="font-size: 0.75rem; color: #666;">"Short-term Gains"</div>
                        <div style="font-size: 1.25rem; color: #16a34a;">{format!("${total_gain:.2}")}</div>
                    </div>
                    <div style="padding: 1rem; border: 1px solid #ddd; border-radius: 4px; min-width: 150px;">
                        <div style="font-size: 0.75rem; color: #666;">"Short-term Losses"</div>
                        <div style="font-size: 1.25rem; color: #dc2626;">{format!("${total_loss:.2}")}</div>
                    </div>
                    <div style="padding: 1rem; border: 1px solid #ddd; border-radius: 4px; min-width: 150px;">
                        <div style="font-size: 0.75rem; color: #666;">"Net"</div>
                        <div style=format!("font-size: 1.25rem; font-weight: bold; color: {net_color};")>{format!("${net:.2}")}</div>
                    </div>
                </div>

                <h3>"By Token"</h3>
                <div style="max-height: 40vh; overflow-y: auto; border: 1px solid #ddd; border-radius: 4px; margin-bottom: 1.5rem;">
                <table style="width: 100%; border-collapse: collapse; font-size: 0.8rem;">
                    <thead>
                        <tr style="border-bottom: 2px solid #333; text-align: left; position: sticky; top: 0; background: white;">
                            <th style="padding: 0.5rem;">"Token"</th>
                            <th style="padding: 0.5rem; text-align: right;">"Proceeds"</th>
                            <th style="padding: 0.5rem; text-align: right;">"Basis"</th>
                            <th style="padding: 0.5rem; text-align: right;">"Gain"</th>
                            <th style="padding: 0.5rem; text-align: right;">"Loss"</th>
                            <th style="padding: 0.5rem; text-align: right;">"Net"</th>
                            <th style="padding: 0.5rem; text-align: right;">"No Basis"</th>
                            <th style="padding: 0.5rem; text-align: center;">"#"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {sums.into_iter().map(|s| {
                            let mint_clone = s.mint.clone();
                            let mint_display = short_addr(&s.mint);
                            let net_color = if s.net >= 0.0 { "#16a34a" } else { "#dc2626" };
                            let has_zero = s.zero_basis_count > 0;
                            let row_bg = if has_zero { "background: #fef3c7;" } else { "" };
                            view! {
                                <tr
                                    style=format!("border-bottom: 1px solid #eee; cursor: pointer; {row_bg}")
                                    on:click=move |_| set_selected_mint.set(Some(mint_clone.clone()))
                                >
                                    <td style="padding: 0.4rem 0.5rem; font-family: monospace;">{mint_display}</td>
                                    <td style="padding: 0.4rem 0.5rem; text-align: right; font-family: monospace;">{format!("${:.2}", s.proceeds)}</td>
                                    <td style="padding: 0.4rem 0.5rem; text-align: right; font-family: monospace;">{format!("${:.2}", s.basis)}</td>
                                    <td style="padding: 0.4rem 0.5rem; text-align: right; font-family: monospace; color: #16a34a;">{format!("${:.2}", s.gain)}</td>
                                    <td style="padding: 0.4rem 0.5rem; text-align: right; font-family: monospace; color: #dc2626;">{format!("${:.2}", s.loss)}</td>
                                    <td style=format!("padding: 0.4rem 0.5rem; text-align: right; font-family: monospace; font-weight: bold; color: {net_color};")>{format!("${:.2}", s.net)}</td>
                                    <td style="padding: 0.4rem 0.5rem; text-align: right; font-family: monospace;">
                                        {if has_zero {
                                            format!("{} (${:.0})", s.zero_basis_count, s.zero_basis_proceeds)
                                        } else {
                                            "—".to_string()
                                        }}
                                    </td>
                                    <td style="padding: 0.4rem 0.5rem; text-align: center;">{s.count}</td>
                                </tr>
                            }
                        }).collect::<Vec<_>>()}
                    </tbody>
                </table>
                </div>
            }
        }}

        <Show when=move || selected_mint.get().is_some()>
            {move || {
                let mint = selected_mint.get().unwrap_or_default();
                let mint_short = short_addr(&mint);
                let recs = mint_records();
                view! {
                    <div style="display: flex; justify-content: space-between; align-items: center;">
                        <h3>{format!("Disposals: {mint_short}")}</h3>
                        <button
                            style="padding: 0.25rem 0.75rem; cursor: pointer;"
                            on:click=move |_| set_selected_mint.set(None)
                        >"Close"</button>
                    </div>
                    <p style="font-family: monospace; font-size: 0.7rem; color: #666; margin-top: -0.5rem;">{mint.clone()}</p>
                    <div style="max-height: 40vh; overflow-y: auto; border: 1px solid #ddd; border-radius: 4px;">
                    <table style="width: 100%; border-collapse: collapse; font-size: 0.8rem;">
                        <thead>
                            <tr style="border-bottom: 2px solid #333; text-align: left; position: sticky; top: 0; background: white;">
                                <th style="padding: 0.5rem;">"Date"</th>
                                <th style="padding: 0.5rem; text-align: right;">"Amount"</th>
                                <th style="padding: 0.5rem; text-align: right;">"Proceeds"</th>
                                <th style="padding: 0.5rem; text-align: right;">"Basis"</th>
                                <th style="padding: 0.5rem; text-align: right;">"Gain/Loss"</th>
                                <th style="padding: 0.5rem;">"Holding"</th>
                                <th style="padding: 0.5rem;">"Sig"</th>
                            </tr>
                        </thead>
                        <tbody>
                            {recs.into_iter().map(|r| {
                                let date = format_date(r.timestamp);
                                let gl_color = if r.gain_loss_usd >= 0.0 { "#16a34a" } else { "#dc2626" };
                                let is_zero_basis = r.cost_basis_usd == 0.0 && r.proceeds_usd > 0.01;
                                let row_bg = if is_zero_basis { "background: #fef3c7;" } else { "" };
                                let holding = match r.holding_period {
                                    soltax_common::HoldingPeriod::ShortTerm => "ST",
                                    soltax_common::HoldingPeriod::LongTerm => "LT",
                                };
                                let sig_short = if r.signature.len() > 12 {
                                    format!("{}…", &r.signature[..8])
                                } else {
                                    r.signature.clone()
                                };
                                let solscan_url = format!("https://solscan.io/tx/{}", r.signature);
                                view! {
                                    <tr style=format!("border-bottom: 1px solid #eee; {row_bg}")>
                                        <td style="padding: 0.4rem 0.5rem; white-space: nowrap;">{date}</td>
                                        <td style="padding: 0.4rem 0.5rem; text-align: right; font-family: monospace;">{format!("{:.4}", r.amount)}</td>
                                        <td style="padding: 0.4rem 0.5rem; text-align: right; font-family: monospace;">{format!("${:.2}", r.proceeds_usd)}</td>
                                        <td style=format!("padding: 0.4rem 0.5rem; text-align: right; font-family: monospace; {}", if is_zero_basis { "color: #b45309; font-weight: bold;" } else { "" })>
                                            {format!("${:.2}", r.cost_basis_usd)}
                                        </td>
                                        <td style=format!("padding: 0.4rem 0.5rem; text-align: right; font-family: monospace; color: {gl_color};")>
                                            {format!("${:.2}", r.gain_loss_usd)}
                                        </td>
                                        <td style="padding: 0.4rem 0.5rem;">{holding}</td>
                                        <td style="padding: 0.4rem 0.5rem; font-family: monospace;">
                                            <a href={solscan_url} target="_blank" style="color: #0066cc; text-decoration: none;">{sig_short}</a>
                                        </td>
                                    </tr>
                                }
                            }).collect::<Vec<_>>()}
                        </tbody>
                    </table>
                    </div>
                }
            }}
        </Show>
    }
}

// --- Transaction Table (unchanged) ---

fn format_date(ts: i64) -> String {
    let d = js_sys::Date::new_0();
    d.set_time((ts as f64) * 1000.0);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        d.get_full_year(),
        d.get_month() + 1,
        d.get_date(),
        d.get_hours(),
        d.get_minutes(),
    )
}

fn format_sol(lamports: u64) -> String {
    format!("{:.6}", lamports as f64 / 1_000_000_000.0)
}

fn short_addr(addr: &str) -> String {
    if addr.len() > 8 {
        format!("{}..{}", &addr[..4], &addr[addr.len() - 4..])
    } else {
        addr.to_string()
    }
}

fn summarize_native(tx: &EnhancedTransaction, wallet: &str) -> String {
    let mut in_lam: u64 = 0;
    let mut out_lam: u64 = 0;
    for nt in &tx.native_transfers {
        if nt.to_user_account.as_deref() == Some(wallet) {
            in_lam += nt.amount;
        }
        if nt.from_user_account.as_deref() == Some(wallet) {
            out_lam += nt.amount;
        }
    }
    let mut parts = Vec::new();
    if in_lam > 0 {
        parts.push(format!("+{} SOL", format_sol(in_lam)));
    }
    if out_lam > 0 {
        parts.push(format!("-{} SOL", format_sol(out_lam)));
    }
    parts.join(", ")
}

fn summarize_tokens(tx: &EnhancedTransaction, wallet: &str) -> Vec<(String, String, String)> {
    tx.token_transfers.iter().filter_map(|tt| {
        let is_in = tt.to_user_account.as_deref() == Some(wallet);
        let is_out = tt.from_user_account.as_deref() == Some(wallet);
        if !is_in && !is_out {
            return None;
        }
        let dir = if is_in { "+" } else { "-" };
        let mint = tt.mint.as_deref().unwrap_or("???");
        Some((
            dir.to_string(),
            format!("{:.4}", tt.token_amount),
            short_addr(mint),
        ))
    }).collect()
}

#[derive(Clone, Copy, PartialEq)]
enum SortCol {
    Date,
    Type,
    Source,
    Fee,
}

#[derive(Clone, Copy, PartialEq)]
enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    fn toggle(self) -> Self {
        match self {
            SortDir::Asc => SortDir::Desc,
            SortDir::Desc => SortDir::Asc,
        }
    }

    fn arrow(self) -> &'static str {
        match self {
            SortDir::Asc => " ↑",
            SortDir::Desc => " ↓",
        }
    }
}

#[component]
fn SortHeader(
    label: &'static str,
    col: SortCol,
    active_col: ReadSignal<SortCol>,
    active_dir: ReadSignal<SortDir>,
    on_click: Callback<SortCol>,
) -> impl IntoView {
    let style = "padding: 0.5rem; cursor: pointer; user-select: none;";
    view! {
        <th style=style on:click=move |_| on_click.run(col)>
            {label}
            {move || if active_col.get() == col { active_dir.get().arrow() } else { "" }}
        </th>
    }
}

#[component]
fn TransactionTable(transactions: ReadSignal<Vec<EnhancedTransaction>>) -> impl IntoView {
    let wallet = derive_wallet(&transactions.get_untracked());

    let (sort_col, set_sort_col) = signal(SortCol::Date);
    let (sort_dir, set_sort_dir) = signal(SortDir::Desc);

    let on_sort = Callback::new(move |col: SortCol| {
        if sort_col.get_untracked() == col {
            set_sort_dir.set(sort_dir.get_untracked().toggle());
        } else {
            set_sort_col.set(col);
            set_sort_dir.set(SortDir::Desc);
        }
    });

    let sorted = move || {
        let w = wallet.clone();
        let col = sort_col.get();
        let dir = sort_dir.get();
        let mut txs = transactions.get();
        txs.sort_by(|a, b| {
            let ord = match col {
                SortCol::Date => a.timestamp.cmp(&b.timestamp),
                SortCol::Type => a.tx_type.cmp(&b.tx_type),
                SortCol::Source => a.source.cmp(&b.source),
                SortCol::Fee => a.fee.cmp(&b.fee),
            };
            match dir {
                SortDir::Asc => ord,
                SortDir::Desc => ord.reverse(),
            }
        });
        (txs, w)
    };

    view! {
        <div style="max-height: 80vh; overflow-y: auto; border: 1px solid #ddd; border-radius: 4px;">
        <table style="width: 100%; border-collapse: collapse; font-size: 0.8rem;">
            <thead>
                <tr style="border-bottom: 2px solid #333; text-align: left; position: sticky; top: 0; background: white;">
                    <SortHeader label="Date" col=SortCol::Date active_col=sort_col active_dir=sort_dir on_click=on_sort />
                    <SortHeader label="Type" col=SortCol::Type active_col=sort_col active_dir=sort_dir on_click=on_sort />
                    <SortHeader label="Source" col=SortCol::Source active_col=sort_col active_dir=sort_dir on_click=on_sort />
                    <th style="padding: 0.5rem;">"SOL Flow"</th>
                    <th style="padding: 0.5rem;">"Token Transfers"</th>
                    <SortHeader label="Fee" col=SortCol::Fee active_col=sort_col active_dir=sort_dir on_click=on_sort />
                    <th style="padding: 0.5rem;">"Sig"</th>
                </tr>
            </thead>
            <tbody>
                {move || {
                    let (txs, w) = sorted();
                    txs.into_iter().map(move |tx| {
                    let date = tx.timestamp
                        .map(format_date)
                        .unwrap_or_else(|| "—".to_string());

                    let sol_flow = summarize_native(&tx, &w);

                    let token_rows = summarize_tokens(&tx, &w);
                    let token_display = if token_rows.is_empty() {
                        "—".to_string()
                    } else {
                        token_rows.iter()
                            .map(|(dir, amt, mint)| format!("{dir}{amt} {mint}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };

                    let fee_sol = tx.fee.map(|f| format_sol(f))
                        .unwrap_or_else(|| "—".to_string());

                    let sig_short = if tx.signature.len() > 12 {
                        format!("{}…", &tx.signature[..8])
                    } else {
                        tx.signature.clone()
                    };

                    let solscan_url = format!("https://solscan.io/tx/{}", tx.signature);

                    view! {
                        <tr style="border-bottom: 1px solid #eee;">
                            <td style="padding: 0.4rem 0.5rem; white-space: nowrap;">{date}</td>
                            <td style="padding: 0.4rem 0.5rem;">{tx.tx_type.unwrap_or_default()}</td>
                            <td style="padding: 0.4rem 0.5rem;">{tx.source.unwrap_or_default()}</td>
                            <td style="padding: 0.4rem 0.5rem; font-family: monospace; white-space: nowrap;">{sol_flow}</td>
                            <td style="padding: 0.4rem 0.5rem; font-family: monospace; max-width: 350px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{token_display}</td>
                            <td style="padding: 0.4rem 0.5rem; font-family: monospace; white-space: nowrap;">{fee_sol}</td>
                            <td style="padding: 0.4rem 0.5rem; font-family: monospace;">
                                <a href={solscan_url} target="_blank" style="color: #0066cc; text-decoration: none;">{sig_short}</a>
                            </td>
                        </tr>
                    }
                }).collect::<Vec<_>>()}}
            </tbody>
        </table>
        </div>
    }
}
