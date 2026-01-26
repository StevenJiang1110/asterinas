// SPDX-License-Identifier: MPL-2.0

use proc_macro2::TokenStream as TokenStream2;
use quote::{ToTokens, quote};
use syn::{
    Attribute, Data, DeriveInput, Path, Token, Visibility, parse_quote, punctuated::Punctuated,
    spanned::Spanned,
};

const DERIVE_IDENT: &str = "derive";
const REPR_IDENT: &str = "repr";
const REPR_C: &str = "C";

/// Splits attributes into non-derive attributes and derive paths
fn split_attrs(attrs: Vec<Attribute>) -> (Vec<Attribute>, Vec<Path>) {
    let mut other_attrs = Vec::new();
    let mut derive_paths = Vec::new();

    for attr in attrs {
        if attr.path().is_ident(DERIVE_IDENT) {
            let parsed: Punctuated<Path, Token![,]> = attr
                .parse_args_with(Punctuated::parse_terminated)
                .expect("failed to parse derive attribute");
            derive_paths.extend(parsed.into_iter());
        } else {
            other_attrs.push(attr);
        }
    }

    (other_attrs, derive_paths)
}

/// Checks if the attributes contain `#[repr(C)]`
fn has_repr_c(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident(REPR_IDENT) {
            return false;
        }
        let mut has_c = false;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident(REPR_C) {
                has_c = true;
            }
            Ok(())
        });
        has_c
    })
}

/// Inserts a path into the vector if it's not already present
fn insert_if_absent(paths: &mut Vec<Path>, new_path: Path) {
    let new_repr = new_path.to_token_stream().to_string();
    if !paths
        .iter()
        .any(|path| path.to_token_stream().to_string() == new_repr)
    {
        paths.push(new_path);
    }
}

pub fn expand_pod_union(mut input: DeriveInput) -> TokenStream2 {
    if !has_repr_c(&input.attrs) {
        panic!("`#[pod_union]` requires `#[repr(C)]` or `#[repr(C, ...)]` on unions");
    }

    let data_union = match input.data {
        Data::Union(ref u) => u,
        _ => panic!("`#[pod_union]` can only be used on unions"),
    };

    let vis: Visibility = input.vis.clone();
    let inner_vis: Visibility = match &vis {
        Visibility::Inherited => parse_quote!(pub(super)),
        other => other.clone(),
    };
    input.vis = inner_vis;

    // Split attributes: keep non-derive attrs, collect derive paths
    let (other_attrs, mut derive_paths) = split_attrs(input.attrs);

    // Add required zerocopy derives
    insert_if_absent(&mut derive_paths, parse_quote!(::zerocopy::FromBytes));
    insert_if_absent(&mut derive_paths, parse_quote!(::zerocopy::Immutable));
    insert_if_absent(&mut derive_paths, parse_quote!(::zerocopy::KnownLayout));

    // Rebuild derive attribute
    let derive_attr: Attribute = parse_quote! {
        #[derive(#(#derive_paths),*)]
    };

    input.attrs = other_attrs;
    input.attrs.push(derive_attr);

    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Generate Pod constraint assertions for all fields
    let field_pod_asserts = data_union.fields.named.iter().map(|field| {
        let ty = &field.ty;
        quote! {
            assert_pod::<#ty>();
        }
    });

    // Generate accessor methods for each field
    let accessor_methods = data_union.fields.named.iter().map(|field| {
        let field_name = &field.ident;
        let field_ty = &field.ty;

        let ref_method_name = field_name;
        let mut_method_name = syn::Ident::new(
            &format!("{}_mut", field_name.as_ref().unwrap()),
            field_name.span(),
        );

        quote! {
            pub fn #ref_method_name(&self) -> &#field_ty {
                let bytes = <Self as ::zerocopy::IntoBytes>::as_bytes(self);
                let slice = &bytes[..::core::mem::size_of::<#field_ty>()];
                <#field_ty as ::zerocopy::FromBytes>::ref_from_bytes(slice).unwrap()
            }

            pub fn #mut_method_name(&mut self) -> &mut #field_ty {
                let bytes = <Self as ::zerocopy::IntoBytes>::as_mut_bytes(self);
                let slice = &mut bytes[..::core::mem::size_of::<#field_ty>()];
                <#field_ty as ::zerocopy::FromBytes>::mut_from_bytes(slice).unwrap()
            }
        }
    });

    // Generate module name to avoid symbol conflicts
    let module_ident = syn::Ident::new(
        &format!(
            "__private_module_generated_by_ostd_pod_{}",
            ident.to_string().to_lowercase()
        ),
        proc_macro2::Span::call_site(),
    );

    // Copy constraint compile-time assertion
    let copy_assert = quote! {
        const _: () = {
            fn assert_copy<T: ::core::marker::Copy>() {}
            fn assert_union_copy #impl_generics() #where_clause {
                assert_copy::<#ident #ty_generics>();
            }
        };
    };

    // Field Pod constraint compile-time assertion
    let pod_assert = quote! {
        const _: () = {
            fn assert_pod<T: ::ostd_pod::Pod>() {}
            fn assert_union_fields #impl_generics() #where_clause {
                #(#field_pod_asserts)*
            }
        };
    };

    quote! {
        mod #module_ident {
            use super::*;

            #input

            impl #impl_generics #ident #ty_generics #where_clause {
                #(#accessor_methods)*
            }

            #pod_assert
            #copy_assert

            unsafe impl #impl_generics ::zerocopy::IntoBytes for #ident #ty_generics #where_clause {
                fn only_derive_is_allowed_to_implement_this_trait() {}
            }
        }

        #vis use #module_ident::#ident;
    }
}
