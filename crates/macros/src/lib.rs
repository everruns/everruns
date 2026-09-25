#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Procedural macros for the [`everruns`](https://docs.rs/everruns) agentic
//! framework.
//!
//! This crate is an implementation detail of `everruns`: the
//! [`macro@tool`] attribute is re-exported as `everruns::tool` behind the
//! default-enabled `macros` feature, and end users depend on `everruns`, not on
//! this crate. All code the macro emits is resolved through
//! `::everruns::__macro_support::…`, so no direct dependency on `serde`,
//! `schemars`, or `serde_json` is required in the calling crate.
//! It is part of the [Everruns](https://everruns.com) ecosystem.
//!
//! # Example
//!
//! Applications use the re-export from `everruns`; this implementation crate
//! exposes the same attribute symbol:
//!
//! ```
//! use everruns_macros::tool;
//!
//! let macro_name = stringify!(tool);
//! assert_eq!(macro_name, "tool");
//! ```

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{
    Attribute, Expr, ExprLit, FnArg, GenericParam, Ident, ItemFn, Lit, LitStr, Pat, PatType,
    ReturnType, Type, parse_macro_input,
};

/// Turn a typed async function into an agent tool.
///
/// `#[everruns::tool]` generates a JSON Schema for the function's arguments and
/// wraps its body in an `everruns::FunctionTool`, so a library user hands a
/// plain async function to `AgentBuilder::tool` without writing a schema or an
/// adapter by hand.
///
/// The attribute **replaces** the annotated function with a zero-argument
/// constructor of the same name that returns a
/// [`FunctionTool`](../everruns/struct.FunctionTool.html); the original body
/// runs when the model calls the tool.
///
/// ```text
/// use everruns::{Agent, Model};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Serialize)]
/// struct Weather { forecast: String }
///
/// /// Return the weather for a city.
/// #[everruns::tool]
/// async fn weather(city: String, fahrenheit: Option<bool>) -> Result<Weather, String> {
///     Ok(Weather { forecast: format!("sunny in {city}") })
/// }
///
/// let agent = Agent::builder()
///     .instructions("Answer weather questions.")
///     .model(Model::simulated("done"))
///     .tool(weather()) // the generated constructor
///     .build();
/// ```
///
/// # Options
///
/// - `#[everruns::tool(name = "…")]` overrides the tool name (default: the
///   function name).
/// - `#[everruns::tool(description = "…")]` sets the description; otherwise the
///   function's doc comment is used. A description is required.
/// - `#[tool(rename = "…")]` on a parameter renames that argument in the schema
///   and the accepted JSON.
/// - `#[everruns::tool(needs_approval)]` asks the agent's approver before every
///   call (`FunctionTool::always_needs_approval`).
/// - `#[everruns::tool(needs_approval = <rule>)]` asks only when `rule` returns
///   `true`. `rule` is a closure or function taking `&<Name>Args` (see below)
///   and returning `bool`; arguments that do not parse are treated as needing
///   approval. An agent with a gated tool and no approver fails to build.
///
/// # Generated items
///
/// Besides the constructor, the macro emits the arguments struct next to it,
/// named after the function in PascalCase with an `Args` suffix
/// (`run_sql` → `RunSqlArgs`) and the function's visibility. Its public fields
/// are the model arguments (derives `Deserialize` and `JsonSchema`), so an
/// approval rule or a host can read a call's arguments with their real types.
///
/// # Call context
///
/// A first parameter of type `ToolCallContext` (by value or `&ToolCallContext`,
/// matched by its last path segment) receives the call's context instead of a
/// model argument, and the tool is built with `FunctionTool::with_context`:
///
/// ```text
/// use everruns::ToolCallContext;
///
/// /// Export a report.
/// #[everruns::tool(needs_approval = |args: &ExportArgs| args.pages > 10)]
/// async fn export(ctx: &ToolCallContext, pages: u32) -> String {
///     ctx.progress("rendering").await;
///     format!("{pages} pages")
/// }
/// ```
///
/// # Supported signatures
///
/// Required arguments, `Option<T>` (optional), renamed arguments, a leading
/// `ToolCallContext`, a unit return, a plain `T` return, and `Result<T, E>`
/// (where `E: Display` becomes a model-visible tool error). Argument types must
/// implement `Deserialize` and `JsonSchema`; return values must implement
/// `Serialize`.
///
/// The macro rejects, with a compile error: non-`async` functions, a `self`
/// receiver, generic parameters, a missing description, non-identifier
/// parameter patterns, and a `ToolCallContext` that is not the first
/// parameter.
#[proc_macro_attribute]
pub fn tool(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as ToolArgs);
    let func = parse_macro_input!(item as ItemFn);
    match expand(args, func) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Parsed `#[everruns::tool(...)]` attribute arguments.
#[derive(Default)]
struct ToolArgs {
    name: Option<String>,
    description: Option<String>,
    approval: Option<Approval>,
}

/// The `needs_approval` option: every call, or calls a rule selects.
enum Approval {
    Always,
    When(Box<Expr>),
}

impl syn::parse::Parse for ToolArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut args = ToolArgs::default();
        let metas =
            syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated(input)?;
        for meta in metas {
            let path = meta.path().clone();
            let key = path.get_ident().map(Ident::to_string).unwrap_or_default();
            match (key.as_str(), meta) {
                ("needs_approval", syn::Meta::Path(_)) => args.approval = Some(Approval::Always),
                ("needs_approval", syn::Meta::NameValue(nv)) => {
                    args.approval = Some(Approval::When(Box::new(nv.value)));
                }
                ("name" | "description", syn::Meta::NameValue(nv)) => {
                    let value = lit_str(&nv.value).ok_or_else(|| {
                        syn::Error::new(nv.value.span(), "expected a string literal")
                    })?;
                    if key == "name" {
                        args.name = Some(value);
                    } else {
                        args.description = Some(value);
                    }
                }
                ("name" | "description", other) => {
                    return Err(syn::Error::new(
                        other.span(),
                        format!("expected `{key} = \"…\"`"),
                    ));
                }
                ("needs_approval", other) => {
                    return Err(syn::Error::new(
                        other.span(),
                        "expected `needs_approval` or `needs_approval = <rule>`",
                    ));
                }
                (other, _) => {
                    return Err(syn::Error::new(
                        path.span(),
                        format!(
                            "unknown `everruns::tool` option `{other}`; expected `name`, `description`, or `needs_approval`"
                        ),
                    ));
                }
            }
        }
        Ok(args)
    }
}

/// Extract a `String` from a string-literal expression.
fn lit_str(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) => Some(s.value()),
        _ => None,
    }
}

/// One accepted function parameter, lowered to a generated-struct field.
struct Field {
    ident: Ident,
    ty: Type,
    rename: Option<LitStr>,
}

fn expand(args: ToolArgs, func: ItemFn) -> syn::Result<TokenStream2> {
    let sig = &func.sig;

    if sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            func.sig.fn_token,
            "`#[everruns::tool]` requires an `async fn`",
        ));
    }

    if let Some(param) = sig.generics.params.first() {
        let span = match param {
            GenericParam::Type(p) => p.span(),
            GenericParam::Lifetime(p) => p.span(),
            GenericParam::Const(p) => p.span(),
        };
        return Err(syn::Error::new(
            span,
            "`#[everruns::tool]` does not support generic parameters",
        ));
    }
    if let Some(where_clause) = &sig.generics.where_clause {
        return Err(syn::Error::new_spanned(
            where_clause,
            "`#[everruns::tool]` does not support `where` clauses",
        ));
    }

    // Collect parameters, rejecting receivers and non-identifier patterns. A
    // leading `ToolCallContext` (by value or reference) is the call context,
    // not a model argument.
    let mut fields: Vec<Field> = Vec::new();
    let mut context: Option<ContextParam> = None;
    for (index, input) in sig.inputs.iter().enumerate() {
        match input {
            FnArg::Receiver(receiver) => {
                return Err(syn::Error::new_spanned(
                    receiver,
                    "`#[everruns::tool]` does not support a `self` receiver",
                ));
            }
            FnArg::Typed(pat_type) => match context_param(&pat_type.ty) {
                Some(kind) if index == 0 => context = Some(kind),
                Some(_) => {
                    return Err(syn::Error::new_spanned(
                        &pat_type.ty,
                        "`ToolCallContext` must be the first parameter of an `#[everruns::tool]` function",
                    ));
                }
                None => fields.push(lower_param(pat_type)?),
            },
        }
    }

    // Resolve the description: explicit option wins over doc comments.
    let description = match args.description {
        Some(text) => text,
        None => doc_comment(&func.attrs),
    };
    if description.trim().is_empty() {
        return Err(syn::Error::new_spanned(
            &func.sig.ident,
            "`#[everruns::tool]` requires a description: add a doc comment or \
             `#[everruns::tool(description = \"…\")]`",
        ));
    }
    let description = description.trim().to_string();

    let fn_ident = &sig.ident;
    let vis = &func.vis;
    let tool_name = args.name.unwrap_or_else(|| fn_ident.to_string());

    // Inner async fn carrying the original body and signature. The parameters
    // are re-emitted with any `#[tool(...)]` helper attribute stripped — it is
    // meaningful only to this macro and would not resolve in the output.
    let impl_ident = format_ident!("__everruns_tool_impl_{}", fn_ident);
    let inner_inputs = sig.inputs.iter().map(|input| match input {
        FnArg::Typed(pat_type) => {
            let mut cleaned = pat_type.clone();
            cleaned.attrs.retain(|attr| !attr.path().is_ident("tool"));
            FnArg::Typed(cleaned)
        }
        other => other.clone(),
    });
    let inner_inputs = quote! { #(#inner_inputs),* };
    let orig_output = &sig.output;
    let orig_body = &func.block;

    // Generated arguments struct, nameable so hosts and approval rules can
    // read typed arguments: `run_sql` -> `RunSqlArgs`. The `Args` suffix keeps
    // it clear of the common `fn weather() -> Weather` naming. No doc comment:
    // schemars would copy it into the model-facing schema.
    let args_ident = format_ident!("{}Args", pascal_case(&fn_ident.to_string()));
    let field_decls = fields.iter().map(|f| {
        let ident = &f.ident;
        let ty = &f.ty;
        let rename = f
            .rename
            .as_ref()
            .map(|name| quote!(#[serde(rename = #name)]));
        quote! { #rename pub #ident: #ty }
    });
    let field_idents: Vec<&Ident> = fields.iter().map(|f| &f.ident).collect();

    let dispatch = result_dispatch(orig_output, &tool_name);

    let (constructor, handler_params, context_arg) = match context {
        None => (
            quote!(::everruns::FunctionTool::new),
            quote!(__args: ::everruns::__macro_support::Value),
            quote!(),
        ),
        Some(kind) => {
            let pass = match kind {
                ContextParam::Value => quote!(__ctx,),
                ContextParam::Ref => quote!(&__ctx,),
            };
            (
                quote!(::everruns::FunctionTool::with_context),
                quote!(
                    __ctx: ::everruns::ToolCallContext,
                    __args: ::everruns::__macro_support::Value
                ),
                pass,
            )
        }
    };

    let approval = match args.approval {
        None => quote!(),
        Some(Approval::Always) => quote!(.always_needs_approval()),
        Some(Approval::When(rule)) => quote! {
            .needs_approval({
                // Pin the rule's argument type so a bare closure infers it.
                fn __everruns_approval_rule<
                    __F: ::core::ops::Fn(&#args_ident) -> bool
                        + ::core::marker::Send
                        + ::core::marker::Sync
                        + 'static,
                >(rule: __F) -> __F {
                    rule
                }
                let __rule = __everruns_approval_rule(#rule);
                move |__args: &::everruns::__macro_support::Value| {
                    match ::everruns::__macro_support::from_value::<#args_ident>(
                        ::core::clone::Clone::clone(__args),
                    ) {
                        ::core::result::Result::Ok(__parsed) => __rule(&__parsed),
                        // Fail closed: arguments the rule cannot read are
                        // asked about rather than waved through.
                        ::core::result::Result::Err(_) => true,
                    }
                }
            })
        },
    };

    let expanded = quote! {
        #[derive(
            ::everruns::__macro_support::serde::Deserialize,
            ::everruns::__macro_support::schemars::JsonSchema,
        )]
        #[serde(crate = "::everruns::__macro_support::serde")]
        #[schemars(crate = "::everruns::__macro_support::schemars", rename = "__Args")]
        #[allow(missing_docs, non_camel_case_types, dead_code)]
        #vis struct #args_ident {
            #(#field_decls,)*
        }

        // `let __out = …await` binds `()` for unit-returning tools, which the
        // `let_unit_value` lint flags; the binding is intentional here.
        #[allow(clippy::let_unit_value)]
        #vis fn #fn_ident() -> ::everruns::FunctionTool {
            #[allow(clippy::used_underscore_items)]
            async fn #impl_ident(#inner_inputs) #orig_output #orig_body

            let __schema = ::everruns::__macro_support::schema_for::<#args_ident>();

            #constructor(
                #tool_name,
                #description,
                __schema,
                move |#handler_params| async move {
                    let __parsed: #args_ident = match ::everruns::__macro_support::from_value(__args) {
                        ::core::result::Result::Ok(__v) => __v,
                        ::core::result::Result::Err(__e) => {
                            // Model-visible on purpose, unlike the `Err` channel
                            // below: the model wrote these arguments, so naming
                            // the bad field is what lets it fix its own call.
                            // It describes the call, never the host.
                            return ::core::result::Result::Ok(
                                ::everruns::ToolResponse::error(::std::format!(
                                    "invalid arguments for tool `{}`: {}",
                                    #tool_name,
                                    __e,
                                )),
                            );
                        }
                    };
                    let __out = #impl_ident(#context_arg #(__parsed.#field_idents),*).await;
                    #dispatch
                },
            )
            #approval
        }
    };

    Ok(expanded)
}

/// How the function takes its call context.
#[derive(Clone, Copy)]
enum ContextParam {
    Value,
    Ref,
}

/// Whether `ty` is `ToolCallContext` or `&ToolCallContext`, matched by the
/// last path segment so any import path works.
fn context_param(ty: &Type) -> Option<ContextParam> {
    let (ty, kind) = match ty {
        Type::Reference(reference) if reference.mutability.is_none() => {
            (reference.elem.as_ref(), ContextParam::Ref)
        }
        other => (other, ContextParam::Value),
    };
    let Type::Path(type_path) = ty else {
        return None;
    };
    type_path
        .path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "ToolCallContext" && segment.arguments.is_empty())
        .then_some(kind)
}

/// `run_sql` -> `RunSql`; a raw identifier's `r#` prefix is dropped.
fn pascal_case(name: &str) -> String {
    name.trim_start_matches("r#")
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

/// Lower one typed parameter to a generated-struct field, rejecting patterns the
/// adapter cannot express and extracting a `#[tool(rename = "…")]` override.
fn lower_param(pat_type: &PatType) -> syn::Result<Field> {
    let ident = match pat_type.pat.as_ref() {
        Pat::Ident(pat_ident) => pat_ident.ident.clone(),
        other => {
            return Err(syn::Error::new_spanned(
                other,
                "`#[everruns::tool]` parameters must be plain identifiers (`name: Type`)",
            ));
        }
    };
    let rename = parse_rename(&pat_type.attrs)?;
    Ok(Field {
        ident,
        ty: (*pat_type.ty).clone(),
        rename,
    })
}

/// Parse an optional `#[tool(rename = "…")]` attribute on a parameter.
fn parse_rename(attrs: &[Attribute]) -> syn::Result<Option<LitStr>> {
    let mut rename = None;
    for attr in attrs {
        if !attr.path().is_ident("tool") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                let value = meta.value()?;
                rename = Some(value.parse::<LitStr>()?);
                Ok(())
            } else {
                Err(meta.error("expected `rename = \"…\"`"))
            }
        })?;
    }
    Ok(rename)
}

/// Join `#[doc = "…"]` lines into a single trimmed description.
fn doc_comment(attrs: &[Attribute]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        if let syn::Meta::NameValue(nv) = &attr.meta
            && let Some(text) = lit_str(&nv.value)
        {
            lines.push(text.trim().to_string());
        }
    }
    lines.join("\n").trim().to_string()
}

/// Produce the tokens that turn the inner call's output into a
/// `Result<ToolResponse, String>` for `FunctionTool::new`.
///
/// `Result<T, E>` returns map `Err(E: Display)` to a model-visible tool error;
/// any other return type (including `()`) is serialized as a success.
fn result_dispatch(output: &ReturnType, tool_name: &str) -> TokenStream2 {
    let serialize = |value: TokenStream2| {
        quote! {
            ::everruns::__macro_support::to_value(&#value)
                .map(::everruns::ToolResponse::json)
                .map_err(|__e| ::std::format!(
                    "failed to serialize result of tool `{}`: {}", #tool_name, __e,
                ))
        }
    };
    if returns_result(output) {
        let serialize_ok = serialize(quote!(__ok));
        quote! {
            match __out {
                ::core::result::Result::Ok(__ok) => { #serialize_ok }
                ::core::result::Result::Err(__err) => ::core::result::Result::Ok(
                    ::everruns::ToolResponse::error(::std::string::ToString::to_string(&__err)),
                ),
            }
        }
    } else {
        let serialize_out = serialize(quote!(__out));
        quote_spanned! {output.span()=> #serialize_out }
    }
}

/// Whether a return type is syntactically `Result<…>`.
fn returns_result(output: &ReturnType) -> bool {
    let ReturnType::Type(_, ty) = output else {
        return false;
    };
    let Type::Path(type_path) = ty.as_ref() else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "Result")
}
