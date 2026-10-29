use heck::ToKebabCase;
use std::collections::BTreeSet;
use std::path::PathBuf;
use syn::{Expr, Lit, Meta};

fn string(expr: &Expr) -> String {
    let Expr::Lit(literal) = expr else {
        panic!("command routing names must be string literals")
    };
    let Lit::Str(value) = &literal.lit else {
        panic!("command routing names must be strings")
    };
    value.value()
}

fn main() {
    println!("cargo:rerun-if-changed=src/commands.rs");
    let schema = std::fs::read_to_string("src/commands.rs").expect("read engine command schema");
    let parsed: syn::DeriveInput = syn::parse_str(&schema).expect("parse engine command schema");
    let syn::Data::Enum(commands) = parsed.data else {
        panic!("Commands must be an enum");
    };
    let mut names = BTreeSet::from(["help".to_owned()]);
    for variant in commands.variants {
        let mut name = variant.ident.to_string().to_kebab_case();
        for attr in variant
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("command"))
        {
            let items = attr
                .parse_args_with(
                    syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
                )
                .expect("parse command attributes");
            for item in items {
                match item {
                    Meta::NameValue(value) if value.path.is_ident("name") => {
                        name = string(&value.value)
                    }
                    Meta::NameValue(value)
                        if value.path.is_ident("alias") || value.path.is_ident("visible_alias") =>
                    {
                        names.insert(string(&value.value));
                    }
                    Meta::Path(path)
                        if path.is_ident("flatten") || path.is_ident("external_subcommand") =>
                    {
                        panic!("update client routing for flattened or external commands")
                    }
                    _ => {}
                }
            }
        }
        names.insert(name);
    }
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(output.join("commands.rs"), format!("#[doc(hidden)]\n#[macro_export]\nmacro_rules! define_engine_commands {{ () => {{ {schema} }}; }}\npub fn engine_command_names() -> &'static [&'static str] {{ &{:?} }}\n", names.into_iter().collect::<Vec<_>>())).expect("write shared command definition");
}
