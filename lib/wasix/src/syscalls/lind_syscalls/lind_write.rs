use rustposix::{lind_lindrustinit, lind_write_inner};

use super::*;
use crate::syscalls::*;

pub fn lind_write<M: MemorySize>(
    mut ctx: FunctionEnvMut<'_, WasiEnv>,
    iovs: i32,
    len: i32,
) -> Result<Errno, WasiError> {
    // Here, we should fetch the pid aka cageid aka wasiEnv id,
    // but I'm suprised that there's no "ID" field for WasiEnv
    // let mut env = ctx.data();
    lind_lindrustinit(0);
    let mut env = ctx.data();
    let memory = unsafe { env.memory_view(&ctx) };
    let buf_addr = iovs as i64 + memory;
    lind_write_inner(1, buf_addr, len, 1);
    Ok(Errno::Success)
}
