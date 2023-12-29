use std::collections::HashSet;

use proc_macro2::{TokenStream, TokenTree};
use quote::{format_ident, quote};
use syn::{Attribute, Data, DeriveInput, Field, Fields, Ident, Path, spanned::Spanned, Token, Type, Visibility};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::token::Comma;

const RENAME_ATTRIBUTE: &str = "optional_rename";
const SKIP_WRAP_ATTRIBUTE: &str = "optional_skip_wrap";
const WRAP_ATTRIBUTE: &str = "optional_wrap";
const CFG_ATTRIBUTE: &str = "cfg";

#[cfg(test)]
mod test;

struct DeriveInputWrapper {
    orig: DeriveInput,
    new: DeriveInput,
}

struct FieldOptions {
    wrapping_behavior: bool,
    cfg_attribute: Option<Attribute>,
    new_type: Option<TokenTree>,
    field_ident: TokenStream,
}

trait OptionalFieldVisitor {
    fn visit(&mut self, global_options: &GlobalOptions, old_field: &mut Field, new_field: &mut Field, field_options: &FieldOptions);
}

struct GenerateCanConvertImpl {
    acc: TokenStream,
}

impl GenerateCanConvertImpl {
    fn new() -> Self {
        GenerateCanConvertImpl {
            acc: quote!{ }
        }
    }

    fn get_implementation(self, derive_input: &DeriveInput, new: &DeriveInput) -> TokenStream {
        let (impl_generics, ty_generics, _) = derive_input.generics.split_for_impl();
        let new_name = &new.ident;
        let acc = self.acc;

        quote! {
            impl #impl_generics #new_name #ty_generics {
                fn can_convert(&self) -> bool {
                    #acc
                    true
                }
            }
        }
    }
}

impl OptionalFieldVisitor for GenerateCanConvertImpl {
    fn visit(&mut self, _global_options: &GlobalOptions, old_field: &mut Field, _new_field: &mut Field, field_options: &FieldOptions) {
        let ident = &field_options.field_ident;
        let cfg_attr = &field_options.cfg_attribute;

        let is_wrapped = field_options.wrapping_behavior;
        let is_nested = field_options.new_type.is_some();
        let is_base_opt = is_type_option(&old_field.ty);
        let inc = match (is_base_opt, is_wrapped, is_nested) {
            (_, true, false) =>
                    quote! { self.#ident.is_some() },
            (_, true, true) =>
                    quote! { if let Some(i) = &self.#ident { !i.can_convert() } else { false } },
            (_, false, true) =>
                    quote! { self.#ident.can_convert() },
            (_, false, false) => quote! { true }
        };
        let acc = &self.acc;
        self.acc = quote!{
            #acc
            #cfg_attr
            if !#inc {
                return false;
            }
        };
    }
}

struct GenerateTryFromImpl {
    field_assign_acc: TokenStream,
    field_check_acc: TokenStream,
}

impl GenerateTryFromImpl {
    fn new() -> Self {
        GenerateTryFromImpl {
            field_check_acc: quote! {},
            field_assign_acc: quote! {},
        }
    }

    fn get_implementation(self, derive_input: &DeriveInput, new: &DeriveInput) -> TokenStream {
        let (impl_generics, ty_generics, where_clause) = derive_input.generics.split_for_impl();
        let old_name = &derive_input.ident;
        let new_name = &new.ident;
        let field_check_acc = self.field_check_acc;
        let field_assign_acc = self.field_assign_acc;

        quote! {
            impl #impl_generics TryFrom<#new_name #ty_generics > #where_clause for #old_name #ty_generics {
                type Error = #new_name #ty_generics;

                fn try_from(v: Self::Error) -> Result<Self, Self::Error> {
                    #field_check_acc
                    Ok(Self {
                        #field_assign_acc
                    })
                }
            }
    }
    }
}

impl OptionalFieldVisitor for GenerateTryFromImpl {
    fn visit(&mut self, _global_options: &GlobalOptions, old_field: &mut Field, _new_field: &mut Field, field_options: &FieldOptions) {
        let ident = &field_options.field_ident;
        let cfg_attr = &field_options.cfg_attribute;

        let is_wrapped = field_options.wrapping_behavior;
        let is_nested = field_options.new_type.is_some();
        let is_base_opt = is_type_option(&old_field.ty);
        let (unwrap, check) = match (is_base_opt, is_wrapped, is_nested) {
            (_, true, false) =>
                (
                    quote! { .unwrap() },
                    quote! { #cfg_attr if v.#ident.is_none() { return Err(v); } }
                ),
            (_, true, true) =>
                (
                    quote! { .unwrap().try_into().unwrap() },
                    quote! { #cfg_attr if let Some(i) = &v.#ident { if !i.can_convert() { return Err(v); } } else { return Err(v); } }
                ),
            (_, false, true) =>
                (
                    quote! { .try_into().unwrap() },
                    quote! { #cfg_attr if !v.#ident.can_convert() { return Err(v); } }
                ),
            (_, false, false) =>
                (
                    quote! {},
                    quote! {}
                )
        };

        let field_assign_acc = &self.field_assign_acc;
        self.field_assign_acc = quote! {
            #field_assign_acc
            #cfg_attr

            #ident: v.#ident #unwrap,
        };

        let field_check_acc = &self.field_check_acc;
        self.field_check_acc = quote! {
            #field_check_acc
            #check
        };
    }
}


struct GenerateApplyFnVisitor {
    acc: TokenStream,
}

impl GenerateApplyFnVisitor {
    fn new() -> Self {
        GenerateApplyFnVisitor {
            acc: quote! {},
        }
    }

    fn get_implementation(self, orig: &DeriveInput, new: &DeriveInput) -> TokenStream {
        let (impl_generics, ty_generics, where_clause) = orig.generics.split_for_impl();
        let orig_name = &orig.ident;
        let new_name = &new.ident;
        let acc = self.acc;
        quote! {
            impl #impl_generics Applyable<#orig_name #ty_generics> #where_clause for #new_name #ty_generics {
                fn apply_to(self, t: &mut #orig_name #ty_generics) {
                    #acc
                }

                    /*
                fn can_be_applied(&self) -> bool {
                    #can_apply_acc
                    true
                }
                     */
            }
        }
    }
}

impl OptionalFieldVisitor for GenerateApplyFnVisitor {
    fn visit(&mut self, _global_options: &GlobalOptions, old_field: &mut Field, _new_field: &mut Field, field_options: &FieldOptions) {
        let ident = &field_options.field_ident;
        let acc = &self.acc;
        let cfg_attr = &field_options.cfg_attribute;

        let is_wrapped = field_options.wrapping_behavior;
        let is_nested = field_options.new_type.is_some();
        let is_base_opt = is_type_option(&old_field.ty);
        let inc = match (is_base_opt, is_wrapped, is_nested) {
            (true, false, true) => quote! {
                                   match (&mut t.#ident, self.#ident) {
                                       (None, Some(nested)) => t.#ident = nested.#ident.try_into(),
                                       (Some(existing), Some(nested)) => nested.#ident.apply_to(existing),
                                       (_, None) => {},
                                   }
                                },
            (true, false, false) => quote!{
                                    if self.#ident.is_some() {
                                        t.#ident = self.#ident;
                                    }
                                },
            (false, false, true) => quote!{ self.#ident.apply_to(&mut t.#ident); },
            (false, false, false) => quote!{ t.#ident = self.#ident; },
            (_, true, true) => quote!{ if let Some(inner) = self.#ident { inner.apply_to(&mut t.#ident); } },
            (_, true, false) => quote!{ if let Some(inner) = self.#ident { t.#ident = inner; } },
        };
        self.acc = quote! {
            #acc

            #cfg_attr
            #inc
        };
    }
}

struct SetNewFieldVisibilityVisitor;

impl OptionalFieldVisitor for SetNewFieldVisibilityVisitor {
    fn visit(&mut self, global_options: &GlobalOptions, _old_field: &mut Field, new_field: &mut Field, _field_options: &FieldOptions) {
        if global_options.make_fields_public {
            new_field.vis = Visibility::Public(syn::token::Pub(new_field.vis.span()))
        }
    }
}

struct SetNewFieldTypeVisitor;

impl OptionalFieldVisitor for SetNewFieldTypeVisitor {
    fn visit(&mut self, _global_options: &GlobalOptions, old_field: &mut Field, new_field: &mut Field, field_options: &FieldOptions) {
        let mut new_type = if let Some(t) = &field_options.new_type {
            quote! {#t}
        } else {
            let t = &old_field.ty;
            quote! {#t}
        };

        if field_options.wrapping_behavior {
            new_type = quote! {Option<#new_type>};
        };
        new_field.ty = Type::Verbatim(new_type);
    }
}

// https://github.com/rust-lang/rust/issues/65823 :(
struct RemoveHelperAttributesVisitor;

impl OptionalFieldVisitor for RemoveHelperAttributesVisitor {
    fn visit(&mut self, _global_options: &GlobalOptions, old_field: &mut Field, new_field: &mut Field, _field_options: &FieldOptions) {
        let indexes_to_remove = old_field
            .attrs
            .iter()
            .enumerate()
            .filter_map(|(i, a)| {
                if a.path().is_ident(RENAME_ATTRIBUTE) {
                    Some(i)
                } else if a.path().is_ident(SKIP_WRAP_ATTRIBUTE) {
                    Some(i)
                } else if a.path().is_ident(WRAP_ATTRIBUTE) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        // Don't forget to reverse so the indices are removed without being shifted!
        for i in indexes_to_remove.into_iter().rev() {
            old_field.attrs.swap_remove(i);
            new_field.attrs.swap_remove(i);
        }
    }
}

fn borrow_fields(derive_input: &mut DeriveInput) -> &mut Punctuated<Field, Comma> {
    let data_struct = match &mut derive_input.data {
        Data::Struct(data_struct) => data_struct,
        _ => panic!("OptionalStruct only works for structs :)"),
    };

    match &mut data_struct.fields {
        Fields::Unnamed(f) => &mut f.unnamed,
        Fields::Named(f) => &mut f.named,
        Fields::Unit => unreachable!("A struct cannot have simply a unit field?"),
    }
}

fn visit_fields(visitors: &mut [&mut dyn OptionalFieldVisitor], global_options: &GlobalOptions, derive_input: &DeriveInput) -> DeriveInputWrapper {
    let mut new = derive_input.clone();
    let mut old = derive_input.clone();
    let old_fields = borrow_fields(&mut old);
    let new_fields = borrow_fields(&mut new);

    for (struct_index, (old_field, new_field)) in old_fields.iter_mut().zip(new_fields.iter_mut()).enumerate() {
        let mut wrapping_behavior = !is_type_option(&old_field.ty) && global_options.default_wrapping_behavior;
        let mut cfg_attribute = None;
        let mut new_type = None;
        old_field.attrs
            .iter()
            .for_each(|a| {
                if a.path().is_ident(RENAME_ATTRIBUTE) {
                    let args = a
                        .parse_args()
                        .expect(&format!("'{RENAME_ATTRIBUTE}' attribute expects one and only one argument (the new type to use)"));
                    new_type = Some(args);
                    wrapping_behavior = false;
                } else if a.path().is_ident(SKIP_WRAP_ATTRIBUTE) {
                    wrapping_behavior = false;
                } else if a.path().is_ident(WRAP_ATTRIBUTE) {
                    wrapping_behavior = true;
                } else if a.path().is_ident(CFG_ATTRIBUTE) {
                    cfg_attribute = Some(a.clone());
                }
            });
        let field_ident = if let Some(ident) = &old_field.ident {
            quote! {#ident}
        } else {
            let i = syn::Index::from(struct_index);
            quote! {#i}
        };
        let field_options = FieldOptions { wrapping_behavior, cfg_attribute, new_type, field_ident };
        for v in &mut *visitors {
            v.visit(&global_options, old_field, new_field, &field_options);
        }
    }
    DeriveInputWrapper {
        orig: old,
        new,
    }
}

impl DeriveInputWrapper {
    fn set_new_name(&mut self, new_name: &str) {
        self.new.ident = Ident::new(new_name, self.new.ident.span());
    }

    fn get_derive_macros(
        &self,
        extra_derive: &[String],
    ) -> TokenStream {
        let mut extra_derive = extra_derive.iter().collect::<HashSet<_>>();
        for attributes in &self.new.attrs {
            let _ = attributes.parse_nested_meta(|derived_trait|
                {
                    let derived_trait = derived_trait.path;
                    let full_path = quote! { #derived_trait };
                    extra_derive.remove(&full_path.to_string());
                    Ok(())
                });
        }


        let mut acc = quote! {};
        for left_trait_to_derive in extra_derive {
            let left_trait_to_derive = format_ident!("{left_trait_to_derive}");
            acc = quote! { # left_trait_to_derive, # acc};
        }

        quote! { #[derive(#acc)] }
    }


    fn finalize_definition(self, macro_parameters: &GlobalOptions) -> (TokenStream, TokenStream) {
        let derives = self.get_derive_macros(&macro_parameters.extra_derive);

        let orig = self.orig;
        let new = self.new;
        (quote! { #orig }, quote! { #derives #new })
    }
}

struct ParsedMacroParameters {
    new_struct_name: Option<String>,
    default_wrapping: bool,
}

impl Parse for ParsedMacroParameters {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut out = ParsedMacroParameters {
            new_struct_name: None,
            default_wrapping: true,
        };

        if let Ok(struct_name) = Ident::parse(input) {
            out.new_struct_name = Some(struct_name.to_string());
        } else {
            return Ok(out);
        };

        if input.parse::<Token![,]>().is_err() {
            return Ok(out);
        };

        if let Ok(wrapping) = syn::LitBool::parse(input) {
            out.default_wrapping = wrapping.value;
        } else {
            return Ok(out);
        };

        Ok(out)
    }
}

// TODO this breaks for e.g. yolo::my::Option
fn is_path_option(p: &Path) -> bool {
    p.segments
        .last()
        .map(|ps| ps.ident == "Option")
        .unwrap_or(false)
}

fn is_type_option(t: &Type) -> bool {
    macro_rules! wtf {
        ($reason : tt) => {
            panic!(
                "Using OptionalStruct for a struct containing a {} is dubious...",
                $reason
            )
        };
    }

    match &t {
        // real work
        Type::Path(type_path) => is_path_option(&type_path.path),
        Type::Array(_) | Type::Tuple(_) => false,
        Type::Paren(type_paren) => is_type_option(&type_paren.elem),

        // No clue what to do with those
        Type::ImplTrait(_) | Type::TraitObject(_) => {
            panic!("Might already be an option I have no way to tell :/")
        }
        Type::Infer(_) => panic!("If you cannot tell, neither can I"),
        Type::Macro(_) => panic!("Don't think I can handle this easily..."),

        // Makes no sense to use those in an OptionalStruct
        Type::Reference(_) => wtf!("reference"),
        Type::Never(_) => wtf!("never-type"),
        Type::Slice(_) => wtf!("slice"),
        Type::Ptr(_) => wtf!("pointer"),
        Type::BareFn(_) => wtf!("function pointer"),

        // Help
        Type::Verbatim(_) => todo!("Didn't get what this was supposed to be..."),
        Type::Group(_) => todo!("Not sure what to do here"),

        // Have to wildcard here but I don't want to (unneeded as long as syn doesn't break semver
        // anyway)
        _ => panic!("Open an issue please :)"),
    }
}

struct GlobalOptions {
    new_struct_name: String,
    extra_derive: Vec<String>,
    default_wrapping_behavior: bool,
    make_fields_public: bool,
}

impl GlobalOptions {
    fn new(attr: ParsedMacroParameters, struct_definition: &DeriveInput) -> Self {
        let new_struct_name = attr.new_struct_name.unwrap_or_else(|| "Optional".to_owned() + &struct_definition.ident.to_string());
        let default_wrapping_behavior = attr.default_wrapping;
        GlobalOptions {
            new_struct_name,
            extra_derive: vec!["Clone", "PartialEq", "Default", "Debug"]
                .into_iter()
                .map(|s| s.to_owned())
                .collect(),
            default_wrapping_behavior,
            make_fields_public: true,
        }
    }
}

/*
// TODO: copy pasted code
let can_apply_acc = match &fields {
    Fields::Unit => unreachable!(),
    Fields::Named(fields_named) => {
        let it = fields_named.named.iter().map(|f| (f.ident.as_ref().unwrap(), &f.attrs));
        acc_check::<_, _, Ident>(it)
    }
    Fields::Unnamed(fields_unnamed) => {
        let it = fields_unnamed.unnamed.iter().enumerate().map(|(i, field)| {
            let i = syn::Index::from(i);
            (quote! {#i}, &field.attrs)
        });
        acc_check(it)
    }
};
*/

pub struct OptionalStructOutput {
    pub original: TokenStream,
    pub generated: TokenStream,
}

pub fn opt_struct(
    attr: TokenStream,
    input: TokenStream,
) -> OptionalStructOutput {
    let derive_input = syn::parse2::<DeriveInput>(input).unwrap();
    let macro_params = GlobalOptions::new(syn::parse2::<_>(attr).unwrap(), &derive_input);

    let mut apply_fn_generator = GenerateApplyFnVisitor::new();
    let mut try_from_generator = GenerateTryFromImpl::new();
    let mut can_convert_generator = GenerateCanConvertImpl::new();

    let mut visitors = [
        &mut RemoveHelperAttributesVisitor as &mut dyn OptionalFieldVisitor,
        &mut SetNewFieldVisibilityVisitor,
        &mut SetNewFieldTypeVisitor,
        &mut apply_fn_generator,
        &mut try_from_generator,
        &mut can_convert_generator,
    ];

    let mut output = visit_fields(&mut visitors, &macro_params, &derive_input);

    output.set_new_name(&macro_params.new_struct_name);

    let apply_fn_impl = apply_fn_generator.get_implementation(&derive_input, &output.new);
    let try_from_impl = try_from_generator.get_implementation(&derive_input, &output.new);
    let can_convert_impl = can_convert_generator.get_implementation(&derive_input, &output.new);

    let (original, new) = output.finalize_definition(&macro_params);

    let generated = quote! {
        #new
        #apply_fn_impl
        #try_from_impl
        #can_convert_impl
    };

    OptionalStructOutput {
        original,
        generated,
    }
}