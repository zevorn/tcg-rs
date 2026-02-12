use super::cpu::RiscvCpu;
use std::os::raw::c_int;

#[link(name = "m")]
extern "C" {
    fn fma(x: f64, y: f64, z: f64) -> f64;
    fn fmaf(x: f32, y: f32, z: f32) -> f32;
    fn rint(x: f64) -> f64;
    fn sqrt(x: f64) -> f64;
    fn sqrtf(x: f32) -> f32;
}

extern "C" {
    fn feclearexcept(excepts: c_int) -> c_int;
    fn fegetround() -> c_int;
    fn feraiseexcept(excepts: c_int) -> c_int;
    fn fesetround(round: c_int) -> c_int;
    fn fetestexcept(excepts: c_int) -> c_int;
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const FE_INVALID: c_int = 0x01;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const FE_DIVBYZERO: c_int = 0x04;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const FE_OVERFLOW: c_int = 0x08;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const FE_UNDERFLOW: c_int = 0x10;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const FE_INEXACT: c_int = 0x20;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const FE_ALL_EXCEPT: c_int = 0x3f;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const FE_TONEAREST: c_int = 0x0000;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const FE_DOWNWARD: c_int = 0x0400;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const FE_UPWARD: c_int = 0x0800;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const FE_TOWARDZERO: c_int = 0x0c00;

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
compile_error!("fenv constants need porting for this host architecture");

pub const FFLAGS_NV: u64 = 1 << 4;
pub const FFLAGS_DZ: u64 = 1 << 3;
pub const FFLAGS_OF: u64 = 1 << 2;
pub const FFLAGS_UF: u64 = 1 << 1;
pub const FFLAGS_NX: u64 = 1 << 0;
pub const FFLAGS_MASK: u64 = 0x1f;

pub const FRM_MASK: u64 = 0x7;
pub const FRM_DYN: u64 = 0x7;

const FCSR_RD_SHIFT: u64 = 5;

fn map_fenv_flags(flags: i32) -> u64 {
    let mut out = 0u64;
    if flags & FE_INEXACT != 0 {
        out |= FFLAGS_NX;
    }
    if flags & FE_UNDERFLOW != 0 {
        out |= FFLAGS_UF;
    }
    if flags & FE_OVERFLOW != 0 {
        out |= FFLAGS_OF;
    }
    if flags & FE_DIVBYZERO != 0 {
        out |= FFLAGS_DZ;
    }
    if flags & FE_INVALID != 0 {
        out |= FFLAGS_NV;
    }
    out
}

fn update_fflags(env: &mut RiscvCpu, flags: u64) {
    env.fflags = (env.fflags | flags) & FFLAGS_MASK;
}

fn set_invalid(env: &mut RiscvCpu) {
    update_fflags(env, FFLAGS_NV);
}

fn map_rm(env: &mut RiscvCpu, rm: u64) -> i32 {
    let mut rm = rm & FRM_MASK;
    if rm == FRM_DYN {
        rm = env.frm & FRM_MASK;
    }
    match rm {
        0 => FE_TONEAREST,
        1 => FE_TOWARDZERO,
        2 => FE_DOWNWARD,
        3 => FE_UPWARD,
        4 => FE_TONEAREST,
        _ => {
            set_invalid(env);
            FE_TONEAREST
        }
    }
}

fn with_fenv<T, F>(env: &mut RiscvCpu, rm: u64, f: F) -> T
where
    F: FnOnce() -> T,
{
    let (res, flags) = with_fenv_flags(env, rm, f);
    update_fflags(env, map_fenv_flags(flags));
    res
}

fn with_fenv_flags<T, F>(env: &mut RiscvCpu, rm: u64, f: F) -> (T, i32)
where
    F: FnOnce() -> T,
{
    let old_rm = unsafe { fegetround() };
    let old_exc = unsafe { fetestexcept(FE_ALL_EXCEPT) };
    unsafe {
        feclearexcept(FE_ALL_EXCEPT);
        fesetround(map_rm(env, rm));
    }
    let res = f();
    let raised = unsafe { fetestexcept(FE_ALL_EXCEPT) };
    unsafe {
        fesetround(old_rm);
        feclearexcept(FE_ALL_EXCEPT);
        feraiseexcept(old_exc);
    }
    (res, raised)
}

fn nanbox_f32(bits: u32) -> u64 {
    0xffff_ffff_0000_0000u64 | (bits as u64)
}

fn canonical_nan_f32() -> u32 {
    0x7fc0_0000
}

fn canonical_nan_f64() -> u64 {
    0x7ff8_0000_0000_0000u64
}

fn read_f32_bits(_env: &mut RiscvCpu, raw: u64) -> u32 {
    if (raw >> 32) as u32 != 0xffff_ffff {
        return canonical_nan_f32();
    }
    raw as u32
}

fn read_f32(env: &mut RiscvCpu, raw: u64) -> f32 {
    f32::from_bits(read_f32_bits(env, raw))
}

fn read_f64(raw: u64) -> f64 {
    f64::from_bits(raw)
}

fn is_nan_f32(bits: u32) -> bool {
    let exp = (bits >> 23) & 0xff;
    let frac = bits & 0x7fffff;
    exp == 0xff && frac != 0
}

fn is_nan_f64(bits: u64) -> bool {
    let exp = (bits >> 52) & 0x7ff;
    let frac = bits & 0x000f_ffff_ffff_ffff;
    exp == 0x7ff && frac != 0
}

fn is_snan_f32(bits: u32) -> bool {
    is_nan_f32(bits) && (bits & (1 << 22)) == 0
}

fn is_snan_f64(bits: u64) -> bool {
    is_nan_f64(bits) && (bits & (1 << 51)) == 0
}

fn fmin_f32(env: &mut RiscvCpu, a: u32, b: u32) -> u32 {
    let a_nan = is_nan_f32(a);
    let b_nan = is_nan_f32(b);
    if is_snan_f32(a) || is_snan_f32(b) {
        set_invalid(env);
    }
    if a_nan || b_nan {
        if a_nan && b_nan {
            return canonical_nan_f32();
        }
        return if a_nan { b } else { a };
    }
    let af = f32::from_bits(a);
    let bf = f32::from_bits(b);
    if af == bf {
        if af == 0.0 {
            return a | b;
        }
        return a;
    }
    if af < bf {
        a
    } else {
        b
    }
}

fn fmax_f32(env: &mut RiscvCpu, a: u32, b: u32) -> u32 {
    let a_nan = is_nan_f32(a);
    let b_nan = is_nan_f32(b);
    if is_snan_f32(a) || is_snan_f32(b) {
        set_invalid(env);
    }
    if a_nan || b_nan {
        if a_nan && b_nan {
            return canonical_nan_f32();
        }
        return if a_nan { b } else { a };
    }
    let af = f32::from_bits(a);
    let bf = f32::from_bits(b);
    if af == bf {
        if af == 0.0 {
            return a & b;
        }
        return a;
    }
    if af > bf {
        a
    } else {
        b
    }
}

fn fmin_f64(env: &mut RiscvCpu, a: u64, b: u64) -> u64 {
    let a_nan = is_nan_f64(a);
    let b_nan = is_nan_f64(b);
    if is_snan_f64(a) || is_snan_f64(b) {
        set_invalid(env);
    }
    if a_nan || b_nan {
        if a_nan && b_nan {
            return canonical_nan_f64();
        }
        return if a_nan { b } else { a };
    }
    let af = f64::from_bits(a);
    let bf = f64::from_bits(b);
    if af == bf {
        if af == 0.0 {
            return a | b;
        }
        return a;
    }
    if af < bf {
        a
    } else {
        b
    }
}

fn fmax_f64(env: &mut RiscvCpu, a: u64, b: u64) -> u64 {
    let a_nan = is_nan_f64(a);
    let b_nan = is_nan_f64(b);
    if is_snan_f64(a) || is_snan_f64(b) {
        set_invalid(env);
    }
    if a_nan || b_nan {
        if a_nan && b_nan {
            return canonical_nan_f64();
        }
        return if a_nan { b } else { a };
    }
    let af = f64::from_bits(a);
    let bf = f64::from_bits(b);
    if af == bf {
        if af == 0.0 {
            return a & b;
        }
        return a;
    }
    if af > bf {
        a
    } else {
        b
    }
}

fn fclass_f32(bits: u32) -> u64 {
    let sign = (bits >> 31) != 0;
    let exp = (bits >> 23) & 0xff;
    let frac = bits & 0x7fffff;
    let is_inf = exp == 0xff && frac == 0;
    let is_nan = exp == 0xff && frac != 0;
    let is_zero = exp == 0 && frac == 0;
    let is_sub = exp == 0 && frac != 0;
    let is_norm = exp != 0 && exp != 0xff;
    let is_snan = is_nan && (frac & (1 << 22)) == 0;
    let is_qnan = is_nan && !is_snan;
    let mut out = 0u64;
    if is_inf && sign {
        out |= 1 << 0;
    } else if is_norm && sign {
        out |= 1 << 1;
    } else if is_sub && sign {
        out |= 1 << 2;
    } else if is_zero && sign {
        out |= 1 << 3;
    } else if is_zero && !sign {
        out |= 1 << 4;
    } else if is_sub && !sign {
        out |= 1 << 5;
    } else if is_norm && !sign {
        out |= 1 << 6;
    } else if is_inf && !sign {
        out |= 1 << 7;
    } else if is_snan {
        out |= 1 << 8;
    } else if is_qnan {
        out |= 1 << 9;
    }
    out
}

fn fclass_f64(bits: u64) -> u64 {
    let sign = (bits >> 63) != 0;
    let exp = (bits >> 52) & 0x7ff;
    let frac = bits & 0x000f_ffff_ffff_ffff;
    let is_inf = exp == 0x7ff && frac == 0;
    let is_nan = exp == 0x7ff && frac != 0;
    let is_zero = exp == 0 && frac == 0;
    let is_sub = exp == 0 && frac != 0;
    let is_norm = exp != 0 && exp != 0x7ff;
    let is_snan = is_nan && (frac & (1 << 51)) == 0;
    let is_qnan = is_nan && !is_snan;
    let mut out = 0u64;
    if is_inf && sign {
        out |= 1 << 0;
    } else if is_norm && sign {
        out |= 1 << 1;
    } else if is_sub && sign {
        out |= 1 << 2;
    } else if is_zero && sign {
        out |= 1 << 3;
    } else if is_zero && !sign {
        out |= 1 << 4;
    } else if is_sub && !sign {
        out |= 1 << 5;
    } else if is_norm && !sign {
        out |= 1 << 6;
    } else if is_inf && !sign {
        out |= 1 << 7;
    } else if is_snan {
        out |= 1 << 8;
    } else if is_qnan {
        out |= 1 << 9;
    }
    out
}

#[no_mangle]
pub extern "C" fn helper_fadd_s(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f32(env, a);
    let bf = read_f32(env, b);
    let res = with_fenv(env, rm, || af + bf);
    nanbox_f32(res.to_bits())
}

#[no_mangle]
pub extern "C" fn helper_fsub_s(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f32(env, a);
    let bf = read_f32(env, b);
    let res = with_fenv(env, rm, || af - bf);
    nanbox_f32(res.to_bits())
}

#[no_mangle]
pub extern "C" fn helper_fmul_s(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f32(env, a);
    let bf = read_f32(env, b);
    let res = with_fenv(env, rm, || af * bf);
    nanbox_f32(res.to_bits())
}

#[no_mangle]
pub extern "C" fn helper_fdiv_s(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f32(env, a);
    let bf = read_f32(env, b);
    let res = with_fenv(env, rm, || af / bf);
    nanbox_f32(res.to_bits())
}

#[no_mangle]
pub extern "C" fn helper_fsqrt_s(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f32(env, a);
    let res = with_fenv(env, rm, || unsafe { sqrtf(af) });
    nanbox_f32(res.to_bits())
}

#[no_mangle]
pub extern "C" fn helper_fmadd_s(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    c: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f32(env, a);
    let bf = read_f32(env, b);
    let cf = read_f32(env, c);
    let res = with_fenv(env, rm, || unsafe { fmaf(af, bf, cf) });
    nanbox_f32(res.to_bits())
}

#[no_mangle]
pub extern "C" fn helper_fmsub_s(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    c: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f32(env, a);
    let bf = read_f32(env, b);
    let cf = read_f32(env, c);
    let res = with_fenv(env, rm, || unsafe { fmaf(af, bf, -cf) });
    nanbox_f32(res.to_bits())
}

#[no_mangle]
pub extern "C" fn helper_fnmsub_s(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    c: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f32(env, a);
    let bf = read_f32(env, b);
    let cf = read_f32(env, c);
    let res = with_fenv(env, rm, || unsafe { fmaf(-af, bf, cf) });
    nanbox_f32(res.to_bits())
}

#[no_mangle]
pub extern "C" fn helper_fnmadd_s(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    c: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f32(env, a);
    let bf = read_f32(env, b);
    let cf = read_f32(env, c);
    let res = with_fenv(env, rm, || unsafe { fmaf(-af, bf, -cf) });
    nanbox_f32(res.to_bits())
}

#[no_mangle]
pub extern "C" fn helper_fsgnj_s(env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let env = unsafe { &mut *env };
    let ab = read_f32_bits(env, a);
    let bb = read_f32_bits(env, b);
    let sign = bb & 0x8000_0000;
    let res = (ab & 0x7fff_ffff) | sign;
    nanbox_f32(res)
}

#[no_mangle]
pub extern "C" fn helper_fsgnjn_s(env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let env = unsafe { &mut *env };
    let ab = read_f32_bits(env, a);
    let bb = read_f32_bits(env, b);
    let sign = (!bb) & 0x8000_0000;
    let res = (ab & 0x7fff_ffff) | sign;
    nanbox_f32(res)
}

#[no_mangle]
pub extern "C" fn helper_fsgnjx_s(env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let env = unsafe { &mut *env };
    let ab = read_f32_bits(env, a);
    let bb = read_f32_bits(env, b);
    let sign = (ab ^ bb) & 0x8000_0000;
    let res = (ab & 0x7fff_ffff) | sign;
    nanbox_f32(res)
}

#[no_mangle]
pub extern "C" fn helper_fmin_s(env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let env = unsafe { &mut *env };
    let ab = read_f32_bits(env, a);
    let bb = read_f32_bits(env, b);
    nanbox_f32(fmin_f32(env, ab, bb))
}

#[no_mangle]
pub extern "C" fn helper_fmax_s(env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let env = unsafe { &mut *env };
    let ab = read_f32_bits(env, a);
    let bb = read_f32_bits(env, b);
    nanbox_f32(fmax_f32(env, ab, bb))
}

#[no_mangle]
pub extern "C" fn helper_feq_s(env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let env = unsafe { &mut *env };
    let ab = read_f32_bits(env, a);
    let bb = read_f32_bits(env, b);
    if is_snan_f32(ab) || is_snan_f32(bb) {
        set_invalid(env);
    }
    if is_nan_f32(ab) || is_nan_f32(bb) {
        return 0;
    }
    (f32::from_bits(ab) == f32::from_bits(bb)) as u64
}

#[no_mangle]
pub extern "C" fn helper_flt_s(env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let env = unsafe { &mut *env };
    let ab = read_f32_bits(env, a);
    let bb = read_f32_bits(env, b);
    if is_nan_f32(ab) || is_nan_f32(bb) {
        set_invalid(env);
        return 0;
    }
    (f32::from_bits(ab) < f32::from_bits(bb)) as u64
}

#[no_mangle]
pub extern "C" fn helper_fle_s(env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let env = unsafe { &mut *env };
    let ab = read_f32_bits(env, a);
    let bb = read_f32_bits(env, b);
    if is_nan_f32(ab) || is_nan_f32(bb) {
        set_invalid(env);
        return 0;
    }
    (f32::from_bits(ab) <= f32::from_bits(bb)) as u64
}

#[no_mangle]
pub extern "C" fn helper_fclass_s(env: *mut RiscvCpu, a: u64) -> u64 {
    let env = unsafe { &mut *env };
    let ab = read_f32_bits(env, a);
    fclass_f32(ab)
}

#[no_mangle]
pub extern "C" fn helper_fcvt_w_s(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f32(env, a);
    fcvt_i32(env, af as f64, rm)
}

#[no_mangle]
pub extern "C" fn helper_fcvt_wu_s(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f32(env, a);
    fcvt_u32(env, af as f64, rm)
}

#[no_mangle]
pub extern "C" fn helper_fcvt_l_s(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f32(env, a);
    fcvt_i64(env, af as f64, rm)
}

#[no_mangle]
pub extern "C" fn helper_fcvt_lu_s(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f32(env, a);
    fcvt_u64(env, af as f64, rm)
}

#[no_mangle]
pub extern "C" fn helper_fcvt_s_w(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let res = with_fenv(env, rm, || a as i32 as f32);
    nanbox_f32(res.to_bits())
}

#[no_mangle]
pub extern "C" fn helper_fcvt_s_wu(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let res = with_fenv(env, rm, || a as u32 as f32);
    nanbox_f32(res.to_bits())
}

#[no_mangle]
pub extern "C" fn helper_fcvt_s_l(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let res = with_fenv(env, rm, || a as i64 as f32);
    nanbox_f32(res.to_bits())
}

#[no_mangle]
pub extern "C" fn helper_fcvt_s_lu(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let res = with_fenv(env, rm, || a as f32);
    nanbox_f32(res.to_bits())
}

#[no_mangle]
pub extern "C" fn helper_fadd_d(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f64(a);
    let bf = read_f64(b);
    let res = with_fenv(env, rm, || af + bf);
    res.to_bits()
}

#[no_mangle]
pub extern "C" fn helper_fsub_d(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f64(a);
    let bf = read_f64(b);
    let res = with_fenv(env, rm, || af - bf);
    res.to_bits()
}

#[no_mangle]
pub extern "C" fn helper_fmul_d(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f64(a);
    let bf = read_f64(b);
    let res = with_fenv(env, rm, || af * bf);
    res.to_bits()
}

#[no_mangle]
pub extern "C" fn helper_fdiv_d(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f64(a);
    let bf = read_f64(b);
    let res = with_fenv(env, rm, || af / bf);
    res.to_bits()
}

#[no_mangle]
pub extern "C" fn helper_fsqrt_d(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f64(a);
    let res = with_fenv(env, rm, || unsafe { sqrt(af) });
    res.to_bits()
}

#[no_mangle]
pub extern "C" fn helper_fmadd_d(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    c: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f64(a);
    let bf = read_f64(b);
    let cf = read_f64(c);
    let res = with_fenv(env, rm, || unsafe { fma(af, bf, cf) });
    res.to_bits()
}

#[no_mangle]
pub extern "C" fn helper_fmsub_d(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    c: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f64(a);
    let bf = read_f64(b);
    let cf = read_f64(c);
    let res = with_fenv(env, rm, || unsafe { fma(af, bf, -cf) });
    res.to_bits()
}

#[no_mangle]
pub extern "C" fn helper_fnmsub_d(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    c: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f64(a);
    let bf = read_f64(b);
    let cf = read_f64(c);
    let res = with_fenv(env, rm, || unsafe { fma(-af, bf, cf) });
    res.to_bits()
}

#[no_mangle]
pub extern "C" fn helper_fnmadd_d(
    env: *mut RiscvCpu,
    a: u64,
    b: u64,
    c: u64,
    rm: u64,
) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f64(a);
    let bf = read_f64(b);
    let cf = read_f64(c);
    let res = with_fenv(env, rm, || unsafe { fma(-af, bf, -cf) });
    res.to_bits()
}

#[no_mangle]
pub extern "C" fn helper_fsgnj_d(_env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let sign = b & (1u64 << 63);
    (a & !(1u64 << 63)) | sign
}

#[no_mangle]
pub extern "C" fn helper_fsgnjn_d(_env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let sign = (!b) & (1u64 << 63);
    (a & !(1u64 << 63)) | sign
}

#[no_mangle]
pub extern "C" fn helper_fsgnjx_d(_env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let sign = (a ^ b) & (1u64 << 63);
    (a & !(1u64 << 63)) | sign
}

#[no_mangle]
pub extern "C" fn helper_fmin_d(env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let env = unsafe { &mut *env };
    fmin_f64(env, a, b)
}

#[no_mangle]
pub extern "C" fn helper_fmax_d(env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let env = unsafe { &mut *env };
    fmax_f64(env, a, b)
}

#[no_mangle]
pub extern "C" fn helper_feq_d(env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let env = unsafe { &mut *env };
    if is_snan_f64(a) || is_snan_f64(b) {
        set_invalid(env);
    }
    if is_nan_f64(a) || is_nan_f64(b) {
        return 0;
    }
    (f64::from_bits(a) == f64::from_bits(b)) as u64
}

#[no_mangle]
pub extern "C" fn helper_flt_d(env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let env = unsafe { &mut *env };
    if is_nan_f64(a) || is_nan_f64(b) {
        set_invalid(env);
        return 0;
    }
    (f64::from_bits(a) < f64::from_bits(b)) as u64
}

#[no_mangle]
pub extern "C" fn helper_fle_d(env: *mut RiscvCpu, a: u64, b: u64) -> u64 {
    let env = unsafe { &mut *env };
    if is_nan_f64(a) || is_nan_f64(b) {
        set_invalid(env);
        return 0;
    }
    (f64::from_bits(a) <= f64::from_bits(b)) as u64
}

#[no_mangle]
pub extern "C" fn helper_fclass_d(env: *mut RiscvCpu, a: u64) -> u64 {
    let _ = env;
    fclass_f64(a)
}

#[no_mangle]
pub extern "C" fn helper_fcvt_w_d(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f64(a);
    fcvt_i32(env, af, rm)
}

#[no_mangle]
pub extern "C" fn helper_fcvt_wu_d(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f64(a);
    fcvt_u32(env, af, rm)
}

#[no_mangle]
pub extern "C" fn helper_fcvt_l_d(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f64(a);
    fcvt_i64(env, af, rm)
}

#[no_mangle]
pub extern "C" fn helper_fcvt_lu_d(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f64(a);
    fcvt_u64(env, af, rm)
}

#[no_mangle]
pub extern "C" fn helper_fcvt_d_w(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let res = with_fenv(env, rm, || a as i32 as f64);
    res.to_bits()
}

#[no_mangle]
pub extern "C" fn helper_fcvt_d_wu(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let res = with_fenv(env, rm, || a as u32 as f64);
    res.to_bits()
}

#[no_mangle]
pub extern "C" fn helper_fcvt_d_l(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let res = with_fenv(env, rm, || a as i64 as f64);
    res.to_bits()
}

#[no_mangle]
pub extern "C" fn helper_fcvt_d_lu(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let res = with_fenv(env, rm, || a as f64);
    res.to_bits()
}

#[no_mangle]
pub extern "C" fn helper_fcvt_s_d(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f64(a);
    let res = with_fenv(env, rm, || af as f32);
    nanbox_f32(res.to_bits())
}

#[no_mangle]
pub extern "C" fn helper_fcvt_d_s(env: *mut RiscvCpu, a: u64, rm: u64) -> u64 {
    let env = unsafe { &mut *env };
    let af = read_f32(env, a) as f64;
    let res = with_fenv(env, rm, || af);
    res.to_bits()
}

const I32_MIN_F64: f64 = -2147483648.0;
const I32_MAX_PLUS1_F64: f64 = 2147483648.0;
const I64_MIN_F64: f64 = -9223372036854775808.0;
const I64_MAX_PLUS1_F64: f64 = 9223372036854775808.0;
const U32_MAX_PLUS1_F64: f64 = 4294967296.0;
const U64_MAX_PLUS1_F64: f64 = 18446744073709551616.0;

fn fcvt_i32(env: &mut RiscvCpu, val: f64, rm: u64) -> u64 {
    if val.is_nan() {
        set_invalid(env);
        return (i32::MAX as i64) as u64;
    }
    if val.is_infinite() {
        set_invalid(env);
        return if val.is_sign_negative() {
            (i32::MIN as i64) as u64
        } else {
            (i32::MAX as i64) as u64
        };
    }
    let (rounded, flags) = with_fenv_flags(env, rm, || unsafe { rint(val) });
    if flags & FE_INVALID != 0
        || !rounded.is_finite()
        || !(I32_MIN_F64..I32_MAX_PLUS1_F64).contains(&rounded)
    {
        set_invalid(env);
        return if val.is_sign_negative() {
            (i32::MIN as i64) as u64
        } else {
            (i32::MAX as i64) as u64
        };
    }
    update_fflags(env, map_fenv_flags(flags));
    (rounded as i32 as i64) as u64
}

fn fcvt_u32(env: &mut RiscvCpu, val: f64, rm: u64) -> u64 {
    if val.is_nan() {
        set_invalid(env);
        return u32::MAX as u64;
    }
    if val.is_infinite() {
        set_invalid(env);
        return if val.is_sign_negative() {
            0
        } else {
            u32::MAX as u64
        };
    }
    if val < 0.0 {
        set_invalid(env);
        return 0;
    }
    let (rounded, flags) = with_fenv_flags(env, rm, || unsafe { rint(val) });
    if flags & FE_INVALID != 0
        || !rounded.is_finite()
        || !(0.0..U32_MAX_PLUS1_F64).contains(&rounded)
    {
        set_invalid(env);
        return u32::MAX as u64;
    }
    update_fflags(env, map_fenv_flags(flags));
    rounded as u32 as u64
}

fn fcvt_i64(env: &mut RiscvCpu, val: f64, rm: u64) -> u64 {
    if val.is_nan() {
        set_invalid(env);
        return i64::MAX as u64;
    }
    if val.is_infinite() {
        set_invalid(env);
        return if val.is_sign_negative() {
            i64::MIN as u64
        } else {
            i64::MAX as u64
        };
    }
    let (rounded, flags) = with_fenv_flags(env, rm, || unsafe { rint(val) });
    if flags & FE_INVALID != 0
        || !rounded.is_finite()
        || !(I64_MIN_F64..I64_MAX_PLUS1_F64).contains(&rounded)
    {
        set_invalid(env);
        return if val.is_sign_negative() {
            i64::MIN as u64
        } else {
            i64::MAX as u64
        };
    }
    update_fflags(env, map_fenv_flags(flags));
    rounded as i64 as u64
}

fn fcvt_u64(env: &mut RiscvCpu, val: f64, rm: u64) -> u64 {
    if val.is_nan() {
        set_invalid(env);
        return u64::MAX;
    }
    if val.is_infinite() {
        set_invalid(env);
        return if val.is_sign_negative() { 0 } else { u64::MAX };
    }
    if val < 0.0 {
        set_invalid(env);
        return 0;
    }
    let (rounded, flags) = with_fenv_flags(env, rm, || unsafe { rint(val) });
    if flags & FE_INVALID != 0
        || !rounded.is_finite()
        || !(0.0..U64_MAX_PLUS1_F64).contains(&rounded)
    {
        set_invalid(env);
        return u64::MAX;
    }
    update_fflags(env, map_fenv_flags(flags));
    rounded as u64
}

#[no_mangle]
pub extern "C" fn helper_fcsr_read(env: *mut RiscvCpu) -> u64 {
    let env = unsafe { &mut *env };
    let fflags = env.fflags & FFLAGS_MASK;
    let frm = env.frm & FRM_MASK;
    fflags | (frm << FCSR_RD_SHIFT)
}

#[no_mangle]
pub extern "C" fn helper_fcsr_write(env: *mut RiscvCpu, val: u64) -> u64 {
    let env = unsafe { &mut *env };
    let old =
        (env.fflags & FFLAGS_MASK) | ((env.frm & FRM_MASK) << FCSR_RD_SHIFT);
    env.fflags = val & FFLAGS_MASK;
    env.frm = (val >> FCSR_RD_SHIFT) & FRM_MASK;
    old
}
