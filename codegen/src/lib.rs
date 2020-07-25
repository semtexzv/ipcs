extern crate proc_macro;

use proc_macro::TokenStream;

use quote::quote;
use syn::parse_macro_input;

/// Check the function provided to be in correct format and generate actual entrypoint, which
/// will invoke this function. TODO: This will be replaced by more general solution, where the actual function
/// will not have to execute static functions to load arguments (either by providing streams, or some other abstraction
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
