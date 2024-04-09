use libc::STDOUT_FILENO;
use rustposix::*;

fn main() {
    lind_lindrustinit(0); // Call the external function
    let my_string = "Hello, world!";
    let c_buf = my_string.as_ptr() as *const libc::c_void;
    let buf_len = my_string.len();
    lind_write(STDOUT_FILENO, c_buf, buf_len, 1);
    lind_lindrustfinalize();
}
