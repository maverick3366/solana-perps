use pinocchio::{account_info::AccountInfo, entrypoint, pubkey::Pubkey, ProgramResult};
use prop_amm_submission_sdk::{set_return_data_bytes, set_return_data_u64, set_storage};

const NAME: &str = "VirtualConc4";
const MODEL_USED: &str = "claude-sonnet-4-6";
const STORAGE_SIZE: usize = 1024;

const OFF_TRADE_COUNT: usize = 0;
const OFF_EMA_PRICE:   usize = 8;
const OFF_EMA_VAR:     usize = 16;
const OFF_BUY_PRESS:   usize = 24;
const OFF_SELL_PRESS:  usize = 32;
const OFF_REGIME_FEE:  usize = 40;
const OFF_INV_SKEW:    usize = 48;
const OFF_CONSEC_ZERO: usize = 56;
const OFF_CONSEC_FULL: usize = 64;
const OFF_VPIN_BUY:    usize = 72;
const OFF_VPIN_TOTAL:  usize = 80;

const SCALE: u64    = 1_000_000_000;
const SCALE128: u128 = 1_000_000_000;
const MIN_FEE: u64  = 100;
const MAX_FEE: u64  = 3000;
const INIT_FEE: u64 = 300;
const FEE_STEP: u64 = 20;

#[derive(wincode::SchemaRead)]
struct SwapArgs {
    side: u8,
    input_amount: u64,
    reserve_x: u64,
    reserve_y: u64,
    storage: [u8; STORAGE_SIZE],
}

#[derive(wincode::SchemaRead)]
struct AfterSwapArgs {
    tag: u8,
    side: u8,
    input_amount: u64,
    output_amount: u64,
    reserve_x: u64,
    reserve_y: u64,
    step: u64,
    storage: [u8; STORAGE_SIZE],
}

#[cfg(not(feature = "no-entrypoint"))]
entrypoint!(process_instruction);

pub fn process_instruction(
    _program_id: &Pubkey,
    _accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    if data.is_empty() {
        return Ok(());
    }
    match data[0] {
        0 | 1 => { set_return_data_u64(compute_swap(data)); }
        2     => { after_swap(data); }
        3     => { set_return_data_bytes(NAME.as_bytes()); }
        4     => { set_return_data_bytes(get_model_used().as_bytes()); }
        _     => {}
    }
    Ok(())
}

pub fn get_model_used() -> &'static str { MODEL_USED }

pub fn compute_swap(data: &[u8]) -> u64 {
    let args: SwapArgs = match wincode::deserialize(data) {
        Ok(a) => a,
        Err(_) => return 0,
    };
    let input = args.input_amount as u128;
    let rx    = args.reserve_x as u128;
    let ry    = args.reserve_y as u128;
    let side  = args.side;

    if input == 0 || rx == 0 || ry == 0 { return 0; }

    // 10 bps fee: multiply input by 9990/10000
    let amt = input * 9990 / 10000;

    // CONC=4: virtual reserves are 4x the actual reserves
    let vrx = rx * 4;
    let vry = ry * 4;

    // Standard x*y=k on virtual reserves
    // side=0: buy X (user sends Y) → vr_in=vry, vr_out=vrx, cap=rx-1
    // side=1: sell X (user sends X) → vr_in=vrx, vr_out=vry, cap=ry-1
    let (vr_in, vr_out, cap) = if side == 0 {
        (vry, vrx, rx.saturating_sub(1))
    } else {
        (vrx, vry, ry.saturating_sub(1))
    };

    let out = (vr_out * amt) / (vr_in + amt);
    let final_out = if out > cap { cap } else { out };

    final_out as u64
}

pub fn after_swap(data: &[u8]) {
    let args: AfterSwapArgs = match wincode::deserialize(data) {
        Ok(a) => a,
        Err(_) => return,
    };
    let side       = args.side;
    let amount_in  = args.input_amount;
    let amount_out = args.output_amount;
    let reserve_x  = args.reserve_x;
    let reserve_y  = args.reserve_y;
    let mut st     = args.storage;

    let trade_count  = rd64(&st, OFF_TRADE_COUNT);
    let ema_price    = rd64(&st, OFF_EMA_PRICE);
    let ema_var      = rd64(&st, OFF_EMA_VAR);
    let buy_press    = rd64(&st, OFF_BUY_PRESS);
    let sell_press   = rd64(&st, OFF_SELL_PRESS);
    let regime_fee   = { let r = rd64(&st, OFF_REGIME_FEE); if r == 0 { INIT_FEE } else { r } };
    let inv_skew     = rd64(&st, OFF_INV_SKEW) as i64;
    let consec_zero  = rd64(&st, OFF_CONSEC_ZERO);
    let consec_full  = rd64(&st, OFF_CONSEC_FULL);
    let vpin_buy     = rd64(&st, OFF_VPIN_BUY);
    let vpin_total   = rd64(&st, OFF_VPIN_TOTAL);

    let new_count = trade_count.saturating_add(1);

    let spot = if reserve_x > 0 {
        ((reserve_y as u128 * SCALE128) / reserve_x as u128) as u64
    } else { ema_price };

    let new_ema = if new_count == 1 { spot } else { ema_slow(ema_price, spot) };
    let dev     = u64_diff(spot, new_ema);
    let sq_dev  = ((dev as u128 * dev as u128) / SCALE128) as u64;
    let new_var = ema_slow(ema_var, sq_dev);

    let db = decay97(buy_press);
    let ds = decay97(sell_press);
    let (new_buy, new_sell) = if side == 1 {
        (db, ds.saturating_add(amount_in))
    } else {
        (db.saturating_add(amount_in), ds)
    };

    let delta: i64 = (amount_in as i64).saturating_sub(amount_out as i64);
    let raw_skew   = inv_skew.saturating_add(delta);
    let new_skew   = ((raw_skew as i128 * 990_000_000i128) / SCALE128 as i128) as i64;

    let bucket: u64 = 50 * SCALE;
    let new_vpin_buy   = if side == 0 { vpin_buy.saturating_add(amount_in) } else { vpin_buy };
    let new_vpin_total = vpin_total.saturating_add(amount_in);
    let (fvb, fvt) = if new_vpin_total >= bucket { (0u64, 0u64) } else { (new_vpin_buy, new_vpin_total) };

    let executed = amount_out > 0;
    let (new_zero, new_full, new_fee) = update_regime(consec_zero, consec_full, regime_fee, executed);

    wr64(&mut st, OFF_TRADE_COUNT,  new_count);
    wr64(&mut st, OFF_EMA_PRICE,    new_ema);
    wr64(&mut st, OFF_EMA_VAR,      new_var);
    wr64(&mut st, OFF_BUY_PRESS,    new_buy);
    wr64(&mut st, OFF_SELL_PRESS,   new_sell);
    wr64(&mut st, OFF_REGIME_FEE,   new_fee);
    wr64(&mut st, OFF_INV_SKEW,     new_skew as u64);
    wr64(&mut st, OFF_CONSEC_ZERO,  new_zero);
    wr64(&mut st, OFF_CONSEC_FULL,  new_full);
    wr64(&mut st, OFF_VPIN_BUY,     fvb);
    wr64(&mut st, OFF_VPIN_TOTAL,   fvt);

    set_storage(&st);
}

fn toxicity_fee(buy_vol: u64, sell_vol: u64) -> u128 {
    let max_extra: u128 = SCALE128 / 100;
    let total = buy_vol as u128 + sell_vol as u128;
    if total == 0 { return 0; }
    let dominant  = if buy_vol > sell_vol { buy_vol as u128 } else { sell_vol as u128 };
    let ratio     = (dominant * SCALE128) / total;
    let threshold = SCALE128 / 2 + SCALE128 / 10;
    if ratio <= threshold { return 0; }
    let excess = ratio - SCALE128 / 2;
    cu128((excess * max_extra) / (SCALE128 / 2), 0, max_extra)
}

fn volatility_fee(ema_var: u64, trade_count: u64) -> u128 {
    let max_extra: u128 = (SCALE128 * 15) / 1000;
    if trade_count < 10 || ema_var == 0 { return 0; }
    let calm: u64 = 1_000;
    if ema_var <= calm { return 0; }
    let excess = (ema_var - calm) as u128;
    let ratio  = (excess * SCALE128) / calm as u128;
    let capped = cu128(ratio, 0, 10 * SCALE128);
    (capped * max_extra) / (10 * SCALE128)
}

fn inventory_mult(skew: i64, side: u8) -> u128 {
    let max_shift: i128 = (SCALE128 as i128 * 8) / 1000;
    let max_skew: i64   = (100 * SCALE) as i64;
    let clamped = if skew > max_skew { max_skew } else if skew < -max_skew { -max_skew } else { skew };
    let norm: i128    = (clamped as i128 * SCALE128 as i128) / max_skew as i128;
    let abs_norm: i128 = if norm < 0 { -norm } else { norm };
    let shift: i128   = (abs_norm * max_shift) / SCALE128 as i128;
    let signed: i128  = if side == 0 {
        if skew > 0 { shift } else { -shift }
    } else {
        if skew > 0 { -shift } else { shift }
    };
    let mult = SCALE128 as i128 + signed;
    let lo = SCALE128 as i128 / 2;
    let hi = (SCALE128 as i128 * 3) / 2;
    (if mult < lo { lo } else if mult > hi { hi } else { mult }) as u128
}

fn update_regime(czero: u64, cfull: u64, fee: u64, executed: bool) -> (u64, u64, u64) {
    if executed {
        let nf = cfull.saturating_add(1);
        let new_fee = if nf > 15 { cu64(fee + FEE_STEP, MIN_FEE, MAX_FEE) } else { fee };
        (0, nf, new_fee)
    } else {
        let nz = czero.saturating_add(1);
        let new_fee = if nz > 5 { cu64(fee.saturating_sub(FEE_STEP), MIN_FEE, MAX_FEE) } else { fee };
        (nz, 0, new_fee)
    }
}

fn ema_slow(old: u64, new_val: u64) -> u64 {
    let alpha: u128     = SCALE128 / 10;
    let one_minus: u128 = SCALE128 - alpha;
    ((old as u128 * one_minus + new_val as u128 * alpha) / SCALE128) as u64
}

fn decay97(val: u64) -> u64 {
    ((val as u128 * 970_000_000u128) / SCALE128) as u64
}

fn u64_diff(a: u64, b: u64) -> u64 {
    if a >= b { a - b } else { b - a }
}

fn cu64(val: u64, lo: u64, hi: u64) -> u64 {
    if val < lo { lo } else if val > hi { hi } else { val }
}

fn cu128(val: u128, lo: u128, hi: u128) -> u128 {
    if val < lo { lo } else if val > hi { hi } else { val }
}

fn rd64(buf: &[u8; STORAGE_SIZE], off: usize) -> u64 {
    (buf[off]     as u64)
        | ((buf[off+1] as u64) << 8)
        | ((buf[off+2] as u64) << 16)
        | ((buf[off+3] as u64) << 24)
        | ((buf[off+4] as u64) << 32)
        | ((buf[off+5] as u64) << 40)
        | ((buf[off+6] as u64) << 48)
        | ((buf[off+7] as u64) << 56)
}

fn wr64(buf: &mut [u8; STORAGE_SIZE], off: usize, val: u64) {
    buf[off]   = (val & 0xFF) as u8;
    buf[off+1] = ((val >>  8) & 0xFF) as u8;
    buf[off+2] = ((val >> 16) & 0xFF) as u8;
    buf[off+3] = ((val >> 24) & 0xFF) as u8;
    buf[off+4] = ((val >> 32) & 0xFF) as u8;
    buf[off+5] = ((val >> 40) & 0xFF) as u8;
    buf[off+6] = ((val >> 48) & 0xFF) as u8;
    buf[off+7] = ((val >> 56) & 0xFF) as u8;
}
