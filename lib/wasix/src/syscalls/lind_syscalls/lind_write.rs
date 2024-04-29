use rustposix::lind_write_inner;

use super::*;
use crate::syscalls::*;

pub fn lind_write<M: MemorySize>(
    mut ctx: FunctionEnvMut<'_, WasiEnv>,
    fd: WasiFd,
    iovs: WasmPtr<__wasi_ciovec_t<M>, M>,
    iovs_len: M::Offset,
    nwritten: WasmPtr<M::Offset, M>,
) -> Result<Errno, WasiError> {
    // Here, we should fetch the pid aka cageid aka wasiEnv id,
    // but I'm suprised that there's no "ID" field for WasiEnv
    // let mut env = ctx.data();
    // lind_write_inner(1, buf, count, 1);
    Ok(Errno::Success)
}
