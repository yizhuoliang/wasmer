use rustposix::{
    lind_lindrustfinalize, lind_lindrustinit, lind_rustposix_thread_init, lind_write_inner,
};

use super::*;
use crate::syscalls::*;

pub fn lind_write<M: MemorySize>(
    mut ctx: FunctionEnvMut<'_, WasiEnv>,
    iovs: WasmPtr<M::Offset, M>,
    len: i32,
) -> Result<Errno, WasiError> {
    // Here, we should fetch the pid aka cageid aka wasiEnv id,
    // but I'm suprised that there's no "ID" field for WasiEnv
    // let mut env = ctx.data();
    lind_lindrustinit(0);
    lind_rustposix_thread_init(1, 0);
    let mut env = ctx.data();
    let memory = unsafe { env.memory_view(&ctx) };
    let offset = iovs.offset().into() as u64;
    let base = memory.data_ptr() as u64;
    let ptr: *const libc::c_void = unsafe {
        // Cast u64 to usize, then usize to *const libc::c_void
        (offset + base) as usize as *const libc::c_void
    };
    lind_write_inner(1, ptr, len as usize, 1);
    lind_lindrustfinalize();
    Ok(Errno::Success)
}
