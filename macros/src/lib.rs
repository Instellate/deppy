extern crate proc_macro;

use darling::{FromDeriveInput, FromField};
use proc_macro::TokenStream;
use quote::quote;
use syn::spanned::Spanned;
use syn::{parse_macro_input, DeriveInput};

#[derive(FromDeriveInput)]
#[darling(attributes(injectable))]
struct StructConfig {
    post_init: Option<syn::Path>,
}

#[derive(FromField)]
#[darling(attributes(injectable))]
struct FieldConfig {
    default_value: Option<syn::Lit>,
    get_value: Option<syn::Path>,
}

#[proc_macro_derive(Injectable, attributes(injectable))]
pub fn injectable(item: TokenStream) -> TokenStream {
    let derive: DeriveInput = parse_macro_input!(item as DeriveInput);

    let config = match StructConfig::from_derive_input(&derive) {
        Ok(c) => c,
        Err(e) => return e.write_errors().into(),
    };

    let struct_ = if let syn::Data::Struct(s) = derive.data {
        s
    } else {
        return syn::Error::new(derive.ident.span(), "Can only derive on structs")
            .to_compile_error()
            .into();
    };
    let struct_name = derive.ident;

    let mut init_fields = quote! {};

    for field in struct_.fields {
        let field_config = match FieldConfig::from_field(&field) {
            Ok(c) => c,
            Err(e) => return e.write_errors().into(),
        };

        if field_config.default_value.is_some() && field_config.get_value.is_some() {
            return syn::Error::new(
                field.span(),
                "Cannot specify both default value and get value",
            )
            .to_compile_error()
            .into();
        }

        if let Some(gv) = field_config.get_value {
            if let Some(i) = field.ident {
                init_fields = quote! {
                    #init_fields
                    #i: #gv(handler),
                }
            }
            continue;
        }

        if let Some(df) = field_config.default_value {
            if let Some(i) = field.ident {
                init_fields = quote! {
                    #init_fields
                    #i: #df.into(),
                }
            }
            continue;
        }

        let path = match field.ty {
            syn::Type::Path(tp) => tp,
            _ => {
                return syn::Error::new(field.ty.span(), "Type must be by value Dep")
                    .to_compile_error()
                    .into();
            }
        };

        let last_segment = match path.path.segments.last() {
            Some(ls) => ls,
            None => {
                return syn::Error::new(path.span(), "Type must be by value Dep")
                    .to_compile_error()
                    .into();
            }
        };

        if last_segment.ident != "Dep" {
            return syn::Error::new(last_segment.ident.span(), "Type must be by value Dep")
                .to_compile_error()
                .into();
        }

        let angle_bracketed = match &last_segment.arguments {
            syn::PathArguments::AngleBracketed(ab) => ab,
            _ => {
                return syn::Error::new(path.span(), "Type must be by value Dep")
                    .to_compile_error()
                    .into();
            }
        };

        let first_generic = match angle_bracketed.args.first() {
            Some(ga) => ga,
            None => {
                return syn::Error::new(path.span(), "Type must be by value Dep")
                    .to_compile_error()
                    .into();
            }
        };

        if let Some(i) = field.ident {
            init_fields = quote! {
                #init_fields
                #i: handler.get_required_service::<#first_generic>(),
            }
        }
    }

    if let Some(pi) = config.post_init {
        quote! {
            impl ::deppy::Injectable for #struct_name {
                fn inject<T: ServiceHandler>(handler: &T) -> Self {
                    let val = Self {
                       #init_fields
                    };
                    #pi(&val);
                    val
                }
            }
        }
    } else {
        quote! {
            impl ::deppy::Injectable for #struct_name {
                fn inject<T: ::deppy::ServiceHandler>(handler: &T) -> Self {
                    Self {
                       #init_fields
                    }
                }
            }
        }
    }
    .into()
}
