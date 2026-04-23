// Proc macros for Everruns
//
// Currently provides `#[audit]` for AOP audit logging on service methods.
//
// NOTE: `#[policy]` was retired. Command-layer authorization is enforced
// uniformly by `Command::run` (see `specs/permissions.md` and
// `crates/server/src/domains/common.rs`). Service-layer policy checks were
// redundant and re-introduced a SaaS-resolver bypass (TM-AUTHZ-008).

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    FnArg, ItemFn, PatType, Token, Type, TypeReference, parse_macro_input, punctuated::Punctuated,
};

/// AOP audit logging for service methods (EVE-226).
///
/// Emits an audit event after successful execution via fire-and-forget.
/// The service struct must expose an `audit_logger()` method returning
/// a concrete, cloneable type that implements `AuditLogger` (e.g.
/// `StorageAuditLogger`), not a `dyn AuditLogger` trait object.
///
/// # Syntax
///
/// ```rust,ignore
/// #[audit(ManagementAction::HarnessCreated, target_type = "harness")]
/// pub async fn create(&self, caller: &Caller, req: Req) -> Result<Harness> { ... }
/// ```
///
/// The macro:
/// 1. Finds `&Caller` parameter by type
/// 2. Wraps the body: on `Ok(result)`, builds and emits an audit event
/// 3. Uses `self.audit_logger()` to get the logger
/// 4. Extracts `target_id` from the result via `.audit_target_id()` if available,
///    or falls back to empty string
///
/// # Authorization
///
/// Authorization is enforced at the command layer (`Command::run`), not at
/// the service method. `#[audit]` only records successful outcomes; it does
/// not gate access. See `specs/permissions.md` for enforcement details.
#[proc_macro_attribute]
pub fn audit(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr with Punctuated::<syn::Expr, Token![,]>::parse_terminated);
    let mut func = parse_macro_input!(item as ItemFn);

    // Parse arguments: first is the action path, then key=value pairs
    let mut args_iter = args.iter();
    let action_expr = match args_iter.next() {
        Some(expr) => expr.clone(),
        None => {
            return syn::Error::new_spanned(
                &func.sig,
                "#[audit] requires an action as first argument",
            )
            .to_compile_error()
            .into();
        }
    };

    let mut target_type_str: Option<String> = None;
    for expr in args_iter {
        if let syn::Expr::Assign(assign) = expr
            && let syn::Expr::Path(path) = assign.left.as_ref()
        {
            let key = path.path.segments.last().map(|s| s.ident.to_string());
            if key.as_deref() == Some("target_type")
                && let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }) = assign.right.as_ref()
            {
                target_type_str = Some(s.value());
            }
        }
    }

    let has_target = target_type_str.is_some();
    let target_type_lit = target_type_str.unwrap_or_default();

    // Find the &Caller parameter
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
        let is_caller = tp.path.segments.last().is_some_and(|s| s.ident == "Caller");
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
                "#[audit] requires a `caller: &Caller` parameter",
            )
            .to_compile_error()
            .into();
        }
    };

    // Wrap the function body to emit audit on success
    let original_stmts = &func.block.stmts;

    let target_code = if has_target {
        quote! {
            let __target_id = __audit_result.as_ref().ok()
                .and_then(|r| everruns_core::audit::HasAuditTargetId::audit_target_id(r))
                .unwrap_or_default();
            __builder = __builder.target(#target_type_lit, __target_id);
        }
    } else {
        quote! {}
    };

    func.block = syn::parse_quote!({
        let __audit_result = (|| async {
            #(#original_stmts)*
        })().await;

        if __audit_result.is_ok() {
            let mut __builder = everruns_core::AuditEvent::management(
                #action_expr,
                #caller.org_id,
                #caller.user_id,
            );
            #target_code
            let __event = __builder.build();
            self.audit_logger().emit(__event);
        }

        __audit_result
    });

    quote!(#func).into()
}
