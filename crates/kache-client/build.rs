use heck::ToKebabCase;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Aliases the parser accepts alongside the kebab-case variant name.
/// The key must match a variant below; the build fails otherwise.
const ALIASES: &[(&str, &[&str])] = &[("stats", &["status"]), ("explain", &["why-miss"])];

fn main() {
    println!("cargo:rerun-if-changed=src/commands.rs");
    let schema = std::fs::read_to_string("src/commands.rs").expect("read engine command schema");
    let parsed: syn::DeriveInput = syn::parse_str(&schema).expect("parse engine command schema");
    let syn::Data::Enum(commands) = parsed.data else {
        panic!("Commands must be an enum");
    };
    let mut names = BTreeSet::from(["help".to_owned()]);
    let mut variants = BTreeSet::new();
    for variant in commands.variants {
        if variant.attrs.iter().any(|attr| !attr.path().is_ident("doc")) {
            panic!("routing reads variant names only; remove attributes from the schema");
        }
        let name = variant.ident.to_string().to_kebab_case();
        variants.insert(name.clone());
        names.insert(name);
    }
    for (command, aliases) in ALIASES {
        assert!(
            variants.contains(*command),
            "alias target {command} is not a command"
        );
        names.extend(aliases.iter().map(|alias| (*alias).to_owned()));
    }
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(output.join("commands.rs"), format!("#[doc(hidden)]\n#[macro_export]\nmacro_rules! define_engine_commands {{ () => {{ {schema} }}; }}\npub fn engine_command_names() -> &'static [&'static str] {{ &{:?} }}\n", names.into_iter().collect::<Vec<_>>())).expect("write shared command definition");
}
