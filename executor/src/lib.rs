use wasmer::*;

pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;


fn _ipcs_arg_count(ctx: &mut Ctx) -> u32 {
    println!("_ipcs_arg_count");
    let data = ctx.data.cast::<IoData>();
    (unsafe { &*data }).args.len() as _
}

fn _ipcs_arg_len(ctx: &mut Ctx, idx: u32) -> u32 {
    println!("_ipcs_arg_len: {}", idx);
    let data = ctx.data.cast::<IoData>();
    (unsafe { &*data }).args[idx as usize].len() as u32
}

fn _ipcs_arg_read(ctx: &mut Ctx, idx: u32, ptr: u32, offset: u32, len: u32) -> u32 {
    println!("_ipcs_arg_read: {}, {}, {}, {}", idx, ptr, offset, len);
    let data = ctx.data.cast::<IoData>();
    let arg = (unsafe { &*data }).args[idx as usize];

    let mem = ctx.memory(0);
    let view = mem.view::<u8>();

    let iter = arg[offset as usize..(offset + len) as usize].iter()
        .zip(view[ptr as usize..(ptr + len) as usize].iter());

    let mut count = 0;
    for (src, dst) in iter {
        dst.set(*src);
        count += 1;
    }
    count
}


fn _ipcs_ret(ctx: &mut Ctx, ptr: u32, len: u32) {

    let data = ctx.data.cast::<IoData>();
    let mem = ctx.memory(0).view::<u8>();
    for v in &mem[ptr as usize .. (ptr + len) as usize] {
        unsafe { (&mut *data).ret.push(v.get()); }
    }
}

struct IoData<'a> {
    ret: Vec<u8>,
    args: &'a [&'a [u8]],
}

// Execute wasm
pub fn exec(wasm: &[u8], args: &[&[u8]]) -> Result<Vec<u8>> {
    let imports = imports! {
        "_ipcs" => {
            "_ipcs_arg_count" => func!(_ipcs_arg_count),
            "_ipcs_arg_len" => func!(_ipcs_arg_len),
            "_ipcs_arg_read" => func!(_ipcs_arg_read),
            "_ipcs_ret" => func!(_ipcs_ret),
        },
    };


    let mut instance = instantiate(wasm, &imports)?;

    let mut io_data = IoData {
        ret: vec![],
        args,
    };

    instance.context_mut().data = &mut io_data as *mut _ as *mut _;

    let entrypoint = format!("_ipcs_start");

    let main = instance.dyn_func(&entrypoint)?;
    assert_eq!(main.signature().params().len(), 0);

    println!("Executing: {:?}", io_data.args);
    instance.call(&entrypoint, &[])?;
    println!("Res: {:?}", io_data.ret);

    return Ok(io_data.ret);
}
