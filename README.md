# soltax

Solana FIFO tax calculator. Fetches transactions via Helius, prices via DeFiLlama, and computes gain/loss per token.

## Setup

```
cp .env.example .env
# fill in HELIUS_API_KEY and WALLET_ADDRESS
```

## Usage

### 1. Fetch transactions

```
cargo run -p soltax-fetcher
```

Paginates all 2025 transactions into `data/transactions_2025.json`. Appends and deduplicates on re-run.

### 2. Generate tax report

```
cargo run --bin soltax-report
```

First run generates `data/tracked_tokens.json` — set tokens to `true`/`false` and re-run. Stablecoins and SOL/ETH-pegged tokens are auto-priced.

### 3. Fetch missing prices

```
cargo run --bin soltax-prices
```

Fills `data/prices.json` from DeFiLlama (free, no key needed). Remaining gaps need manual entry.

### 4. Add prior-year cost basis

Edit `data/initial_lots.json` with `costBasisUsd` and `acquiredAt` (unix timestamp) for tokens held from previous years.

### 5. Exclude specific transactions

Add signatures to `data/excluded_signatures.json`:

```json
["4tkGpv...", "abc123..."]
```

### 6. Re-run report

```
cargo run --bin soltax-report
```

Outputs `data/gain_loss.json`, `data/remaining_lots.json`, and a terminal summary.

### 7. View in browser

```
cd crates/app && trunk serve --open
```

Tax Report tab shows per-token breakdown with drill-down. Yellow rows = missing cost basis from prior years.

## Project structure

```
crates/
  common/    shared types
  fetcher/   Helius transaction fetcher
  tax/       filter, event extraction, FIFO engine, CLI tools
  app/       Leptos CSR frontend
data/        transactions, prices, lots, reports (gitignored)
```
