use rustposix::{
    lind_lindrustfinalize, lind_lindrustinit, lind_rustposix_thread_init, lind_write_inner,
};

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
    lind_rustposix_thread_init(1, 0);
    let mut env = ctx.data();
    let memory = unsafe { env.memory_view(&ctx) };
    let buf_addr = iovs as i64 + memory.data_ptr() as i64;
    let ptr: *const libc::c_void = unsafe {
        // Cast i64 to usize, then usize to *const libc::c_void
        buf_addr as usize as *const libc::c_void
    };
    lind_write_inner(1, ptr, len as usize, 1);
    lind_lindrustfinalize();
    Ok(Errno::Success)
}
