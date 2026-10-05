//! `#[command(...)]`: one declaration for a domain command.
//!
//! Applied to an `impl Command for X` block. The block keeps `type Output`,
//! `execute`, and any hand-written overrides (`param_schema`,
//! `output_schema`, ...). The attribute supplies the rest:
//!
//! - `meta()`, `policy()`, `positional_arg()`, `read_only()`, `cli()`;
//! - the `inventory` registration that feeds MCP, gRPC, the CLI contract and
//!   the policy-coverage tests;
//! - with `http = <mode>`: an `HttpCommand` impl (the server's generic axum
//!   handler serves it at `path`) and a `#[utoipa::path]` operation that is
//!   added to the OpenAPI document at runtime.
//!
//! ```ignore
//! #[command(
//!     name = "get_skill",
//!     category = "skills",
//!     description = "Get a single skill by ID.",
//!     method = "GET",
//!     path = "/v1/skills/{id}",
//!     policy = SKILL_VIEW,
//!     positional = "id",
//!     http = with_urls,
//!     responses((status = 404, description = "Skill not found")),
//! )]
//! impl Command for GetSkill {
//!     type Output = Skill;
//!     async fn execute(self, ctx: &Ctx) -> Result<Skill, CommandError> { ... }
//! }
//! ```
//!
//! Decision: path parameters are named after the command's fields
//! (`{id}`), so the HTTP path, the catalog path and the command input agree
//! and the generic handler can merge path, query and body into one params
//! object without a rename table.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Expr, ExprLit, GenericArgument, ImplItem, ItemImpl, Lit, LitStr, Meta, PathArguments, Token,
    Type, parse_macro_input,
};

#[proc_macro_attribute]
pub fn command(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as CommandArgs);
    let item = parse_macro_input!(item as ItemImpl);
    expand(args, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// How the generated HTTP handler shapes a successful response.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HttpMode {
    Plain,
    WithUrls,
    Created,
    CreatedWithUrls,
    NoContent,
    List,
    ListWithUrls,
    PaginatedWithUrls,
}

impl HttpMode {
    fn parse(ident: &syn::Ident) -> syn::Result<Self> {
        Ok(match ident.to_string().as_str() {
            "plain" => Self::Plain,
            "with_urls" => Self::WithUrls,
            "created" => Self::Created,
            "created_with_urls" => Self::CreatedWithUrls,
            "no_content" => Self::NoContent,
            "list" => Self::List,
            "list_with_urls" => Self::ListWithUrls,
            "paginated_with_urls" => Self::PaginatedWithUrls,
            other => {
                return Err(syn::Error::new(
                    ident.span(),
                    format!(
                        "unknown http mode `{other}`; expected one of plain, with_urls, created, \
                         created_with_urls, no_content, list, list_with_urls, paginated_with_urls"
                    ),
                ));
            }
        })
    }

    fn respond_type(self) -> TokenStream2 {
        let name = match self {
            Self::Plain => "Plain",
            Self::WithUrls => "WithUrls",
            Self::Created => "Created",
            Self::CreatedWithUrls => "CreatedWithUrls",
            Self::NoContent => "NoContent",
            Self::List => "List",
            Self::ListWithUrls => "ListWithUrls",
            Self::PaginatedWithUrls => "PaginatedWithUrls",
        };
        let ident = syn::Ident::new(name, Span::call_site());
        quote!(crate::api::command_http::respond::#ident)
    }
}

#[derive(Default)]
struct CommandArgs {
    name: Option<LitStr>,
    category: Option<LitStr>,
    description: Option<LitStr>,
    method: Option<LitStr>,
    path: Option<LitStr>,
    tag: Option<LitStr>,
    policy: Option<Expr>,
    positional: Option<LitStr>,
    read_only: Option<Expr>,
    cli: Option<Expr>,
    http: Option<(HttpMode, Span)>,
    params: Option<TokenStream2>,
    request_body: Option<TokenStream2>,
    responses: Option<TokenStream2>,
}

fn lit_str(value: &Expr) -> syn::Result<LitStr> {
    match value {
        Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) => Ok(s.clone()),
        other => Err(syn::Error::new(other.span(), "expected a string literal")),
    }
}

impl Parse for CommandArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = CommandArgs::default();
        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        for meta in metas {
            let key = meta
                .path()
                .get_ident()
                .map(ToString::to_string)
                .unwrap_or_default();
            match (key.as_str(), &meta) {
                ("name", Meta::NameValue(nv)) => args.name = Some(lit_str(&nv.value)?),
                ("category", Meta::NameValue(nv)) => args.category = Some(lit_str(&nv.value)?),
                ("description", Meta::NameValue(nv)) => {
                    args.description = Some(lit_str(&nv.value)?)
                }
                ("method", Meta::NameValue(nv)) => {
                    let method = lit_str(&nv.value)?;
                    if !matches!(
                        method.value().as_str(),
                        "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
                    ) {
                        return Err(syn::Error::new(
                            method.span(),
                            "method must be GET, POST, PUT, PATCH or DELETE",
                        ));
                    }
                    args.method = Some(method);
                }
                ("path", Meta::NameValue(nv)) => args.path = Some(lit_str(&nv.value)?),
                ("tag", Meta::NameValue(nv)) => args.tag = Some(lit_str(&nv.value)?),
                ("positional", Meta::NameValue(nv)) => args.positional = Some(lit_str(&nv.value)?),
                ("policy", Meta::NameValue(nv)) => args.policy = Some(nv.value.clone()),
                ("read_only", Meta::NameValue(nv)) => args.read_only = Some(nv.value.clone()),
                ("cli", Meta::NameValue(nv)) => args.cli = Some(nv.value.clone()),
                ("http", Meta::NameValue(nv)) => {
                    let Expr::Path(path) = &nv.value else {
                        return Err(syn::Error::new(nv.value.span(), "expected an http mode"));
                    };
                    let ident = path
                        .path
                        .get_ident()
                        .ok_or_else(|| syn::Error::new(path.span(), "expected an http mode"))?;
                    args.http = Some((HttpMode::parse(ident)?, ident.span()));
                }
                ("params", Meta::List(list)) => args.params = Some(list.tokens.clone()),
                ("request_body", Meta::List(list)) => args.request_body = Some(list.tokens.clone()),
                ("responses", Meta::List(list)) => args.responses = Some(list.tokens.clone()),
                _ => {
                    return Err(syn::Error::new(
                        meta.span(),
                        "unknown #[command] argument; expected name, category, description, \
                         method, path, tag, policy, positional, read_only, cli, http, \
                         params(..), request_body(..) or responses(..)",
                    ));
                }
            }
        }
        Ok(args)
    }
}

fn required(value: Option<LitStr>, key: &str) -> syn::Result<LitStr> {
    value.ok_or_else(|| {
        syn::Error::new(
            Span::call_site(),
            format!("#[command] requires `{key} = \"...\"`"),
        )
    })
}

/// `{name}` placeholders of a route template, in order.
fn path_params(path: &str) -> Vec<String> {
    path.split('/')
        .filter_map(|segment| {
            segment
                .strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))
                .map(str::to_string)
        })
        .collect()
}

/// `T` from `Vec<T>` / `Paginated<T>` (the last path segment's single generic).
fn element_type(output: &Type, wrapper: &str) -> syn::Result<Type> {
    if let Type::Path(path) = output
        && let Some(last) = path.path.segments.last()
        && last.ident == wrapper
        && let PathArguments::AngleBracketed(args) = &last.arguments
        && let Some(GenericArgument::Type(inner)) = args.args.first()
    {
        return Ok(inner.clone());
    }
    Err(syn::Error::new(
        output.span(),
        format!("this http mode needs `type Output = {wrapper}<T>`"),
    ))
}

fn expand(args: CommandArgs, mut item: ItemImpl) -> syn::Result<TokenStream2> {
    let Some((trait_path, _)) = &item.trait_ else {
        return Err(syn::Error::new(
            item.span(),
            "#[command] goes on an `impl Command for T` block",
        ));
    };
    if trait_path.segments.last().map(|s| s.ident != "Command") != Some(false) {
        return Err(syn::Error::new(
            trait_path.span(),
            "#[command] goes on an `impl Command for T` block",
        ));
    }

    let name = required(args.name, "name")?;
    let category = required(args.category, "category")?;
    let description = required(args.description, "description")?;
    let method = required(args.method, "method")?;
    let path = required(args.path, "path")?;
    let self_ty = item.self_ty.clone();

    let defined: Vec<String> = item
        .items
        .iter()
        .filter_map(|item| match item {
            ImplItem::Fn(f) => Some(f.sig.ident.to_string()),
            _ => None,
        })
        .collect();
    for (generated, given) in [
        ("meta", true),
        ("policy", args.policy.is_some()),
        ("positional_arg", args.positional.is_some()),
        ("read_only", args.read_only.is_some()),
        ("cli", args.cli.is_some()),
    ] {
        if given && defined.iter().any(|d| d == generated) {
            return Err(syn::Error::new(
                item.span(),
                format!("`fn {generated}` is generated by #[command]; remove it from the impl"),
            ));
        }
    }

    let output = item
        .items
        .iter()
        .find_map(|item| match item {
            ImplItem::Type(t) if t.ident == "Output" => Some(t.ty.clone()),
            _ => None,
        })
        .ok_or_else(|| syn::Error::new(item.span(), "missing `type Output = ...`"))?;

    let mut generated: Vec<ImplItem> = vec![syn::parse_quote! {
        fn meta() -> crate::domains::common::CommandMeta {
            crate::domains::common::CommandMeta {
                name: #name,
                category: #category,
                description: #description,
                method: #method,
                path: #path,
            }
        }
    }];
    if let Some(policy) = &args.policy {
        generated.push(syn::parse_quote! {
            fn policy() -> ::core::option::Option<&'static crate::kernel_imports::Policy> {
                ::core::option::Option::Some(&#policy)
            }
        });
    }
    if let Some(positional) = &args.positional {
        generated.push(syn::parse_quote! {
            fn positional_arg() -> ::core::option::Option<&'static str> {
                ::core::option::Option::Some(#positional)
            }
        });
    }
    if let Some(read_only) = &args.read_only {
        generated.push(syn::parse_quote! {
            fn read_only() -> bool {
                #read_only
            }
        });
    }
    if let Some(cli) = &args.cli {
        // A const so the declared slices get 'static promotion.
        generated.push(syn::parse_quote! {
            fn cli() -> ::core::option::Option<crate::domains::common::CliRoute> {
                const ROUTE: crate::domains::common::CliRoute = #cli;
                ::core::option::Option::Some(ROUTE)
            }
        });
    }
    item.items.extend(generated);

    let mut out = quote! {
        #item

        ::inventory::submit! { crate::domains::common::CommandDescriptor::of::<#self_ty>() }
    };

    if let Some((mode, mode_span)) = args.http {
        out.extend(expand_http(HttpSpec {
            mode,
            mode_span,
            self_ty: &self_ty,
            output: &output,
            name: &name,
            description: &description,
            method: &method,
            path: &path,
            tag: args.tag.as_ref().unwrap_or(&category),
            params: args.params,
            request_body: args.request_body,
            responses: args.responses,
        })?);
    } else if args.params.is_some() || args.request_body.is_some() || args.responses.is_some() {
        return Err(syn::Error::new(
            Span::call_site(),
            "params(..), request_body(..) and responses(..) document the generated HTTP \
             handler; add `http = <mode>`",
        ));
    }

    Ok(out)
}

struct HttpSpec<'a> {
    mode: HttpMode,
    mode_span: Span,
    self_ty: &'a Type,
    output: &'a Type,
    name: &'a LitStr,
    description: &'a LitStr,
    method: &'a LitStr,
    path: &'a LitStr,
    tag: &'a LitStr,
    params: Option<TokenStream2>,
    request_body: Option<TokenStream2>,
    responses: Option<TokenStream2>,
}

fn expand_http(spec: HttpSpec<'_>) -> syn::Result<TokenStream2> {
    let HttpSpec {
        mode,
        mode_span,
        self_ty,
        output,
        name,
        description,
        method,
        path,
        tag,
        params,
        request_body,
        responses,
    } = spec;

    let common = quote!(crate::api::common);
    let success = match mode {
        HttpMode::Plain => quote!((status = 200, description = "Success", body = #output)),
        HttpMode::WithUrls => {
            quote!((status = 200, description = "Success", body = #common::WithUrls<#output>))
        }
        HttpMode::Created => quote!((status = 201, description = "Created", body = #output)),
        HttpMode::CreatedWithUrls => {
            quote!((status = 201, description = "Created", body = #common::WithUrls<#output>))
        }
        HttpMode::NoContent => quote!((status = 204, description = "No content")),
        HttpMode::List => {
            let t = element_type(output, "Vec")?;
            quote!((status = 200, description = "Success", body = #common::ListResponse<#t>))
        }
        HttpMode::ListWithUrls => {
            let t = element_type(output, "Vec")?;
            quote!((status = 200, description = "Success",
                body = #common::ListResponse<#common::WithUrls<#t>>))
        }
        HttpMode::PaginatedWithUrls => {
            let t = element_type(output, "Paginated")?;
            quote!((status = 200, description = "Success",
                body = #common::PaginatedResponse<#common::WithUrls<#t>>))
        }
    };
    let _ = mode_span;

    let path_param_docs = path_params(&path.value()).into_iter().map(|param| {
        let lit = LitStr::new(&param, path.span());
        quote!((#lit = String, Path, description = "Prefixed public identifier"))
    });
    let user_params = params.into_iter();
    let request_body = request_body.map(|body| quote!(request_body = #body,));
    let user_responses = responses.map(|r| quote!(, #r));
    let http_method = syn::Ident::new(&method.value().to_lowercase(), method.span());

    let doc_fn = format_ident!("__command_openapi_{}", name.value());
    let doc_path = format_ident!("__path___command_openapi_{}", name.value());
    let respond = mode.respond_type();

    Ok(quote! {
        impl crate::api::command_http::HttpCommand for #self_ty {
            type Respond = #respond;
        }

        #[::utoipa::path(
            #http_method,
            path = #path,
            operation_id = #name,
            summary = #description,
            params(#(#path_param_docs,)* #(#user_params)*),
            #request_body
            responses(#success #user_responses),
            tags = [#tag]
        )]
        #[allow(dead_code)]
        fn #doc_fn() {}

        ::inventory::submit! {
            crate::api::command_http::CommandOpenApi::of::<#doc_path>()
        }
    })
}
