use aya_ebpf::programs::TracePointContext;

#[inline(always)]
pub(crate) fn read_i32(ctx: &TracePointContext, offset: usize, out: &mut i32) -> bool {
    match unsafe { ctx.read_at::<i32>(offset) } {
        Ok(value) => {
            *out = value;
            true
        }
        Err(_) => false,
    }
}

#[inline(always)]
pub(crate) fn read_i64(ctx: &TracePointContext, offset: usize, out: &mut i64) -> bool {
    match unsafe { ctx.read_at::<i64>(offset) } {
        Ok(value) => {
            *out = value;
            true
        }
        Err(_) => false,
    }
}

#[inline(always)]
pub(crate) fn read_u32(ctx: &TracePointContext, offset: usize, out: &mut u32) -> bool {
    match unsafe { ctx.read_at::<u32>(offset) } {
        Ok(value) => {
            *out = value;
            true
        }
        Err(_) => false,
    }
}

#[inline(always)]
pub(crate) fn read_u64(ctx: &TracePointContext, offset: usize, out: &mut u64) -> bool {
    match unsafe { ctx.read_at::<u64>(offset) } {
        Ok(value) => {
            *out = value;
            true
        }
        Err(_) => false,
    }
}

#[inline(always)]
pub(crate) fn read_comm16(ctx: &TracePointContext, offset: usize, out: &mut [u8; 16]) -> bool {
    match unsafe { ctx.read_at::<[u8; 16]>(offset) } {
        Ok(value) => {
            *out = value;
            true
        }
        Err(_) => false,
    }
}

#[inline(always)]
pub(crate) fn read_optional_u32(ctx: &TracePointContext, offset: u32, out: &mut u32) -> bool {
    if offset == 0 {
        false
    } else {
        read_u32(ctx, offset as usize, out)
    }
}

#[inline(always)]
pub(crate) fn read_optional_u64(ctx: &TracePointContext, offset: u32, out: &mut u64) -> bool {
    if offset == 0 {
        false
    } else {
        read_u64(ctx, offset as usize, out)
    }
}

#[inline(always)]
pub(crate) fn read_sequence_field(
    ctx: &TracePointContext,
    offset: u32,
    size: u32,
    out: &mut u64,
) -> bool {
    if offset == 0 {
        false
    } else if size >= 8 {
        read_u64(ctx, offset as usize, out)
    } else {
        let mut value: u32 = 0;
        if !read_u32(ctx, offset as usize, &mut value) {
            return false;
        }
        *out = value as u64;
        true
    }
}
