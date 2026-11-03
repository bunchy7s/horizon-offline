//! Generation of a mod's own trait methods, events and dispatch helpers.
use super::EventLifetimes;
use quote::{format_ident, quote};
use syn::visit_mut::VisitMut;

/// A trait declaration supplied to the staged game's build script after horizon-mixins has run.
pub struct TraitInjection {
    name: syn::Ident,
    /// The bindings' struct, as each catalog hook's (`InjectExampleTickEvent`).
    event: syn::Ident,
    parameters: Vec<(syn::Ident, syn::Type)>,
    contract: TraitContract,
}

/// What a trait method may do with its event.
pub enum TraitContract {
    /// HEAD: observe and change `&mut` bindings.
    Listen,
    /// Cancellable HEAD on a unit target, where `Flow::Cancel` returns early.
    Cancel,
    /// RETURN, MODIFY_ARG or MODIFY_EXPRESSION_VALUE: the ident names the `$value` binding,
    /// which is moved into the event and returned after the call.
    Value(syn::Ident),
}

struct EventFields {
    lifetimes: EventLifetimes,
    types: Vec<syn::Type>,
    scalars: Vec<bool>,
}

impl TraitInjection {
    /// Prepare a mod-owned trait method from parsed bindings and its return contract.
    pub fn new(
        name: syn::Ident,
        parameters: Vec<(syn::Ident, syn::Type)>,
        contract: TraitContract,
    ) -> Self {
        let event = format_ident!(
            "{}Event",
            name.to_string()
                .split('_')
                .map(|word| {
                    let mut chars = word.chars();
                    chars.next().map_or(String::new(), |first| {
                        first.to_uppercase().chain(chars).collect()
                    })
                })
                .collect::<String>()
        );
        Self {
            name,
            event,
            parameters,
            contract,
        }
    }

    /// The declarations supplied to the staged game's build script.
    pub fn input(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name.to_string(),
            "event": self.event.to_string(),
            "declaration": self.declaration(),
            "event_declaration": self.event(),
        })
    }

    /// Dispatch only to this injection’s owning mod, preserving its write-back contract.
    pub fn helper(&self, owner: usize) -> String {
        let name = &self.name;
        let event = &self.event;
        let params = self.parameters.iter().map(|(n, t)| quote!(#n: #t));
        let scalars = self.fields().scalars;
        let inits = self
            .parameters
            .iter()
            .zip(&scalars)
            .map(
                |((n, _), scalar)| {
                    if *scalar { quote!(#n: *#n) } else { quote!(#n) }
                },
            );
        let write_back = self
            .parameters
            .iter()
            .zip(&scalars)
            .filter(|(_, scalar)| **scalar)
            .map(|((n, _), _)| quote!(*#n = event.#n;));
        let call = quote! {
            let mut event = crate::mod_api::#event { #(#inits),* };
            let _flow = owner.#name(&mut event);
            #(#write_back)*
        };
        // Before the mods are installed (an injection into code `main_with` runs first), or
        // with fewer mods than the build had, the injection does nothing or passes the value on.
        let (output, result, absent) = match &self.contract {
            TraitContract::Listen => (quote!(), call, quote!()),
            TraitContract::Cancel => (
                quote!(-> core::ops::ControlFlow<()>),
                quote! {
                    #call
                    if _flow == crate::mod_api::Flow::Cancel {
                        return core::ops::ControlFlow::Break(());
                    }
                    core::ops::ControlFlow::Continue(())
                },
                quote!(core::ops::ControlFlow::Continue(())),
            ),
            TraitContract::Value(value) => {
                let ty = self
                    .parameters
                    .iter()
                    .find(|(n, _)| n == value)
                    .map(|(_, t)| t);
                (quote!(-> #ty), quote!(#call event.#value), quote!(#value))
            }
        };
        quote! {
            fn horizon_trait_dispatch(#(#params),*) #output {
                let Some(owner) = crate::mod_api::mods().get(#owner) else {
                    return #absent;
                };
                #result
            }
        }
        .to_string()
    }

    fn declaration(&self) -> String {
        let name = &self.name;
        let event = &self.event;
        let (output, default) = match self.contract {
            TraitContract::Cancel => (quote!(-> Flow), quote!(Flow::Continue)),
            TraitContract::Listen | TraitContract::Value(_) => (quote!(), quote!()),
        };
        let lifetimes = self.fields().lifetimes;
        let elided = match (lifetimes.a, lifetimes.b) {
            (_, true) => quote!(<'_, '_>),
            (true, false) => quote!(<'_>),
            (false, false) => quote!(),
        };
        quote!(fn #name(&self, _: &mut #event #elided) #output { #default }).to_string()
    }

    /// Which lifetimes the event takes, its fields' types and which of them are scalars.
    fn fields(&self) -> EventFields {
        let mut lifetimes = EventLifetimes::default();
        let (types, scalars) = self
            .parameters
            .iter()
            .map(|(name, ty)| match &self.contract {
                TraitContract::Value(value) if name == value => {
                    let mut field = ty.clone();
                    lifetimes.visit_type_mut(&mut field);
                    (field, false)
                }
                _ => lifetimes.field(ty),
            })
            .unzip();
        EventFields {
            lifetimes,
            types,
            scalars,
        }
    }

    fn event(&self) -> String {
        let event = &self.event;
        let name = self.name.to_string();
        let fields = self.fields();
        let lifetimes = fields.lifetimes;
        let types = fields.types;
        let generics = match (lifetimes.a, lifetimes.b) {
            (_, true) => quote!(<'a, 'b>),
            (true, false) => quote!(<'a>),
            (false, false) => quote!(),
        };
        let names = self.parameters.iter().map(|(n, _)| n);
        let doc = format!(
            " What [`Hooks::{name}`] is told. Fields may be added: the struct is `#[non_exhaustive]`."
        );
        quote! {
            #[doc = #doc]
            #[non_exhaustive]
            pub struct #event #generics { #(pub #names: #types),* }
        }
        .to_string()
    }
}
