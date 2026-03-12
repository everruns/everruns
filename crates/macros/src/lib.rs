// Proc macros for Everruns
//
// Decision: #[policy] attribute macro for declarative policy enforcement at the service layer.
// The macro injects `POLICY.evaluate(caller)?;` at the top of the function body,
// finding the `caller: &Caller` parameter by type automatically.

use proc_macro::TokenStream;
use quote::quote;
use syn::{FnArg, ItemFn, PatType, Type, TypeReference, parse_macro_input};

/// Declarative policy enforcement for service methods.
///
/// Injects a policy evaluation check at the start of the function body.
/// The function must have a `caller: &Caller` parameter (the name can vary,
/// but the type must be `&Caller`).
///
/// # Usage
///
/// ```rust,ignore
/// use everruns_macros::policy;
///
/// const HARNESS_MANAGE: Policy = Policy {
///     id: "harness.manage",
///     rules: &[Rule::UserHasPermission(Permission::OrgHarnessesManage)],
/// };
///
/// impl HarnessService {
///     #[policy(HARNESS_MANAGE)]
///     pub async fn create(&self, caller: &Caller, req: CreateHarnessRequest) -> Result<Harness> {
///         // policy check is injected here automatically
///     }
/// }
/// ```
///
/// # Multiple Policies
///
/// Stack multiple `#[policy]` attributes for methods requiring several checks:
///
/// ```rust,ignore
/// #[policy(POLICY_A)]
/// #[policy(POLICY_B)]
/// pub async fn special_op(&self, caller: &Caller) -> Result<()> {
///     // both policies checked in order (A first, then B)
/// }
/// ```
///
/// # Compile Errors
///
/// - If the function has no `&Caller` parameter, a compile error is raised.
#[proc_macro_attribute]
pub fn policy(attr: TokenStream, item: TokenStream) -> TokenStream {
    let policy_path = parse_macro_input!(attr as syn::Path);
    let mut func = parse_macro_input!(item as ItemFn);

    // Find the &Caller parameter by scanning for `&Caller` type
    let caller_ident = func.sig.inputs.iter().find_map(|arg| {
        let FnArg::Typed(PatType { pat, ty, .. }) = arg else {
            return None;
        };
        let Type::Reference(TypeReference { elem, .. }) = ty.as_ref() else {
            return None;
        };
        let Type::Path(tp) = elem.as_ref() else {
            return None;
        };
        let is_caller = tp
            .path
            .segments
            .last()
            .is_some_and(|s| s.ident == "Caller");
        if is_caller && let syn::Pat::Ident(pi) = pat.as_ref() {
            return Some(pi.ident.clone());
        }
        None
    });

    let caller = match caller_ident {
        Some(ident) => ident,
        None => {
            return syn::Error::new_spanned(
                &func.sig,
                "#[policy] requires a `caller: &Caller` parameter",
            )
            .to_compile_error()
            .into();
        }
    };

    // Prepend policy evaluation to the function body
    let original_stmts = &func.block.stmts;
    func.block = syn::parse_quote!({
        #policy_path.evaluate(#caller)?;
        #(#original_stmts)*
    });

    quote!(#func).into()
}
