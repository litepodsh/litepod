//! The guardrail for OWASP A01/A04: no organization-scoped route may authorize
//! itself by hand.
//!
//! Seven critical cross-tenant bugs in this codebase were the same omission
//! repeated — a handler that took `organization_id` out of the path and never
//! checked it. The extractors in `middleware::org_scope` make the check
//! unskippable *if used*; this test is what forces them to be used. It reads the
//! route table as text, resolves each handler back to its source, and fails when
//! an organization-scoped route reaches a handler whose signature carries none
//! of them.
//!
//! Reading source text is unusual for a test, but it is the only way to see the
//! route table: axum's `Router` exposes no introspection, and a runtime test
//! would need a valid body for every POST/PUT on the platform. This costs
//! nothing, needs no dependencies, and would have caught all seven.
//!
//! When it fails, either add the extractor to the handler, or add the route to
//! `ALLOWLIST` with a reason that survives review.

use std::collections::BTreeSet;
use std::fs;

const ROUTER_SRC: &str = include_str!("router.rs");
const TERMINAL_SRC: &str = include_str!("../podman/terminal/exec.rs");

/// Any of these in a handler signature proves the check ran.
const EXTRACTORS: &[&str] = &[
    // `security::authz::extract`
    "RequireOrg<",
    "RequireProject<",
    "RequireEnv<",
    "RequireApp<",
];

/// Organization-scoped handlers not yet routed through an extractor.
///
/// Every entry needs a reason. The list only shrinks: a stale entry — one whose
/// handler now uses an extractor — fails `allowlist_has_no_stale_entries`, so it
/// cannot rot into a permanent exemption.
///
/// Everything here still authorizes correctly today, by hand. Being on this list
/// means the check is a convention rather than a guarantee — nothing stops the
/// next edit from dropping it. Migrating a module is a mechanical change: swap
/// the `Path`/`Extension` pair for the matching extractor, delete the two check
/// lines, and delete its entries below.
///
/// This list covers path-scoped routes only. Body-scoped requests, including
/// nested organization IDs, are checked separately by `body_scope` below.
const ALLOWLIST: &[(&str, &str)] = &[
    // Empty, and worth keeping that way: every organization-scoped route in the
    // product now takes an extractor. An entry here means a route where the check
    // is a convention again rather than a guarantee.
];

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Route {
    path: String,
    handler: String,
}

/// Pulls `(path, handler)` pairs out of `.route("...", get(a::b).post(c::d))`
/// calls by walking balanced parentheses. Deliberately dumb: it understands the
/// shape of a route registration and nothing else about Rust.
fn routes_in(src: &str) -> Vec<Route> {
    let mut found = Vec::new();
    let bytes = src.as_bytes();
    let mut cursor = 0;

    while let Some(offset) = src[cursor..].find(".route(") {
        let open = cursor + offset + ".route(".len();
        let mut depth = 1usize;
        let mut index = open;
        let mut in_string = false;
        while index < bytes.len() && depth > 0 {
            match bytes[index] {
                b'"' if !in_string => in_string = true,
                // Route literals contain no escapes; a naive close is correct here.
                b'"' if in_string => in_string = false,
                b'(' if !in_string => depth += 1,
                b')' if !in_string => depth -= 1,
                _ => {}
            }
            index += 1;
        }
        let span = &src[open..index.saturating_sub(1)];
        cursor = index;

        let Some(path) = span
            .split_once('"')
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(literal, _)| literal.to_string())
        else {
            continue;
        };

        for handler in handlers_in(span) {
            found.push(Route {
                path: path.clone(),
                handler,
            });
        }
    }
    found
}

/// Every `module::some_handler` mentioned inside a route registration.
fn handlers_in(span: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (index, _) in span.match_indices("::") {
        let module: String = span[..index]
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let name: String = span[index + 2..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !module.is_empty() && name.ends_with("_handler") {
            found.push(format!("{module}::{name}"));
        }
    }
    found
}

/// `compose::compose_deploy_handler` -> `src/handler/compose.rs`.
fn source_of(module: &str) -> Option<String> {
    let root = env!("CARGO_MANIFEST_DIR");
    let candidates = [
        format!("{root}/src/handler/{module}.rs"),
        format!("{root}/src/{module}/mod.rs"),
        // Enterprise handlers (`crate::ee::handler::sso`).
        format!("{root}/src/ee/handler/{module}.rs"),
        // SCIM handlers (`use crate::ee::sso::scim`), scoped by `ScimTenant`.
        format!("{root}/src/ee/sso/{module}.rs"),
        // Request analytics (`crate::metrics_history::requests`).
        format!("{root}/src/metrics_history/{module}.rs"),
    ];
    candidates
        .iter()
        .find_map(|path| fs::read_to_string(path).ok())
}

/// The parameter list of `pub async fn {name}`, up to the return arrow.
fn signature_of<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let start = src.find(&format!("pub async fn {name}("))?;
    let end = src[start..].find(") ->")? + start;
    Some(&src[start..end])
}

fn org_scoped_routes() -> Vec<Route> {
    let mut routes: Vec<Route> = routes_in(ROUTER_SRC)
        .into_iter()
        .chain(routes_in(TERMINAL_SRC))
        .filter(|route| route.path.contains("{organization_id}"))
        .collect();
    routes.sort();
    routes.dedup();
    routes
}

#[test]
fn every_organization_scoped_route_authorizes_through_an_extractor() {
    let allowed: BTreeSet<&str> = ALLOWLIST.iter().map(|(handler, _)| *handler).collect();
    let mut offenders = Vec::new();

    for route in org_scoped_routes() {
        if allowed.contains(route.handler.as_str()) {
            continue;
        }
        let (module, name) = route.handler.split_once("::").expect("qualified handler");
        let Some(src) = source_of(module) else {
            offenders.push(format!(
                "{}  ({}) — could not locate the source file for module `{module}`",
                route.handler, route.path
            ));
            continue;
        };
        let Some(signature) = signature_of(&src, name) else {
            offenders.push(format!(
                "{}  ({}) — no `pub async fn {name}` found in module `{module}`",
                route.handler, route.path
            ));
            continue;
        };
        if !EXTRACTORS.iter().any(|marker| signature.contains(marker)) {
            offenders.push(format!("{}  ({})", route.handler, route.path));
        }
    }

    assert!(
        offenders.is_empty(),
        "These organization-scoped routes authorize by hand instead of taking an \
         `RequireOrg`/`RequireProject`/`RequireEnv`/`RequireApp` \
         extractor (see `security::authz::extract`). Add the extractor, or add the handler to \
         `ALLOWLIST` in this file with a reason:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn allowlist_has_no_stale_entries() {
    let mut stale = Vec::new();

    for (handler, reason) in ALLOWLIST {
        let Some((module, name)) = handler.split_once("::") else {
            stale.push(format!("{handler} — not a `module::handler` path"));
            continue;
        };
        let Some(src) = source_of(module) else {
            stale.push(format!("{handler} — module `{module}` no longer exists"));
            continue;
        };
        let Some(signature) = signature_of(&src, name) else {
            stale.push(format!("{handler} — handler no longer exists"));
            continue;
        };
        if EXTRACTORS.iter().any(|marker| signature.contains(marker)) {
            stale.push(format!("{handler} — now uses an extractor; drop the entry"));
            continue;
        }
        assert!(
            !reason.trim().is_empty(),
            "{handler} is allowlisted without a reason"
        );
    }

    assert!(
        stale.is_empty(),
        "`ALLOWLIST` in this file is out of date. It is meant to shrink, never to rot:\n  {}",
        stale.join("\n  ")
    );
}

#[test]
fn the_route_parser_actually_sees_the_route_table() {
    // Guards the two tests above against silently passing on an empty parse —
    // a change to how routes are written would otherwise disable the guardrail
    // rather than fail it.
    let routes = org_scoped_routes();
    assert!(
        routes.len() > 100,
        "parsed only {} organization-scoped routes; the parser has lost track of \
         the route table in `router.rs`",
        routes.len()
    );
    assert!(
        routes
            .iter()
            .any(|route| route.handler == "compose::compose_content_put_handler"),
        "parser missed a known organization-scoped route"
    );
}

// Body IDs can also be nested (e.g. invitations.organizations[].organization_id).
// Parse Rust types rather than matching DTO names or the spelling of Vec/Option.
mod body_scope {
    use super::*;
    use std::collections::BTreeMap;
    use syn::visit::Visit;

    #[derive(Default)]
    struct Types(BTreeSet<String>);
    impl<'ast> Visit<'ast> for Types {
        fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
            if let Some(segment) = node.path.segments.last() {
                self.0.insert(segment.ident.to_string());
            }
            syn::visit::visit_type_path(self, node);
        }
    }

    #[derive(Default)]
    struct Definitions {
        direct: BTreeSet<String>,
        dependencies: BTreeMap<String, BTreeSet<String>>,
    }
    impl<'ast> Visit<'ast> for Definitions {
        fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
            let name = node.ident.to_string();
            let mut types = Types::default();
            for field in &node.fields {
                if field
                    .ident
                    .as_ref()
                    .is_some_and(|id| id == "organization_id")
                {
                    self.direct.insert(name.clone());
                }
                types.visit_type(&field.ty);
            }
            self.dependencies.entry(name).or_default().extend(types.0);
            syn::visit::visit_item_struct(self, node);
        }
        fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
            let name = node.ident.to_string();
            let mut types = Types::default();
            for variant in &node.variants {
                for field in &variant.fields {
                    if field
                        .ident
                        .as_ref()
                        .is_some_and(|id| id == "organization_id")
                    {
                        self.direct.insert(name.clone());
                    }
                    types.visit_type(&field.ty);
                }
            }
            self.dependencies.entry(name).or_default().extend(types.0);
        }
        fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
            let mut types = Types::default();
            types.visit_type(&node.ty);
            self.dependencies
                .entry(node.ident.to_string())
                .or_default()
                .extend(types.0);
        }
    }
    impl Definitions {
        fn scoped(&self) -> BTreeSet<String> {
            let mut scoped = self.direct.clone();
            loop {
                let before = scoped.len();
                for (name, dependencies) in &self.dependencies {
                    if !dependencies.is_disjoint(&scoped) {
                        scoped.insert(name.clone());
                    }
                }
                if scoped.len() == before {
                    return scoped;
                }
            }
        }
    }

    fn read_definitions(directory: &std::path::Path, definitions: &mut Definitions) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                read_definitions(&path, definitions);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = fs::read_to_string(&path).unwrap();
                definitions.visit_file(
                    &syn::parse_file(&source)
                        .unwrap_or_else(|error| panic!("{}: {error}", path.display())),
                );
            }
        }
    }

    fn has_scoped_json(signature: &syn::Signature, scoped: &BTreeSet<String>) -> bool {
        signature.inputs.iter().any(|argument| {
            let syn::FnArg::Typed(argument) = argument else {
                return false;
            };
            let mut types = Types::default();
            types.visit_type(&argument.ty);
            types.0.contains("Json") && !types.0.is_disjoint(scoped)
        })
    }

    // This endpoint grants membership in multiple organizations at once. It
    // checks admin access to EVERY requested organization inside its transaction
    // in cloud mode, and requires a sysadmin in self-hosted mode. Regression:
    // handler::invitations::security_tests::body_organizations_require_authority.
    const MANUAL_BODY_SCOPES: &[(&str, &str)] = &[(
        "invitations::create_handler",
        "multi-organization invitation; transactional admin check plus self-hosted sysadmin gate",
    )];

    #[test]
    fn every_body_organization_is_covered() {
        let mut definitions = Definitions::default();
        read_definitions(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut definitions,
        );
        let scoped = definitions.scoped();
        assert!(
            scoped.contains("CreateInvitationRequest"),
            "nested request discovery stopped working"
        );
        let mut seen = BTreeSet::new();
        for route in routes_in(ROUTER_SRC)
            .into_iter()
            .chain(routes_in(TERMINAL_SRC))
        {
            let (module, name) = route.handler.split_once("::").unwrap();
            let source =
                source_of(module).unwrap_or_else(|| panic!("missing source: {}", route.handler));
            let file = syn::parse_file(&source).unwrap();
            let function = file
                .items
                .iter()
                .find_map(|item| match item {
                    syn::Item::Fn(function) if function.sig.ident == name => Some(function),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing handler: {}", route.handler));
            if !has_scoped_json(&function.sig, &scoped) {
                continue;
            }
            // A path extractor is not sufficient here: independently supplied
            // body IDs also need a reviewed relationship to the authorized scope.
            assert!(
                MANUAL_BODY_SCOPES
                    .iter()
                    .any(|(handler, reason)| *handler == route.handler && !reason.is_empty()),
                "{} accepts organization IDs in its body without reviewed authorization coverage",
                route.handler
            );
            seen.insert(route.handler);
        }
        for (handler, _) in MANUAL_BODY_SCOPES {
            assert!(
                seen.contains(*handler),
                "stale body authorization exception: {handler}"
            );
        }
    }

    #[test]
    fn detects_direct_nested_optional_and_aliased_body_ids() {
        let file = syn::parse_file(
            r#"
            struct Target { organization_id: String }
            type Targets = Vec<Target>;
            struct Request { targets: Option<Targets> }
            struct Safe { message: String }
            async fn nested(Json(body): Json<Request>) {}
            async fn direct(Json(body): Json<Target>) {}
            async fn safe(Json(body): Json<Safe>) {}
        "#,
        )
        .unwrap();
        let mut definitions = Definitions::default();
        definitions.visit_file(&file);
        let scoped = definitions.scoped();
        for item in &file.items {
            if let syn::Item::Fn(function) = item {
                assert_eq!(
                    has_scoped_json(&function.sig, &scoped),
                    function.sig.ident != "safe"
                );
            }
        }
    }
}
