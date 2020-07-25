extern crate codegen;
pub use codegen::entrypoint;

#[link(wasm_import_module = "_ipcs")]
extern "C" {
    pub fn _ipcs_arg_count() -> u32;
    pub fn _ipcs_arg_len(i: u32) -> u32;
    pub fn _ipcs_arg_read(i: u32, ptr: u32, offset: u32, len: u32) -> u32;
    pub fn _ipcs_ret(ptr: u32, len: u32);
}

pub fn arg_count() -> u32 {
    unsafe { _ipcs_arg_count() }
}

pub fn arg_buf(i: u32) -> Box<[u8]> {
    unsafe {
        let len = _ipcs_arg_len(i);
        let mut vec: Vec<u8> = Vec::with_capacity(len as usize);
        vec.resize(len as usize, 0);
        let read = _ipcs_arg_read(i, vec.as_ptr() as _, 0, len);
        assert!(read <= len, "Read invalid length");
        vec.into_boxed_slice()
    }
}

pub fn ret(data: &[u8]) {
    unsafe {
        _ipcs_ret(data.as_ptr() as u32, data.len() as u32);
    }
}
