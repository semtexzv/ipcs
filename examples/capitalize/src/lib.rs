use ipcs;

#[ipcs::entrypoint]
pub fn exec_bytes() {
    let v1 = ipcs::arg_buf(0);
    let mut v = String::from_utf8_lossy(&v1).to_string();
    v.make_ascii_uppercase();
    ipcs::ret(&v.as_bytes());
}
