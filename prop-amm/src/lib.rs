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

const SCALE: u64     = 1_000_000_000;
const SCALE128: u128 = 1_000_000_000;
const MIN_FEE: u64   = 100;
const MAX_FEE: u64   = 3000;
const INIT_FEE: u64  = 300;
const FEE_STEP: u64  = 20;

#[derive(wincode::SchemaRead)]
struct ComputeSwapInstruction {
    side: u8,
    input_amount: u64,
    reserve_x: u64,
    reserve_y: u64,
    _storage: [u8; STORAGE_SIZE],
}

#[derive(wincode::SchemaRead)]
struct AfterSwapInstruction {
    _tag: u8,
    side: u8,
    input_amount: u64,
    output_amount: u64,
    reserve_x: u64,
    reserve_y: u64,
    _step: u64,
    storage: [u8; STORAGE_SIZE],
}

#[cfg(not(feature = "no-entrypoint"))]
entrypoint!(process_instruction);

pub fn process_instruction(
    _program_id: &Pubkey,
    _accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    if instruction_data.is_empty() {
        return Ok(());
    }
    match instruction_data[0] {
        0 | 1 => {
            let output = compute_swap(instruction_data);
            set_return_data_u64(output);
        }
        2 => {
            after_swap(instruction_data);
        }
        3 => set_return_data_bytes(NAME.as_bytes()),
        4 => set_return_data_bytes(get_model_used().as_bytes()),
        _ => {}
    }
    Ok(())
}

pub fn get_model_used() -> &'static str {
    MODEL_USED
}

pub fn compute_swap(data: &[u8]) -> u64 {
    let decoded: ComputeSwapInstruction = match wincode::deserialize(data) {
        Ok(decoded) => decoded,
        Err(_) => return 0,
    };
    let side = decoded.side;
    let input_amount = decoded.input_amount as u128;
    let reserve_x = decoded.reserve_x as u128;
    let reserve_y = decoded.reserve_y as u128;

    if reserve_x == 0 || reserve_y == 0 {
        return 0;
    }

    let amt = input_amount * 9990 / 10000;
    let vrx = reserve_x * 4;
    let vry = reserve_y * 4;

    match side {
        0 => {
            let new_vry = vry + amt;
            let out = (vrx * amt) / new_vry;
            let cap = reserve_x.saturating_sub(1);
            if out > cap { cap as u64 } else { out as u64 }
        }
        1 => {
            let new_vrx = vrx + amt;
            let out = (vry * amt) / new_vrx;
            let cap = reserve_y.saturating_sub(1);
            if out > cap { cap as u64 } else { out as u64 }
        }
        _ => 0,
    }
}

pub fn after_swap(data: &[u8]) {
    let decoded: AfterSwapInstruction = match wincode::deserialize(data) {
        Ok(decoded) => decoded,
        Err(_) => return,
    };
    let side       = decoded.side;
    let amount_in  = decoded.input_amount;
    let amount_out = decoded.output_amount;
    let reserve_x  = decoded.reserve_x;
    let reserve_y  = decoded.reserve_y;
    let mut st     = decoded.storage;

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

    let bucket: u64    = 50 * SCALE;
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

fn rd64(buf: &[u8; STORAGE_SIZE], off: usize) -> u64 {
    (buf[off] as u64)
        | ((buf[off + 1] as u64) << 8)
        | ((buf[off + 2] as u64) << 16)
        | ((buf[off + 3] as u64) << 24)
        | ((buf[off + 4] as u64) << 32)
        | ((buf[off + 5] as u64) << 40)
        | ((buf[off + 6] as u64) << 48)
        | ((buf[off + 7] as u64) << 56)
}

fn wr64(buf: &mut [u8; STORAGE_SIZE], off: usize, val: u64) {
    buf[off]     = (val & 0xFF) as u8;
    buf[off + 1] = ((val >> 8) & 0xFF) as u8;
    buf[off + 2] = ((val >> 16) & 0xFF) as u8;
    buf[off + 3] = ((val >> 24) & 0xFF) as u8;
    buf[off + 4] = ((val >> 32) & 0xFF) as u8;
    buf[off + 5] = ((val >> 40) & 0xFF) as u8;
    buf[off + 6] = ((val >> 48) & 0xFF) as u8;
    buf[off + 7] = ((val >> 56) & 0xFF) as u8;
}
