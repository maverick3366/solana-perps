# CLAUDE.md — solana-perps / platyperps

## Project overview

This repo contains two components:

1. **Python Dash dashboard** (`app.py`, `ui/`) — real-time analytics for Solana perpetuals markets (Drift, Mango, FTX, dYdX). Fetches data via public web APIs and the Drift anchor IDL.
2. **Rust AMM submission** (`prop-amm/`) — a `RegimeAdaptive` constant-product AMM for the prop_amm competition. Uses `pinocchio` + `prop_amm_submission_sdk`. Model: `claude-sonnet-4-6`.

---

## Dashboard (Python)

### Stack
- **Framework**: Dash / Plotly
- **Data**: HTTP API requests (Drift, Mango, FTX, dYdX, CoinGecko)
- **Drift integration**: `drift-py` submodule + anchorpy
- **Mango integration**: `mango-explorer` submodule

### Run locally
```bash
pip install -r requirements.txt
python app.py
```

### Entry points
| File | Purpose |
|------|---------|
| `app.py` | Main Dash app and API fetch logic |
| `ui/driftsummary.py` | Drift market summary component |
| `ui/mangosummary.py` | Mango market summary component |
| `ui/header.py` | Page header |
| `ui/footer.py` | Page footer |

### Shared test wallet
`.config/solana/id.json` — `PErPaQKaF6SjobprDiyyyLjKKTFTF5whL6cxFf1kwCC` (public, safe to use)

---

## Rust AMM — `prop-amm/`

### Algorithm: RegimeAdaptive

A constant-product AMM (x·y = k) with dynamic fees driven by three market-regime signals:

| Signal | Effect |
|--------|--------|
| **Volatility** (`ema_var`) | Raises fee up to +1.5% when variance spikes |
| **Order-flow toxicity** (`buy_press` / `sell_press`) | Raises fee up to +1% when one side dominates ≥ 60% of volume |
| **Regime** (`consec_zero` / `consec_full`) | Lowers fee by 2 bps after 5 consecutive zero-fills; raises by 2 bps after 15 consecutive full-fills |

Inventory skew nudges output ±0.8% to mean-revert the pool balance.
VPIN bucket resets every 50 × SCALE units of total volume.

### Fee bounds
| Constant | Value | bps |
|----------|-------|-----|
| `MIN_FEE` | 100 | 10.0 |
| `INIT_FEE` | 300 | 30.0 |
| `MAX_FEE` | 3000 | 300.0 |

Fees are stored in tenths-of-bps (e.g. 300 → 30.0 bps).

### Storage layout (1024 bytes, little-endian u64)
| Offset | Field | Type |
|--------|-------|------|
| 0 | `trade_count` | u64 |
| 8 | `ema_price` | u64 |
| 16 | `ema_var` | u64 |
| 24 | `buy_press` | u64 |
| 32 | `sell_press` | u64 |
| 40 | `regime_fee` | u64 |
| 48 | `inv_skew` | i64 as u64 |
| 56 | `consec_zero` | u64 |
| 64 | `consec_full` | u64 |
| 72 | `vpin_buy` | u64 |
| 80 | `vpin_total` | u64 |

### Dispatch table
| Tag | Handler |
|-----|---------|
| 0 (buy) | `compute_swap` |
| 1 (sell) | `compute_swap` |
| 2 | `after_swap` |
| 3 | return `NAME` bytes |
| 4 | return `MODEL_USED` bytes |

### Build (Solana BPF)
```bash
cd prop-amm
cargo build-sbf
```

### Build (native / unit tests)
```bash
cd prop-amm
cargo test --features no-entrypoint
```

### Key crates
- `pinocchio` — lightweight Solana program framework (BPF only; gated behind `no-entrypoint` for native builds)
- `prop_amm_submission_sdk` — platform SDK (`set_return_data_*`, `set_storage`)
- `wincode` — schema-based deserialization (`SchemaRead` derive, `deserialize`)

### Native compilation fix
All `pinocchio` imports and `process_instruction` are wrapped in
`#[cfg(not(feature = "no-entrypoint"))]` so that the pure AMM logic
(`compute_swap`, `after_swap`, helpers) compiles cleanly as a native Rust library
for platform testing.

---

## Git workflow

- Development branch: `claude/update-claude-md-aFSFC`
- Push: `git push -u origin claude/update-claude-md-aFSFC`
- Never push directly to `master` or `main`.
