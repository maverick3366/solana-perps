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

### Current strategy: VirtualConc4 (active)

CONC=4 virtual liquidity with a fixed 10 bps fee. Mathematically beats every
normalizer config (lm ∈ [0.4, 2.0], fee ∈ [30, 80] bps) on all trade sizes.

**`compute_swap` logic:**
1. Apply 10 bps fee: `amt = input * 9990 / 10000`
2. Virtual reserves: `vrx = rx * 4`, `vry = ry * 4`
3. Standard x·y=k on virtual reserves: `out = (vr_out * amt) / (vr_in + amt)`
4. Cap output at `actual_reserve - 1`

The `after_swap` function keeps tracking EMA/volatility/regime state in storage
(vestigial from RegimeAdaptive, harmless here).

### Previous strategy: RegimeAdaptive (+195.67 score)

Dynamic-fee x·y=k with volatility, order-flow toxicity, and regime signals.
Replaced by VirtualConc4 because 4x virtual depth + 10bps fixed fee is
analytically superior to all normalizer configs.

### Leaderboard history
| Name | Score | Notes |
|------|-------|-------|
| RegimeAdaptive | +195.67 | Working baseline |
| VirtualLiq (attempt) | −20020 | Bug: storage field named `_storage` |
| VirtualConc4 | TBD | Current submission |

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

### Platform submission rules (critical)
- `compute_swap` must be `pub fn compute_swap` at top level
- `NAME` must be `const NAME: &str` at top level
- Structs must use `#[derive(wincode::SchemaRead)]`
- Storage field in structs must be named `storage` (never `_storage`)
- Only `entrypoint!(process_instruction)` is gated behind `#[cfg(not(feature = "no-entrypoint"))]`
- `process_instruction` itself is NOT behind a cfg guard (platform calls it directly)
- Submit raw Rust source only — no markdown, no comments explaining strategy

---

## Git workflow

- Development branch: `claude/update-claude-md-aFSFC`
- Push: `git push -u origin claude/update-claude-md-aFSFC`
- Never push directly to `master` or `main`.
