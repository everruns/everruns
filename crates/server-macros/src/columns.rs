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
///
/// `{Row as a}` qualifies every column with a table alias (`a.id, a.name`),
/// for joins. A string constant cannot be mapped at compile time, so a query
/// with an aliased placeholder is assembled once, on first use, into a
/// `static` and still hands out a `&'static str`.
pub(crate) fn expand_sql(lit: LitStr) -> syn::Result<TokenStream2> {
    let text = lit.value();
    let mut parts: Vec<TokenStream2> = Vec::new();
    let mut rest = text.as_str();
    let mut found = false;
    let mut aliased = false;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let inner = &after[..close];
        let (name, alias) = match inner.split_once(" as ") {
            Some((name, alias)) => (name.trim(), Some(alias.trim())),
            None => (inner, None),
        };
        let is_path = name.starts_with(|c: char| c.is_ascii_uppercase())
            && name.split("::").all(|seg| {
                !seg.is_empty() && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
        let alias_ok = alias.is_none_or(|a| {
            a.starts_with(|c: char| c.is_ascii_lowercase())
                && a.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        });
        if !is_path || !alias_ok {
            parts.push(literal(&rest[..open + 1], &lit));
            rest = after;
            continue;
        }
        let path: syn::Path = syn::parse_str(name)?;
        parts.push(literal(&rest[..open], &lit));
        parts.push(match alias {
            None => quote!(#path::COLUMNS),
            Some(alias) => {
                aliased = true;
                let prefix = format!("{alias}.");
                quote!(&#path::COLUMNS
                    .split(", ")
                    .map(|column| ::std::format!("{}{}", #prefix, column))
                    .collect::<::std::vec::Vec<_>>()
                    .join(", "))
            }
        });
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
    if !aliased {
        return Ok(quote!(::const_format::concatcp!(#(#parts),*)));
    }
    Ok(quote!({
        static QUERY: ::std::sync::LazyLock<::std::string::String> =
            ::std::sync::LazyLock::new(|| {
                let mut query = ::std::string::String::new();
                #(query.push_str(#parts);)*
                query
            });
        QUERY.as_str()
    }))
}

fn literal(text: &str, span_of: &LitStr) -> TokenStream2 {
    let lit = LitStr::new(text, span_of.span());
    quote!(#lit)
}
