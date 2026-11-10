//! `#[timed]`: record a function's duration in `op_duration_seconds{component, op, outcome}`.
//! See the `microscaler-timing` crate for the semantics; this crate only expands the attribute.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2, TokenTree};
use quote::{quote, ToTokens};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{
    parse_macro_input, Expr, ExprLit, ItemFn, Lit, LitBool, LitStr, Meta, ReturnType, Token, Type,
};

/// Time every call of the function into `op_duration_seconds{component, op, outcome}`.
///
/// - `#[timed]`: `component` = the module path, `op` = the function name.
/// - `#[timed(component = "lifeguard.pool", op = "checkout")]`: either may be given; both must
///   be string literals (static, low-cardinality names; never data).
/// - `#[timed(result = false)]`: the return type looks like a `Result` (ends in `Result`) but is
///   not one; record `ok` always.
/// - `#[timed(crate = "path")]`: where `microscaler_timing` is, if not at `::microscaler_timing`.
///
/// Works on sync and `async` fns, free functions and methods. If the return type is a `Result`
/// (its last path segment ends in `Result`: `Result`, `io::Result`, `anyhow::Result`, ...), `Ok`
/// records `ok` and `Err` records `err`, or `timeout` when the error implements
/// `microscaler_timing::IsTimeout` and says so. A panic records `err`; an async call dropped
/// before completing (cancelled, timed out by its caller) records `err`.
#[proc_macro_attribute]
pub fn timed(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as Args);
    let func = parse_macro_input!(item as ItemFn);
    match expand(&args, func) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[derive(Default)]
struct Args {
    component: Option<LitStr>,
    op: Option<LitStr>,
    result: Option<bool>,
    krate: Option<syn::Path>,
}

impl Parse for Args {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut args = Self::default();
        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        for meta in metas {
            let Meta::NameValue(nv) = meta else {
                return Err(syn::Error::new_spanned(
                    meta,
                    "expected `component = \"...\"`, `op = \"...\"`, `result = false` or `crate = \"...\"`",
                ));
            };
            let key = nv
                .path
                .get_ident()
                .map(ToString::to_string)
                .unwrap_or_default();
            match (key.as_str(), &nv.value) {
                (
                    "component",
                    Expr::Lit(ExprLit {
                        lit: Lit::Str(s), ..
                    }),
                ) => {
                    validate_label(s)?;
                    args.component = Some(s.clone());
                }
                (
                    "op",
                    Expr::Lit(ExprLit {
                        lit: Lit::Str(s), ..
                    }),
                ) => {
                    validate_label(s)?;
                    args.op = Some(s.clone());
                }
                (
                    "result",
                    Expr::Lit(ExprLit {
                        lit: Lit::Bool(LitBool { value, .. }),
                        ..
                    }),
                ) => {
                    args.result = Some(*value);
                }
                (
                    "crate",
                    Expr::Lit(ExprLit {
                        lit: Lit::Str(s), ..
                    }),
                ) => {
                    args.krate = Some(s.parse()?);
                }
                ("component" | "op" | "crate", _) => {
                    return Err(syn::Error::new_spanned(
                        &nv.value,
                        "must be a string literal: labels are static names, never runtime values",
                    ));
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        &nv.path,
                        "unknown argument; expected `component`, `op`, `result` or `crate`",
                    ));
                }
            }
        }
        Ok(args)
    }
}

/// Labels are code names: letters, digits and `_ . : / -`, at most 128 bytes.
fn validate_label(s: &LitStr) -> syn::Result<()> {
    let v = s.value();
    let ok = !v.is_empty()
        && v.len() <= 128
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '/' | '-'));
    if ok {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            s,
            "label must be 1-128 characters of [A-Za-z0-9_.:/-] (a static code name, never data)",
        ))
    }
}

/// Whether the return type is (syntactically) a `Result`: its last path segment ends in `Result`.
fn returns_result(ret: &ReturnType) -> bool {
    let ReturnType::Type(_, ty) = ret else {
        return false;
    };
    let Type::Path(path) = &**ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|seg| seg.ident.to_string().ends_with("Result"))
}

/// Whether the type mentions `impl`, a reference or a lifetime (then it cannot be written on a
/// closure: elided lifetimes are not allowed there, `impl Trait` is not allowed in a `let`).
fn type_tokens(ty: &Type) -> (bool, bool) {
    fn walk(stream: TokenStream2, has_impl: &mut bool, has_ref: &mut bool) {
        for tt in stream {
            match tt {
                TokenTree::Ident(i) if i == "impl" => *has_impl = true,
                TokenTree::Punct(p) if p.as_char() == '&' || p.as_char() == '\'' => *has_ref = true,
                TokenTree::Group(g) => walk(g.stream(), has_impl, has_ref),
                _ => {}
            }
        }
    }
    let (mut has_impl, mut has_ref) = (false, false);
    walk(ty.to_token_stream(), &mut has_impl, &mut has_ref);
    (has_impl, has_ref)
}

fn expand(args: &Args, func: ItemFn) -> syn::Result<TokenStream2> {
    let ItemFn {
        attrs,
        vis,
        sig,
        block,
        // syn 3 fn modifiers (future syntax: gen, final, contracts) parse to nothing on stable Rust and syn
        // itself prints an ItemFn without them, so the timed wrapper does the same.
        ..
    } = func;
    if sig.constness.is_some() {
        return Err(syn::Error::new_spanned(
            sig.constness,
            "#[timed] cannot time a const fn",
        ));
    }
    let krate = args
        .krate
        .clone()
        .unwrap_or_else(|| syn::parse_quote!(::microscaler_timing));
    let op = args
        .op
        .clone()
        .unwrap_or_else(|| LitStr::new(&sig.ident.to_string(), sig.ident.span()));
    let component = args
        .component
        .as_ref()
        .map_or_else(|| quote!(::core::module_path!()), ToTokens::to_token_stream);
    let is_result = args.result.unwrap_or(true) && returns_result(&sig.output);
    let site = syn::Ident::new("__MS_TIMING_SITE", Span::mixed_site());

    let finish = if is_result {
        quote! {
            let __ms_outcome = match &__ms_ret {
                ::core::result::Result::Ok(_) => #krate::Outcome::Ok,
                ::core::result::Result::Err(__ms_err) => {
                    #[allow(unused_imports)]
                    use #krate::__private::{ViaPlain as _, ViaTimeout as _};
                    (&#krate::__private::Probe(__ms_err)).__ms_outcome()
                }
            };
            __ms_timer.finish(__ms_outcome);
        }
    } else {
        quote! { __ms_timer.finish(#krate::Outcome::Ok); }
    };

    let (ret_ty, has_impl, has_ref) = match &sig.output {
        ReturnType::Default => (Some(quote!(())), false, false),
        ReturnType::Type(_, ty) => {
            let (i, r) = type_tokens(ty);
            (Some(ty.to_token_stream()), i, r)
        }
    };
    // `let` annotation gives `?` in the body its error type; impossible for `impl Trait`.
    let let_ty = if has_impl {
        quote!()
    } else {
        let ty = ret_ty.clone().unwrap_or_default();
        quote!(: #ty)
    };

    let body = if sig.asyncness.is_some() {
        quote! {
            let mut __ms_timer = #site.start();
            __ms_timer.set_drop_outcome(#krate::Outcome::Err);
            let __ms_ret #let_ty = async move #block.await;
            #finish
            __ms_ret
        }
    } else {
        let closure_ret = if has_impl || has_ref {
            quote!()
        } else {
            let ty = ret_ty.unwrap_or_default();
            quote!(-> #ty)
        };
        quote! {
            let __ms_timer = #site.start();
            #[allow(clippy::redundant_closure_call)]
            let __ms_ret #let_ty = (move || #closure_ret #block)();
            #finish
            __ms_ret
        }
    };

    Ok(quote! {
        #(#attrs)*
        #vis #sig {
            static #site: #krate::Site = #krate::Site::new(#component, #op);
            #body
        }
    })
}
