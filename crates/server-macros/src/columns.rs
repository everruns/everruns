//! `#[derive(Columns)]`: the SQL column list of a `FromRow` struct.
//!
//! Emits `pub const COLUMNS: &'static str`, the struct's field names joined
//! with `", "` in declaration order. Repositories write `sql!("SELECT {Row}
//! FROM ...")`, so a column list is spelled once, next to the struct
//! `FromRow` decodes into, instead of once per query, and cannot drift from
//! the fields it has to fill.
//!
//! Honors `#[sqlx(rename = "...")]` and leaves out `#[sqlx(skip)]` fields.
//! `#[sqlx(flatten)]` is rejected: the flattened struct's columns would have
//! to come from its own derive, which a string constant cannot express.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, LitStr};

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream2> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Columns can only be derived for structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Columns needs named fields",
        ));
    };

    let mut columns = Vec::new();
    for field in &fields.named {
        let mut name = field.ident.as_ref().map(|ident| ident.to_string());
        for attr in field.attrs.iter().filter(|a| a.path().is_ident("sqlx")) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("skip") {
                    name = None;
                } else if meta.path.is_ident("rename") {
                    name = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("flatten") {
                    return Err(meta.error("Columns does not support #[sqlx(flatten)]"));
                } else if meta.input.peek(syn::Token![=]) {
                    // Other key = value options (`try_from`, `json`, ...) do
                    // not change the column name.
                    let _: syn::Expr = meta.value()?.parse()?;
                }
                Ok(())
            })?;
        }
        columns.extend(name);
    }

    let list = columns.join(", ");
    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics #ident #ty_generics #where_clause {
            /// The columns this row decodes, in field order, for `SELECT` and
            /// `RETURNING` lists.
            pub const COLUMNS: &'static str = #list;
        }
    })
}

/// `sql!("SELECT {Row} FROM t WHERE id = $1")`: a `&'static str` query with
/// `{Row}` replaced by `Row::COLUMNS`.
///
/// A placeholder is `{` + a Rust path starting with an uppercase letter +
/// `}`, so Postgres literals such as `'{}'::jsonb` or `'{a,b}'` pass through.
pub(crate) fn expand_sql(lit: LitStr) -> syn::Result<TokenStream2> {
    let text = lit.value();
    let mut parts: Vec<TokenStream2> = Vec::new();
    let mut rest = text.as_str();
    let mut found = false;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let name = &after[..close];
        let is_path = name.starts_with(|c: char| c.is_ascii_uppercase())
            && name.split("::").all(|seg| {
                !seg.is_empty() && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
        if !is_path {
            parts.push(literal(&rest[..open + 1], &lit));
            rest = after;
            continue;
        }
        let path: syn::Path = syn::parse_str(name)?;
        parts.push(literal(&rest[..open], &lit));
        parts.push(quote!(#path::COLUMNS));
        rest = &after[close + 1..];
        found = true;
    }
    if !found {
        return Err(syn::Error::new_spanned(
            &lit,
            "sql! needs at least one {Row} placeholder; use a plain string literal otherwise",
        ));
    }
    parts.push(literal(rest, &lit));
    Ok(quote!(::const_format::concatcp!(#(#parts),*)))
}

fn literal(text: &str, span_of: &LitStr) -> TokenStream2 {
    let lit = LitStr::new(text, span_of.span());
    quote!(#lit)
}
