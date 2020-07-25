extern crate proc_macro;

use proc_macro::{TokenStream};

use syn::parse_macro_input;
use quote::quote;


#[proc_macro_attribute]
pub fn entrypoint(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let fun = parse_macro_input!(item as syn::ItemFn);

    if fun.sig.inputs.len() > 0 {
        panic!("Function must have 0 arguments")
    }
    let name = fun.sig.ident.clone();

    let res = quote! {
        #fun

        #[no_mangle]
        fn _ipcs_start() {
            #name();
        }
    };

    res.into()
}