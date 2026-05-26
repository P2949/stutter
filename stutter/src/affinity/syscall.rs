use std::{io, mem};

use stutter_core::ids::Tid;

use super::cpu_mask::{CpuMask, cpu_set_size};

pub fn read_allowed_mask(tid: Tid) -> io::Result<CpuMask> {
    read_allowed_mask_raw(tid.as_u32())
}

pub fn read_allowed_mask_raw(tid: u32) -> io::Result<CpuMask> {
    let set = sched_getaffinity(tid)?;
    Ok(mask_from_cpu_set(&set))
}

pub fn set_affinity(tid: Tid, mask: &CpuMask) -> io::Result<()> {
    set_affinity_raw(tid.as_u32(), mask)
}

pub fn set_affinity_raw(tid: u32, mask: &CpuMask) -> io::Result<()> {
    sched_setaffinity(tid, &mask_to_cpu_set(mask))
}

fn sched_getaffinity(tid: u32) -> io::Result<libc::cpu_set_t> {
    // SAFETY: cpu_set_t is a plain C bitset; zeroed storage is valid before libc fills it.
    let mut set = unsafe { mem::zeroed::<libc::cpu_set_t>() };
    // SAFETY: set points to initialized storage of the expected size and tid is passed as the
    // kernel pid_t expected by sched_getaffinity.
    let result = unsafe {
        libc::sched_getaffinity(
            tid as libc::pid_t,
            mem::size_of::<libc::cpu_set_t>(),
            &mut set,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(set)
}

fn sched_setaffinity(tid: u32, set: &libc::cpu_set_t) -> io::Result<()> {
    // SAFETY: set is a valid cpu_set_t built by mask_to_cpu_set, and the size argument matches.
    let result = unsafe {
        libc::sched_setaffinity(tid as libc::pid_t, mem::size_of::<libc::cpu_set_t>(), set)
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn mask_to_cpu_set(mask: &CpuMask) -> libc::cpu_set_t {
    let mut set = cpu_set_new();
    for cpu in mask.cpus() {
        cpu_set_add(&mut set, cpu);
    }
    set
}

fn mask_from_cpu_set(set: &libc::cpu_set_t) -> CpuMask {
    let mut mask = CpuMask::empty();
    for cpu in 0..cpu_set_size() {
        if cpu_set_contains(set, cpu) {
            mask.set(cpu);
        }
    }
    mask
}

fn cpu_set_new() -> libc::cpu_set_t {
    // SAFETY: cpu_set_t is a plain C bitset; zeroed is a valid empty value before CPU_ZERO
    // normalizes the libc-specific representation.
    let mut set = unsafe { mem::zeroed::<libc::cpu_set_t>() };
    // SAFETY: set points to a valid mutable cpu_set_t allocated above.
    unsafe {
        libc::CPU_ZERO(&mut set);
    }
    set
}

fn cpu_set_add(set: &mut libc::cpu_set_t, cpu: u32) {
    // SAFETY: cpu comes from CpuMask::cpus(), which only yields values below CPU_SETSIZE, and set
    // is a valid mutable cpu_set_t.
    unsafe {
        libc::CPU_SET(cpu as usize, set);
    }
}

fn cpu_set_contains(set: &libc::cpu_set_t, cpu: u32) -> bool {
    // SAFETY: callers iterate cpu values below CPU_SETSIZE, and set is a valid cpu_set_t.
    unsafe { libc::CPU_ISSET(cpu as usize, set) }
}
