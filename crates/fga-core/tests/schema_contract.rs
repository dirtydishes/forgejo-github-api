use async_graphql::parser::{
    parse_schema,
    types::{TypeDefinition, TypeKind, TypeSystemDefinition},
};
use std::collections::{BTreeMap, BTreeSet};

fn types(sdl: &str) -> BTreeMap<String, TypeDefinition> {
    parse_schema(sdl)
        .unwrap()
        .definitions
        .into_iter()
        .filter_map(|definition| match definition {
            TypeSystemDefinition::Type(ty) => Some((ty.node.name.to_string(), ty.node)),
            _ => None,
        })
        .collect()
}

#[test]
fn implemented_sdl_is_a_subset_of_pinned_github_schema() {
    let reference = types(include_str!("../../../fixtures/github-g1.graphql"));
    let implemented = types(&fga_core::graphql::schema().sdl());
    for (name, actual) in &implemented {
        if matches!(name.as_str(), "Int" | "String" | "Boolean" | "ID") {
            continue;
        }
        let expected = reference
            .get(name)
            .unwrap_or_else(|| panic!("unknown type {name}"));
        match (&actual.kind, &expected.kind) {
            (TypeKind::Scalar, TypeKind::Scalar) => {}
            (TypeKind::Object(actual), TypeKind::Object(expected)) => {
                let fields = |object: &async_graphql::parser::types::ObjectType| {
                    object
                        .fields
                        .iter()
                        .map(|f| f.node.name.to_string())
                        .collect::<BTreeSet<_>>()
                };
                assert_eq!(fields(actual), fields(expected), "{name} G1 fields");
                for field in &actual.fields {
                    let expected = expected
                        .fields
                        .iter()
                        .find(|f| f.node.name == field.node.name)
                        .unwrap_or_else(|| panic!("unknown field {name}.{}", field.node.name));
                    assert_eq!(
                        field.node.ty.node, expected.node.ty.node,
                        "{name}.{} type/nullability",
                        field.node.name
                    );
                    for arg in &field.node.arguments {
                        let expected = expected
                            .node
                            .arguments
                            .iter()
                            .find(|a| a.node.name == arg.node.name)
                            .expect("known argument");
                        assert_eq!(
                            arg.node.ty.node, expected.node.ty.node,
                            "argument type/nullability"
                        );
                        assert_eq!(
                            arg.node.default_value.as_ref().map(|v| &v.node),
                            expected.node.default_value.as_ref().map(|v| &v.node),
                            "argument default"
                        );
                    }
                    for arg in &expected.node.arguments {
                        if !arg.node.ty.node.nullable && arg.node.default_value.is_none() {
                            assert!(
                                field
                                    .node
                                    .arguments
                                    .iter()
                                    .any(|a| a.node.name == arg.node.name),
                                "missing required argument"
                            );
                        }
                    }
                }
            }
            (TypeKind::Enum(actual), TypeKind::Enum(expected)) => {
                let names = |values: &async_graphql::parser::types::EnumType| {
                    values
                        .values
                        .iter()
                        .map(|v| v.node.value.to_string())
                        .collect::<BTreeSet<_>>()
                };
                assert_eq!(names(actual), names(expected), "{name} enum values");
            }
            _ => panic!("{name} type kind differs"),
        }
    }
    for name in reference.keys() {
        assert!(implemented.contains_key(name), "missing G1 type {name}");
    }
}
