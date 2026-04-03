use soltax_common::{GainLoss, HoldingPeriod, Lot, PriceMap, TaxEvent, TaxEventKind, price_key};
use std::collections::{HashMap, VecDeque};

const ONE_YEAR_SECS: i64 = 365 * 24 * 60 * 60;

pub struct FifoEngine {
    /// Per-mint FIFO lot queues
    lots: HashMap<String, VecDeque<Lot>>,
    /// Completed gain/loss records
    pub results: Vec<GainLoss>,
    /// Events where price was missing
    pub missing_prices: Vec<(String, i64)>,
}

impl FifoEngine {
    pub fn new(initial_lots: Vec<Lot>) -> Self {
        let mut lots: HashMap<String, VecDeque<Lot>> = HashMap::new();
        for lot in initial_lots {
            lots.entry(lot.mint.clone()).or_default().push_back(lot);
        }
        Self {
            lots,
            results: Vec::new(),
            missing_prices: Vec::new(),
        }
    }

    pub fn process(&mut self, events: &[TaxEvent], prices: &PriceMap) {
        for event in events {
            match event.kind {
                TaxEventKind::Acquisition => self.acquire(event, prices),
                TaxEventKind::Disposal | TaxEventKind::Fee => self.dispose(event, prices),
            }
        }
    }

    fn acquire(&mut self, event: &TaxEvent, prices: &PriceMap) {
        let key = price_key(&event.mint, event.timestamp);
        let price_usd = match prices.get(&key) {
            Some(&p) => p,
            None => {
                self.missing_prices.push((event.mint.clone(), event.timestamp));
                return;
            }
        };

        let cost_basis = event.amount * price_usd;
        self.lots
            .entry(event.mint.clone())
            .or_default()
            .push_back(Lot {
                mint: event.mint.clone(),
                amount: event.amount,
                cost_basis_usd: cost_basis,
                acquired_at: event.timestamp,
            });
    }

    fn dispose(&mut self, event: &TaxEvent, prices: &PriceMap) {
        let key = price_key(&event.mint, event.timestamp);
        let price_usd = match prices.get(&key) {
            Some(&p) => p,
            None => {
                self.missing_prices.push((event.mint.clone(), event.timestamp));
                return;
            }
        };

        let proceeds = event.amount * price_usd;
        let mut remaining = event.amount;
        let queue = self.lots.entry(event.mint.clone()).or_default();

        let mut total_cost_basis = 0.0;
        let mut earliest_acquired = event.timestamp;

        while remaining > 1e-12 {
            let lot = match queue.front_mut() {
                Some(lot) => lot,
                None => {
                    // No lots left — unknown cost basis (could be prior unsourced tokens)
                    // Record with zero cost basis so user can see it
                    break;
                }
            };

            let consume = remaining.min(lot.amount);
            let fraction = consume / lot.amount;
            let cost = lot.cost_basis_usd * fraction;

            total_cost_basis += cost;
            if lot.acquired_at < earliest_acquired {
                earliest_acquired = lot.acquired_at;
            }

            lot.amount -= consume;
            lot.cost_basis_usd -= cost;
            remaining -= consume;

            if lot.amount < 1e-12 {
                queue.pop_front();
            }
        }

        let amount_disposed = event.amount - remaining;
        if amount_disposed > 1e-12 {
            let holding = if event.timestamp - earliest_acquired >= ONE_YEAR_SECS {
                HoldingPeriod::LongTerm
            } else {
                HoldingPeriod::ShortTerm
            };

            let proceeds_for_disposed = amount_disposed / event.amount * proceeds;

            self.results.push(GainLoss {
                signature: event.signature.clone(),
                timestamp: event.timestamp,
                mint: event.mint.clone(),
                amount: amount_disposed,
                proceeds_usd: proceeds_for_disposed,
                cost_basis_usd: total_cost_basis,
                gain_loss_usd: proceeds_for_disposed - total_cost_basis,
                holding_period: holding,
            });
        }

        // If remaining > 0, we ran out of lots
        if remaining > 1e-12 {
            let proceeds_for_unknown = remaining / event.amount * proceeds;
            self.results.push(GainLoss {
                signature: event.signature.clone(),
                timestamp: event.timestamp,
                mint: event.mint.clone(),
                amount: remaining,
                proceeds_usd: proceeds_for_unknown,
                cost_basis_usd: 0.0,
                gain_loss_usd: proceeds_for_unknown,
                holding_period: HoldingPeriod::ShortTerm,
            });
        }
    }

    /// Returns remaining lots (carry forward to next year).
    pub fn remaining_lots(&self) -> Vec<Lot> {
        self.lots
            .values()
            .flat_map(|q| q.iter().filter(|l| l.amount > 1e-12).cloned())
            .collect()
    }

    pub fn summary(&self) -> TaxSummary {
        let mut short_term_gain = 0.0;
        let mut short_term_loss = 0.0;
        let mut long_term_gain = 0.0;
        let mut long_term_loss = 0.0;

        for gl in &self.results {
            match gl.holding_period {
                HoldingPeriod::ShortTerm => {
                    if gl.gain_loss_usd >= 0.0 {
                        short_term_gain += gl.gain_loss_usd;
                    } else {
                        short_term_loss += gl.gain_loss_usd;
                    }
                }
                HoldingPeriod::LongTerm => {
                    if gl.gain_loss_usd >= 0.0 {
                        long_term_gain += gl.gain_loss_usd;
                    } else {
                        long_term_loss += gl.gain_loss_usd;
                    }
                }
            }
        }

        TaxSummary {
            short_term_gain,
            short_term_loss,
            long_term_gain,
            long_term_loss,
            net: short_term_gain + short_term_loss + long_term_gain + long_term_loss,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TaxSummary {
    pub short_term_gain: f64,
    pub short_term_loss: f64,
    pub long_term_gain: f64,
    pub long_term_loss: f64,
    pub net: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_fifo_gain() {
        // Buy 10 tokens at $1, sell 10 tokens at $2 = $10 gain
        let initial = vec![Lot {
            mint: "TOKEN".into(),
            amount: 10.0,
            cost_basis_usd: 10.0,
            acquired_at: 1700000000,
        }];

        let events = vec![TaxEvent {
            timestamp: 1735700000,
            signature: "sell1".into(),
            kind: TaxEventKind::Disposal,
            mint: "TOKEN".into(),
            amount: 10.0,
        }];

        let mut prices = PriceMap::new();
        prices.insert(price_key("TOKEN", 1735700000), 2.0);

        let mut engine = FifoEngine::new(initial);
        engine.process(&events, &prices);

        assert_eq!(engine.results.len(), 1);
        let gl = &engine.results[0];
        assert!((gl.proceeds_usd - 20.0).abs() < 0.01);
        assert!((gl.cost_basis_usd - 10.0).abs() < 0.01);
        assert!((gl.gain_loss_usd - 10.0).abs() < 0.01);
        assert_eq!(gl.holding_period, HoldingPeriod::LongTerm);
    }

    #[test]
    fn fifo_partial_lots() {
        // Two lots: 5 at $1, 5 at $3. Sell 7 at $2.
        // FIFO: consume 5@$1 + 2@$3 = cost $11, proceeds $14, gain $3
        let initial = vec![
            Lot { mint: "T".into(), amount: 5.0, cost_basis_usd: 5.0, acquired_at: 1700000000 },
            Lot { mint: "T".into(), amount: 5.0, cost_basis_usd: 15.0, acquired_at: 1710000000 },
        ];

        let events = vec![TaxEvent {
            timestamp: 1735700000,
            signature: "s".into(),
            kind: TaxEventKind::Disposal,
            mint: "T".into(),
            amount: 7.0,
        }];

        let mut prices = PriceMap::new();
        prices.insert(price_key("T", 1735700000), 2.0);

        let mut engine = FifoEngine::new(initial);
        engine.process(&events, &prices);

        assert_eq!(engine.results.len(), 1);
        let gl = &engine.results[0];
        assert!((gl.proceeds_usd - 14.0).abs() < 0.01);
        assert!((gl.cost_basis_usd - 11.0).abs() < 0.01);
        assert!((gl.gain_loss_usd - 3.0).abs() < 0.01);

        // 3 tokens remaining from second lot
        let rem = engine.remaining_lots();
        assert_eq!(rem.len(), 1);
        assert!((rem[0].amount - 3.0).abs() < 0.01);
    }
}
