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
///
/// # Supported signatures
///
/// Required arguments, `Option<T>` (optional), renamed arguments, a unit
/// return, a plain `T` return, and `Result<T, E>` (where `E: Display` becomes a
/// model-visible tool error). Argument types must implement `Deserialize` and
/// `JsonSchema`; return values must implement `Serialize`.
///
/// The macro rejects, with a compile error: non-`async` functions, a `self`
/// receiver, generic parameters, a missing description, and non-identifier
/// parameter patterns.
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
}

impl syn::parse::Parse for ToolArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut args = ToolArgs::default();
        let metas =
            syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated(
                input,
            )?;
        for meta in metas {
            let key = meta
                .path
                .get_ident()
                .map(Ident::to_string)
                .unwrap_or_default();
            let value = lit_str(&meta.value)
                .ok_or_else(|| syn::Error::new(meta.value.span(), "expected a string literal"))?;
            match key.as_str() {
                "name" => args.name = Some(value),
                "description" => args.description = Some(value),
                other => {
                    return Err(syn::Error::new(
                        meta.path.span(),
                        format!(
                            "unknown `everruns::tool` option `{other}`; expected `name` or `description`"
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

    // Collect parameters, rejecting receivers and non-identifier patterns.
    let mut fields: Vec<Field> = Vec::new();
    for input in &sig.inputs {
        match input {
            FnArg::Receiver(receiver) => {
                return Err(syn::Error::new_spanned(
                    receiver,
                    "`#[everruns::tool]` does not support a `self` receiver",
                ));
            }
            FnArg::Typed(pat_type) => fields.push(lower_param(pat_type)?),
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

    // Generated arguments struct.
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

    let expanded = quote! {
        // `let __out = …await` binds `()` for unit-returning tools, which the
        // `let_unit_value` lint flags; the binding is intentional here.
        #[allow(clippy::let_unit_value)]
        #vis fn #fn_ident() -> ::everruns::FunctionTool {
            #[derive(
                ::everruns::__macro_support::serde::Deserialize,
                ::everruns::__macro_support::schemars::JsonSchema,
            )]
            #[serde(crate = "::everruns::__macro_support::serde")]
            #[schemars(crate = "::everruns::__macro_support::schemars")]
            #[allow(non_camel_case_types, dead_code)]
            struct __Args {
                #(#field_decls,)*
            }

            #[allow(clippy::used_underscore_items)]
            async fn #impl_ident(#inner_inputs) #orig_output #orig_body

            let __schema = ::everruns::__macro_support::schema_for::<__Args>();

            ::everruns::FunctionTool::new(
                #tool_name,
                #description,
                __schema,
                move |__args: ::everruns::__macro_support::Value| async move {
                    let __parsed: __Args = match ::everruns::__macro_support::from_value(__args) {
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
                    let __out = #impl_ident(#(__parsed.#field_idents),*).await;
                    #dispatch
                },
            )
        }
    };

    Ok(expanded)
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
