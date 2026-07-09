//! `#[migration("<rfc3339>")]` attribute macro.
//!
//! Two accepted forms, dispatched on the function's return type:
//!
//! ```ignore
//! // SQL form: plain fn returning the query string
//! #[migration("2026-04-09T21:00:00Z")]
//! fn rename_vault_grant_principal() -> &'static str {
//!     "UPDATE vault_grant SET principal = ..."
//! }
//!
//! // Code form: async fn that owns its own db work
//! #[migration("2026-05-01T00:00:00Z")]
//! async fn rotate_credential_keys(db: &Surreal<Db>) -> Result<(), surrealdb::Error> {
//!     // …
//! }
//! ```
//!
//! Both expand to an `inventory::submit!` that registers a
//! `crate::db::migrations::Migration` with the parsed timestamp and a wrapper
//! closure. The original fn is left intact so it can be called directly from
//! tests.

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, LitStr, ReturnType, Type, parse_macro_input};

pub fn migration(attr: TokenStream, item: TokenStream) -> TokenStream {
    let ts_lit = parse_macro_input!(attr as LitStr);
    let ts_value = ts_lit.value();

    let nanos = match chrono::DateTime::parse_from_rfc3339(&ts_value) {
        Ok(dt) => match dt.with_timezone(&chrono::Utc).timestamp_nanos_opt() {
            Some(n) => n,
            None => {
                return syn::Error::new(
                    ts_lit.span(),
                    "`#[migration(...)]` timestamp is out of range for i64 nanos",
                )
                .to_compile_error()
                .into();
            }
        },
        Err(e) => {
            return syn::Error::new(
                ts_lit.span(),
                format!(
                    "`#[migration(...)]` timestamp must be a valid RFC 3339 literal: {e}"
                ),
            )
            .to_compile_error()
            .into();
        }
    };

    let input = parse_macro_input!(item as ItemFn);
    let fn_ident = input.sig.ident.clone();
    let is_async = input.sig.asyncness.is_some();

    let flavor = match classify_return(&input.sig.output) {
        Some(flavor) => flavor,
        None => {
            return syn::Error::new_spanned(
                &input.sig.output,
                "#[migration] fn must return either `&'static str` (SQL form) or \
                 `Result<(), surrealdb::Error>` (code form)",
            )
            .to_compile_error()
            .into();
        }
    };

    let run_body = match flavor {
        MigrationFlavor::Sql => {
            if is_async {
                return syn::Error::new_spanned(
                    input.sig.fn_token,
                    "SQL-form migration must not be `async`",
                )
                .to_compile_error()
                .into();
            }
            quote! {
                |db| Box::pin(async move {
                    let sql: &'static str = #fn_ident();
                    db.query(sql).await?.check()?;
                    Ok(())
                })
            }
        }
        MigrationFlavor::Code => {
            if !is_async {
                return syn::Error::new_spanned(
                    input.sig.fn_token,
                    "code-form migration must be `async`",
                )
                .to_compile_error()
                .into();
            }
            quote! {
                |db| Box::pin(#fn_ident(db))
            }
        }
    };

    let nanos_lit = proc_macro2::Literal::i64_suffixed(nanos);
    let expanded = quote! {
        #input

        ::inventory::submit! {
            crate::db::migrations::Migration {
                timestamp_nanos: #nanos_lit,
                run: #run_body,
            }
        }
    };

    expanded.into()
}

enum MigrationFlavor {
    Sql,
    Code,
}

fn classify_return(output: &ReturnType) -> Option<MigrationFlavor> {
    let ty = match output {
        ReturnType::Default => return None,
        ReturnType::Type(_, ty) => ty,
    };
    let rendered = quote!(#ty).to_string().replace(char::is_whitespace, "");
    if rendered == "&'staticstr" || rendered == "&str" {
        return Some(MigrationFlavor::Sql);
    }
    if let Type::Path(path) = ty.as_ref()
        && path
            .path
            .segments
            .last()
            .is_some_and(|seg| seg.ident == "Result")
    {
        return Some(MigrationFlavor::Code);
    }
    None
}
