pub mod librustposix {
    #[link(name = "rustposix")]
    extern "C" {
        pub(crate) fn lindrustinit();
    }
}

pub fn lind_lindrustinit() {
    unsafe {
        crate::librustposix::lindrustinit();
    }
}
