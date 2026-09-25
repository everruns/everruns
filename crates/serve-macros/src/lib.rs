//! Attribute macros for [`serve`](https://docs.rs/everruns-serve), the
//! experimental agent framework in the [Everruns](https://everruns.com)
//! ecosystem.
//!
//! Each attribute keeps the annotated item callable exactly as written and adds
//! a link-time registration (via `inventory`) that `App::builder().discover()`
//! collects. Nothing here scans the filesystem: the file layout is a
//! convention, and the source path of every registration (`file!()`) is carried
//! into the manifest so the host and the dev UI can point back at it.
//!
//! Applications use the re-exports from `serve`, never this crate directly.
//!
//! ```
//! use serve_macros::{agent, tool};
//!
//! assert_eq!(stringify!(agent), "agent");
//! assert_eq!(stringify!(tool), "tool");
//! ```

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Expr, ExprLit, FnArg, ItemFn, Lit, LitStr, Meta, Pat, PatType, ReturnType, Token,
    Type, parse_macro_input,
};

/// Register a function returning `serve::Agent` as an agent of this app.
///
/// ```text
/// #[agent]                 // name = function name
/// #[agent(default)]        // served on /v1/sessions when the app has several agents
/// #[agent(sub)]            // a subagent: exposed to the other agents as `ask_<name>`
/// #[agent(name = "triage")]
/// ```
#[proc_macro_attribute]
pub fn agent(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as Args);
    let func = parse_macro_input!(item as ItemFn);
    finish(expand_agent(args, func))
}

/// Register an async function as a tool. The function name is the tool name,
/// the doc comment is its description, and the remaining parameters become the
/// tool's JSON arguments. An optional first parameter `cx: &Cx` receives the
/// per-session context.
///
/// The macro also generates a `pub struct` named after the function in
/// PascalCase (`run_sql` → `RunSql`) holding the arguments, so an approval
/// predicate can inspect them:
///
/// ```text
/// #[tool(needs_approval = |a: &RunSql| a.sql.contains("DELETE"))]
/// async fn run_sql(cx: &Cx, sql: String) -> Result<Rows> { … }
/// ```
///
/// `#[tool(needs_approval)]` always asks. `name = "…"` and `description = "…"`
/// override the defaults.
#[proc_macro_attribute]
pub fn tool(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as Args);
    let func = parse_macro_input!(item as ItemFn);
    finish(expand_tool(args, func))
}

/// Register a function returning a `serve::Channel` implementation. Its
/// inbound webhook is served at `POST /v1/channels/{name}`. A companion module
/// with the same name is generated so a schedule can address the channel:
/// `slack::channel("C0123ABC")`.
#[proc_macro_attribute]
pub fn channel(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as Args);
    let func = parse_macro_input!(item as ItemFn);
    finish(expand_channel(args, func))
}

/// Register an async function `(cx: &Cx) -> Result` to run on a cron schedule.
/// Five-field cron (`"0 9 * * MON"`) is accepted; a six- or seven-field
/// expression with seconds is passed through.
#[proc_macro_attribute]
pub fn schedule(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as Args);
    let func = parse_macro_input!(item as ItemFn);
    finish(expand_schedule(args, func))
}

/// Register a function whose return value is a connection. Tools read it with
/// `cx.connection::<T>()`; a `serve::McpServer` is also attached to every
/// agent as an MCP server. Credentials stay in the connection, never in the
/// model context. Returning `Result<T>` makes a failure a discovery error.
#[proc_macro_attribute]
pub fn connection(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as Args);
    let func = parse_macro_input!(item as ItemFn);
    finish(expand_connection(args, func))
}

/// Register an async function `(t: &mut EvalCx) -> Result` as an eval. Evals
/// run in-process with `cargo run -- eval`, or against a deployed URL with
/// `cargo run -- eval --against <url>`.
#[proc_macro_attribute]
pub fn eval(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as Args);
    let func = parse_macro_input!(item as ItemFn);
    finish(expand_eval(args, func))
}

fn finish(result: syn::Result<TokenStream2>) -> TokenStream {
    match result {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

// --- Attribute arguments ----------------------------------------------------

/// Parsed attribute arguments shared by every macro. Each macro accepts only
/// the keys that mean something to it and rejects the rest.
#[derive(Default)]
struct Args {
    /// A bare leading string literal, e.g. the cron in `#[schedule("…")]`.
    positional: Option<LitStr>,
    /// `key` flags such as `sub`, `default`, `needs_approval`.
    flags: Vec<syn::Ident>,
    /// `key = value` pairs.
    pairs: Vec<(syn::Ident, Expr)>,
}

impl syn::parse::Parse for Args {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut args = Args::default();
        if input.peek(LitStr) {
            args.positional = Some(input.parse()?);
            if input.is_empty() {
                return Ok(args);
            }
            input.parse::<Token![,]>()?;
        }
        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        for meta in metas {
            match meta {
                Meta::Path(path) => {
                    let ident = path
                        .get_ident()
                        .cloned()
                        .ok_or_else(|| syn::Error::new(path.span(), "expected an identifier"))?;
                    args.flags.push(ident);
                }
                Meta::NameValue(nv) => {
                    let ident =
                        nv.path.get_ident().cloned().ok_or_else(|| {
                            syn::Error::new(nv.path.span(), "expected an identifier")
                        })?;
                    args.pairs.push((ident, nv.value));
                }
                Meta::List(list) => {
                    return Err(syn::Error::new(list.span(), "unexpected nested arguments"));
                }
            }
        }
        Ok(args)
    }
}

impl Args {
    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|flag| flag == name)
    }

    fn string(&self, name: &str) -> syn::Result<Option<String>> {
        for (key, value) in &self.pairs {
            if key == name {
                return lit_str(value)
                    .map(Some)
                    .ok_or_else(|| syn::Error::new(value.span(), "expected a string literal"));
            }
        }
        Ok(None)
    }

    fn expr(&self, name: &str) -> Option<&Expr> {
        self.pairs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    /// Reject any key this macro does not understand, naming the ones it does.
    fn only(&self, macro_name: &str, allowed: &[&str]) -> syn::Result<()> {
        let unknown = self
            .flags
            .iter()
            .chain(self.pairs.iter().map(|(key, _)| key))
            .find(|key| !allowed.iter().any(|name| *key == name));
        match unknown {
            Some(key) => Err(syn::Error::new(
                key.span(),
                format!(
                    "unknown `#[{macro_name}]` option `{key}`; expected one of: {}",
                    allowed.join(", ")
                ),
            )),
            None => Ok(()),
        }
    }
}

fn lit_str(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) => Some(s.value()),
        _ => None,
    }
}

fn doc_comment(attrs: &[Attribute]) -> String {
    let mut lines = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        if let Meta::NameValue(nv) = &attr.meta
            && let Some(text) = lit_str(&nv.value)
        {
            lines.push(text.trim().to_string());
        }
    }
    lines.join("\n").trim().to_string()
}

fn require_sync_nullary(func: &ItemFn, macro_name: &str) -> syn::Result<()> {
    if func.sig.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            func.sig.asyncness,
            format!("`#[{macro_name}]` expects a plain `fn`, not `async fn`"),
        ));
    }
    if !func.sig.inputs.is_empty() {
        return Err(syn::Error::new_spanned(
            &func.sig.inputs,
            format!("`#[{macro_name}]` functions take no arguments"),
        ));
    }
    if !func.sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &func.sig.generics,
            format!("`#[{macro_name}]` does not support generics"),
        ));
    }
    Ok(())
}

fn require_async(func: &ItemFn, macro_name: &str) -> syn::Result<()> {
    if func.sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            func.sig.fn_token,
            format!("`#[{macro_name}]` requires an `async fn`"),
        ));
    }
    if !func.sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &func.sig.generics,
            format!("`#[{macro_name}]` does not support generics"),
        ));
    }
    Ok(())
}

// --- #[agent] -----------------------------------------------------------------

fn expand_agent(args: Args, func: ItemFn) -> syn::Result<TokenStream2> {
    args.only("agent", &["sub", "default", "name"])?;
    require_sync_nullary(&func, "agent")?;
    let ident = &func.sig.ident;
    let name = args.string("name")?.unwrap_or_else(|| ident.to_string());
    let sub = args.flag("sub");
    let default = args.flag("default");
    let doc = doc_comment(&func.attrs);
    Ok(quote! {
        #func

        ::serve::__private::inventory::submit! {
            ::serve::__private::AgentRegistration {
                name: #name,
                doc: #doc,
                sub: #sub,
                default: #default,
                source: ::core::file!(),
                build: #ident,
            }
        }
    })
}

// --- #[tool] ------------------------------------------------------------------

fn expand_tool(args: Args, mut func: ItemFn) -> syn::Result<TokenStream2> {
    args.only("tool", &["name", "description", "needs_approval"])?;
    require_async(&func, "tool")?;

    let fn_ident = func.sig.ident.clone();
    let vis = func.vis.clone();
    let tool_name = args.string("name")?.unwrap_or_else(|| fn_ident.to_string());
    let description = match args.string("description")? {
        Some(text) => text,
        None => doc_comment(&func.attrs),
    };
    if description.is_empty() {
        return Err(syn::Error::new_spanned(
            &fn_ident,
            "`#[tool]` needs a description: add a `///` doc comment or `description = \"…\"`",
        ));
    }

    // Split the optional leading `&Cx` from the model-visible arguments.
    let mut takes_cx = false;
    let mut fields = Vec::new();
    for (index, input) in func.sig.inputs.iter_mut().enumerate() {
        let FnArg::Typed(pat_type) = input else {
            return Err(syn::Error::new_spanned(
                input,
                "`#[tool]` does not support a `self` receiver",
            ));
        };
        if index == 0 && is_cx_ref(&pat_type.ty) {
            takes_cx = true;
            continue;
        }
        fields.push(lower_param(pat_type)?);
        // `#[doc]` on a parameter documents the schema field; the fn keeps none.
        pat_type.attrs.retain(|attr| !attr.path().is_ident("doc"));
    }

    let args_ident = format_ident!("{}", pascal_case(&fn_ident.to_string()));
    let field_decls = fields.iter().map(|field| {
        let ident = &field.ident;
        let ty = &field.ty;
        let docs = &field.docs;
        let default = if is_option(ty) {
            quote!(#[serde(default)])
        } else {
            quote!()
        };
        quote! { #(#docs)* #default pub #ident: #ty }
    });
    let field_idents: Vec<_> = fields.iter().map(|field| &field.ident).collect();
    let call = if takes_cx {
        quote!(#fn_ident(&__cx, #(__args.#field_idents),*))
    } else {
        quote!(#fn_ident(#(__args.#field_idents),*))
    };
    let output = if returns_result(&func.sig.output) {
        quote!(::serve::__private::tool_result(__out))
    } else {
        quote!(::serve::__private::tool_value(&__out))
    };

    let approval = match (args.flag("needs_approval"), args.expr("needs_approval")) {
        (true, _) => quote!(::serve::__private::Approval::Always),
        (false, Some(predicate)) => quote! {
            ::serve::__private::Approval::When({
                fn __serve_needs_approval(__value: &::serve::__private::Value) -> bool {
                    match ::serve::__private::parse::<#args_ident>(__value.clone()) {
                        ::core::result::Result::Ok(__args) => {
                            let __predicate = #predicate;
                            __predicate(&__args)
                        }
                        // Arguments that do not parse never reach the tool body.
                        ::core::result::Result::Err(_) => false,
                    }
                }
                __serve_needs_approval
            })
        },
        (false, None) => quote!(::serve::__private::Approval::Never),
    };

    Ok(quote! {
        #func

        // No doc comment: schemars would copy it into the model-facing schema.
        #[derive(
            ::core::fmt::Debug,
            ::core::clone::Clone,
            ::serve::__private::serde::Deserialize,
            ::serve::__private::serde::Serialize,
            ::serve::__private::schemars::JsonSchema,
        )]
        #[serde(crate = "::serve::__private::serde")]
        #[schemars(crate = "::serve::__private::schemars")]
        #vis struct #args_ident {
            #(#field_decls,)*
        }

        const _: () = {
            fn __serve_schema() -> ::serve::__private::Value {
                ::serve::__private::schema_for::<#args_ident>()
            }

            #[allow(clippy::let_unit_value, unused_variables)]
            fn __serve_call(
                __cx: ::serve::Cx,
                __value: ::serve::__private::Value,
            ) -> ::serve::__private::ToolFuture {
                ::std::boxed::Box::pin(async move {
                    let __args: #args_ident = match ::serve::__private::parse(__value) {
                        ::core::result::Result::Ok(__args) => __args,
                        ::core::result::Result::Err(__err) => {
                            return ::serve::__private::invalid_arguments(#tool_name, __err);
                        }
                    };
                    let __out = #call.await;
                    #output
                })
            }

            ::serve::__private::inventory::submit! {
                ::serve::__private::ToolRegistration {
                    name: #tool_name,
                    description: #description,
                    source: ::core::file!(),
                    schema: __serve_schema,
                    approval: #approval,
                    call: __serve_call,
                }
            }
        };
    })
}

struct Field {
    ident: syn::Ident,
    ty: Type,
    docs: Vec<Attribute>,
}

fn lower_param(pat_type: &PatType) -> syn::Result<Field> {
    let Pat::Ident(pat_ident) = pat_type.pat.as_ref() else {
        return Err(syn::Error::new_spanned(
            &pat_type.pat,
            "`#[tool]` parameters must be plain identifiers (`name: Type`)",
        ));
    };
    Ok(Field {
        ident: pat_ident.ident.clone(),
        ty: (*pat_type.ty).clone(),
        docs: pat_type
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("doc"))
            .cloned()
            .collect(),
    })
}

fn is_cx_ref(ty: &Type) -> bool {
    let Type::Reference(reference) = ty else {
        return false;
    };
    let Type::Path(path) = reference.elem.as_ref() else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "Cx")
}

fn is_option(ty: &Type) -> bool {
    last_segment_is(ty, "Option")
}

fn returns_result(output: &ReturnType) -> bool {
    match output {
        ReturnType::Type(_, ty) => last_segment_is(ty, "Result"),
        ReturnType::Default => false,
    }
}

fn last_segment_is(ty: &Type, name: &str) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == name)
}

fn pascal_case(snake: &str) -> String {
    snake
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

// --- #[channel] ---------------------------------------------------------------

fn expand_channel(args: Args, func: ItemFn) -> syn::Result<TokenStream2> {
    args.only("channel", &["name"])?;
    require_sync_nullary(&func, "channel")?;
    let ident = &func.sig.ident;
    let vis = &func.vis;
    let name = args.string("name")?.unwrap_or_else(|| ident.to_string());
    Ok(quote! {
        #func

        /// Delivery targets on this channel (generated by `#[channel]`).
        #[allow(dead_code)]
        #vis mod #ident {
            /// Address a conversation on this channel, e.g. a Slack channel id.
            pub fn channel(target: impl ::core::convert::Into<::std::string::String>) -> ::serve::DeliveryTarget {
                ::serve::DeliveryTarget::new(#name, target)
            }
        }

        const _: () = {
            fn __serve_build() -> ::std::boxed::Box<dyn ::serve::Channel> {
                ::std::boxed::Box::new(#ident())
            }

            ::serve::__private::inventory::submit! {
                ::serve::__private::ChannelRegistration {
                    name: #name,
                    source: ::core::file!(),
                    build: __serve_build,
                }
            }
        };
    })
}

// --- #[schedule] --------------------------------------------------------------

fn expand_schedule(args: Args, func: ItemFn) -> syn::Result<TokenStream2> {
    args.only("schedule", &["name", "cron"])?;
    require_async(&func, "schedule")?;
    let ident = &func.sig.ident;
    let name = args.string("name")?.unwrap_or_else(|| ident.to_string());
    let cron = match (&args.positional, args.string("cron")?) {
        (Some(lit), _) => lit.value(),
        (None, Some(cron)) => cron,
        (None, None) => {
            return Err(syn::Error::new_spanned(
                ident,
                "`#[schedule]` needs a cron expression: `#[schedule(\"0 9 * * MON\")]`",
            ));
        }
    };
    if cron.split_whitespace().count() < 5 {
        return Err(syn::Error::new_spanned(
            ident,
            format!("`#[schedule]` cron `{cron}` must have at least five fields"),
        ));
    }
    Ok(quote! {
        #func

        const _: () = {
            fn __serve_run(__cx: ::serve::Cx) -> ::serve::__private::BoxFuture<'static, ::serve::Result> {
                ::std::boxed::Box::pin(async move { #ident(&__cx).await })
            }

            ::serve::__private::inventory::submit! {
                ::serve::__private::ScheduleRegistration {
                    name: #name,
                    cron: #cron,
                    source: ::core::file!(),
                    run: __serve_run,
                }
            }
        };
    })
}

// --- #[connection] ------------------------------------------------------------

fn expand_connection(args: Args, func: ItemFn) -> syn::Result<TokenStream2> {
    args.only("connection", &["name"])?;
    require_sync_nullary(&func, "connection")?;
    let ident = &func.sig.ident;
    let name = args.string("name")?.unwrap_or_else(|| ident.to_string());
    // `-> Result<T>` connections may fail at boot; discovery reports it.
    let value = if returns_result(&func.sig.output) {
        quote!(::serve::__private::ConnectionValue::new(#name, #ident()?))
    } else {
        quote!(::serve::__private::ConnectionValue::new(#name, #ident()))
    };
    Ok(quote! {
        #func

        const _: () = {
            fn __serve_build() -> ::serve::Result<::serve::__private::ConnectionValue> {
                ::core::result::Result::Ok(#value)
            }

            ::serve::__private::inventory::submit! {
                ::serve::__private::ConnectionRegistration {
                    name: #name,
                    source: ::core::file!(),
                    build: __serve_build,
                }
            }
        };
    })
}

// --- #[eval] ------------------------------------------------------------------

fn expand_eval(args: Args, func: ItemFn) -> syn::Result<TokenStream2> {
    args.only("eval", &["name"])?;
    require_async(&func, "eval")?;
    let ident = &func.sig.ident;
    let name = args.string("name")?.unwrap_or_else(|| ident.to_string());
    let doc = doc_comment(&func.attrs);
    Ok(quote! {
        #func

        const _: () = {
            fn __serve_run<'a>(
                __t: &'a mut ::serve::EvalCx,
            ) -> ::serve::__private::BoxFuture<'a, ::serve::Result> {
                ::std::boxed::Box::pin(#ident(__t))
            }

            ::serve::__private::inventory::submit! {
                ::serve::__private::EvalRegistration {
                    name: #name,
                    doc: #doc,
                    source: ::core::file!(),
                    run: __serve_run,
                }
            }
        };
    })
}

#[cfg(test)]
mod tests {
    use super::pascal_case;

    #[test]
    fn pascal_case_joins_snake_segments() {
        assert_eq!(pascal_case("run_sql"), "RunSql");
        assert_eq!(pascal_case("weather"), "Weather");
        assert_eq!(pascal_case("__odd__name"), "OddName");
    }
}
