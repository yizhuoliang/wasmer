use rustposix::{
    lind_lindrustfinalize, lind_lindrustinit, lind_rustposix_thread_init, lind_syscall_inner,
};

use super::*;
use crate::syscalls::*;

// This is exposed to the glibc, in the WASM user space
pub fn lind_syscall<M: MemorySize>(
    mut ctx: FunctionEnvMut<'_, WasiEnv>,
    call_number: u32,
    call_name: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
    arg6: u64,
) -> Result<Errno, WasiError> {
    // lind_rustposix_thread_init(1, 0);
    // Get the linear memory start address
    // Ideally we should only do this once for each cage,
    // but here I pass it for every call, for simplicity
    let mut env = ctx.data();
    let memory = unsafe { env.memory_view(&ctx) };
    let start_address = memory.data_ptr() as u64;

    lind_syscall_inner(
        call_number,
        call_name,
        start_address,
        arg1,
        arg2,
        arg3,
        arg4,
        arg5,
        arg6,
    );

    Ok(Errno::Success)
}
