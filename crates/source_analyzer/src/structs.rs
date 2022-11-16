use std::{collections::BTreeMap, iter::once, path::Path};

use anyhow::{anyhow, bail, Context};
use quote::{format_ident, ToTokens};
use syn::{
    punctuated::Punctuated, AngleBracketedGenericArguments, GenericArgument, PathArguments, Type,
    TypePath,
};

use crate::{expand_variables_from_path, CyclerInstances, Field, Modules, PathSegment};

#[derive(Debug, Default)]
pub struct Structs {
    pub configuration: StructHierarchy,
    pub cycler_structs: BTreeMap<String, CyclerStructs>,
}

impl Structs {
    pub fn try_from_crates_directory<P>(crates_directory: P) -> anyhow::Result<Self>
    where
        P: AsRef<Path>,
    {
        let mut structs = Self::default();

        let cycler_instances = CyclerInstances::try_from_crates_directory(&crates_directory)
            .context("Failed to get cycler instances")?;
        let modules = Modules::try_from_crates_directory(&crates_directory)
            .context("Failed to get modules")?;

        for (cycler_module, module_names) in modules.cycler_modules_to_modules.iter() {
            let cycler_structs = structs
                .cycler_structs
                .entry(cycler_module.clone())
                .or_default();
            let cycler_instances = &cycler_instances.modules_to_instances[cycler_module];

            for module_name in module_names.iter() {
                let contexts = &modules.modules[module_name].contexts;

                for field in contexts.main_outputs.iter() {
                    match field {
                        Field::MainOutput { data_type, name } => {
                            match &mut cycler_structs.main_outputs {
                                StructHierarchy::Struct { fields } => {
                                    fields.insert(
                                        name.to_string(),
                                        StructHierarchy::Field {
                                            data_type: data_type.clone(),
                                        },
                                    );
                                }
                                _ => bail!("Unexpected non-struct hierarchy in main outputs"),
                            }
                        }
                        _ => {
                            bail!("Unexpected field {field:?} in MainOutputs");
                        }
                    }
                }
                for field in contexts
                    .new_context
                    .iter()
                    .chain(contexts.cycle_context.iter())
                {
                    match field {
                        Field::AdditionalOutput {
                            data_type,
                            name,
                            path,
                        } => {
                            let expanded_paths = expand_variables_from_path(
                                path,
                                &BTreeMap::from_iter([(
                                    "cycler_instance".to_string(),
                                    cycler_instances.clone(),
                                )]),
                            )
                            .with_context(|| {
                                anyhow!("Failed to expand path variables for additional output `{name}`")
                            })?;

                            let data_type_wrapped_in_option = Type::Path(TypePath {
                                qself: None,
                                path: syn::Path {
                                    leading_colon: None,
                                    segments: Punctuated::from_iter([syn::PathSegment {
                                        ident: format_ident!("Option"),
                                        arguments: PathArguments::AngleBracketed(
                                            AngleBracketedGenericArguments {
                                                colon2_token: None,
                                                lt_token: Default::default(),
                                                args: Punctuated::from_iter([
                                                    GenericArgument::Type(data_type.clone()),
                                                ]),
                                                gt_token: Default::default(),
                                            },
                                        ),
                                    }]),
                                },
                            });
                            for path in expanded_paths {
                                let insertion_rules =
                                    path_to_insertion_rules(&path, &data_type_wrapped_in_option);
                                cycler_structs
                                    .additional_outputs
                                    .insert(insertion_rules)
                                    .with_context(|| {
                                        anyhow!("Failed to insert expanded path into additional outputs for additional output `{name}`")
                                    })?;
                            }
                        }
                        Field::Parameter {
                            data_type,
                            name,
                            path,
                        } => {
                            let expanded_paths = expand_variables_from_path(
                                path,
                                &BTreeMap::from_iter([(
                                    "cycler_instance".to_string(),
                                    cycler_instances.clone(),
                                )]),
                            )
                            .with_context(|| {
                                anyhow!("Failed to expand path variables for parameter `{name}`")
                            })?;

                            for path in expanded_paths {
                                let path_contains_optional =
                                    path.iter().any(|segment| segment.is_optional);
                                let data_type = match path_contains_optional {
                                    true => unwrap_option_data_type(data_type.clone())
                                        .with_context(|| {
                                            anyhow!("Failed to unwrap Option<T> from data type for parameter `{name}`")
                                        })?,
                                    false => data_type.clone(),
                                };
                                let insertion_rules = path_to_insertion_rules(&path, &data_type);
                                structs
                                    .configuration
                                    .insert(insertion_rules)
                                    .with_context(|| {
                                        anyhow!("Failed to insert expanded path into configuration for parameter `{name}`")
                                    })?;
                            }
                        }
                        Field::PersistentState {
                            data_type,
                            name,
                            path,
                        } => {
                            let expanded_paths = expand_variables_from_path(
                                path,
                                &BTreeMap::from_iter([(
                                    "cycler_instance".to_string(),
                                    cycler_instances.clone(),
                                )]),
                            )
                            .with_context(|| {
                                anyhow!(
                                    "Failed to expand path variables for persistent state `{name}`"
                                )
                            })?;

                            for path in expanded_paths {
                                let insertion_rules = path_to_insertion_rules(&path, data_type);
                                cycler_structs
                                    .persistent_state
                                    .insert(insertion_rules)
                                    .with_context(|| {
                                        anyhow!("Failed to insert expanded path into persistent state for persistent state `{name}`")
                                    })?;
                            }
                        }
                        Field::HardwareInterface { .. }
                        | Field::HistoricInput { .. }
                        | Field::Input { .. }
                        | Field::PerceptionInput { .. }
                        | Field::RequiredInput { .. } => {}
                        _ => {
                            bail!("Unexpected field {field:?} in `NewContext` or `CycleContext`");
                        }
                    }
                }
            }
        }

        Ok(structs)
    }
}

#[derive(Debug, Default)]
pub struct CyclerStructs {
    pub main_outputs: StructHierarchy,
    pub additional_outputs: StructHierarchy,
    pub persistent_state: StructHierarchy,
}

#[derive(Debug)]
pub enum StructHierarchy {
    Struct {
        fields: BTreeMap<String, StructHierarchy>,
    },
    Optional {
        child: Box<StructHierarchy>,
    },
    Field {
        data_type: Type,
    },
}

impl Default for StructHierarchy {
    fn default() -> Self {
        Self::Struct {
            fields: Default::default(),
        }
    }
}

impl StructHierarchy {
    fn insert(&mut self, mut insertion_rules: Vec<InsertionRule>) -> anyhow::Result<()> {
        let first_rule = match insertion_rules.first() {
            Some(first_rule) => first_rule,
            None => return Ok(()),
        };

        match self {
            StructHierarchy::Struct { fields } => match first_rule {
                InsertionRule::InsertField { name } => fields
                    .entry(name.clone())
                    .or_default()
                    .insert(insertion_rules.split_off(1)),
                InsertionRule::BeginOptional => {
                    if !fields.is_empty() {
                        bail!("Failed to begin optional in-place of non-empty struct");
                    }
                    let mut child = StructHierarchy::default();
                    child.insert(insertion_rules.split_off(1))?;
                    *self = StructHierarchy::Optional {
                        child: Box::new(child),
                    };
                    Ok(())
                }
                InsertionRule::BeginStruct => self.insert(insertion_rules.split_off(1)),
                InsertionRule::AppendDataType { data_type } => {
                    if !fields.is_empty() {
                        bail!("Failed to append data type in-place of non-empty struct");
                    }
                    *self = StructHierarchy::Field {
                        data_type: data_type.clone(),
                    };
                    Ok(())
                }
            },
            StructHierarchy::Optional { child } => match first_rule {
                InsertionRule::InsertField { name } => {
                    bail!("Failed to insert field with name `{name}` to optional")
                }
                InsertionRule::BeginOptional => child.insert(insertion_rules.split_off(1)),
                InsertionRule::BeginStruct => bail!("Failed to begin struct in-place of optional"),
                InsertionRule::AppendDataType { .. } => {
                    bail!("Failed to append data type in-place of optional")
                }
            },
            StructHierarchy::Field { data_type } => match first_rule {
                InsertionRule::InsertField { .. } => Ok(()),
                InsertionRule::BeginOptional => Ok(()),
                InsertionRule::BeginStruct => Ok(()),
                InsertionRule::AppendDataType {
                    data_type: data_type_to_be_appended,
                } => {
                    if data_type != data_type_to_be_appended {
                        bail!( // TODO: Ja, wo denn?!
                            "Unmatching data types: previous data type {} does not match data type {} to be appended",
                            data_type.to_token_stream(),
                            data_type_to_be_appended.to_token_stream(),
                        );
                    }
                    Ok(())
                }
            },
        }
    }
}

#[derive(Clone, Debug)]
enum InsertionRule {
    InsertField { name: String },
    BeginOptional,
    BeginStruct,
    AppendDataType { data_type: Type },
}

fn path_to_insertion_rules(path: &[PathSegment], data_type: &Type) -> Vec<InsertionRule> {
    path.iter()
        .map(|segment| {
            assert_eq!(segment.is_variable, false);
            match segment.is_optional {
                true => vec![
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: segment.name.clone(),
                    },
                    InsertionRule::BeginOptional,
                ],
                false => vec![
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: segment.name.clone(),
                    },
                ],
            }
        })
        .flatten()
        .chain(once(InsertionRule::AppendDataType {
            data_type: data_type.clone(),
        }))
        .collect()
}

// TODO: is this still needed?
fn unwrap_option_data_type(data_type: Type) -> anyhow::Result<Type> {
    match data_type {
        Type::Path(TypePath {
            path: syn::Path { segments, .. },
            ..
        }) if segments.len() == 1 && segments.first().unwrap().ident == "Option" => {
            match &segments.first().unwrap().arguments {
                PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. })
                    if args.len() == 1 =>
                {
                    match args.first().unwrap() {
                        GenericArgument::Type(nested_data_type) => Ok(nested_data_type.clone()),
                        _ => bail!(
                            "Unexpected generic argument, expected type argument in data type"
                        ),
                    }
                }
                _ => bail!("Expected exactly one generic type argument in data type"),
            }
        }
        _ => bail!("Execpted Option<T> as data type"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_expand_to_correct_insertion_rules() {
        let data_type = Type::Verbatim(Default::default());
        let cases = [
            (
                "a/b/c",
                vec![
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "a".to_string(),
                    },
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "b".to_string(),
                    },
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "c".to_string(),
                    },
                    InsertionRule::AppendDataType {
                        data_type: data_type.clone(),
                    },
                ],
            ),
            (
                "a?/b/c",
                vec![
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "a".to_string(),
                    },
                    InsertionRule::BeginOptional,
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "b".to_string(),
                    },
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "c".to_string(),
                    },
                    InsertionRule::AppendDataType {
                        data_type: data_type.clone(),
                    },
                ],
            ),
            (
                "a?/b?/c",
                vec![
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "a".to_string(),
                    },
                    InsertionRule::BeginOptional,
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "b".to_string(),
                    },
                    InsertionRule::BeginOptional,
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "c".to_string(),
                    },
                    InsertionRule::AppendDataType {
                        data_type: data_type.clone(),
                    },
                ],
            ),
            (
                "a?/b?/c?",
                vec![
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "a".to_string(),
                    },
                    InsertionRule::BeginOptional,
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "b".to_string(),
                    },
                    InsertionRule::BeginOptional,
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "c".to_string(),
                    },
                    InsertionRule::BeginOptional,
                    InsertionRule::AppendDataType {
                        data_type: data_type.clone(),
                    },
                ],
            ),
            (
                "a/b?/c?",
                vec![
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "a".to_string(),
                    },
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "b".to_string(),
                    },
                    InsertionRule::BeginOptional,
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "c".to_string(),
                    },
                    InsertionRule::BeginOptional,
                    InsertionRule::AppendDataType {
                        data_type: data_type.clone(),
                    },
                ],
            ),
            (
                "a/b/c?",
                vec![
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "a".to_string(),
                    },
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "b".to_string(),
                    },
                    InsertionRule::BeginStruct,
                    InsertionRule::InsertField {
                        name: "c".to_string(),
                    },
                    InsertionRule::BeginOptional,
                    InsertionRule::AppendDataType {
                        data_type: data_type.clone(),
                    },
                ],
            ),
        ];

        for case in cases {
            let path = case.0;
            let path_segments: Vec<_> = path.split('/').map(PathSegment::from).collect();
            let insertion_rules = path_to_insertion_rules(&path_segments, &data_type);
            let expected_insertion_rules = case.1;

            assert_eq!(insertion_rules.len(), expected_insertion_rules.len(), "path: {path:?}, insertion_rules: {insertion_rules:?}, expected_insertion_rules: {expected_insertion_rules:?}");
            for (insertion_rule, expected_insertion_rule) in insertion_rules
                .into_iter()
                .zip(expected_insertion_rules.into_iter())
            {
                match (&insertion_rule, &expected_insertion_rule) {
                    (InsertionRule::InsertField { name }, InsertionRule::InsertField { name: expected_name }) if name == expected_name => {},
                    (InsertionRule::BeginOptional, InsertionRule::BeginOptional) => {},
                    (InsertionRule::BeginStruct, InsertionRule::BeginStruct) => {},
                    (InsertionRule::AppendDataType { data_type }, InsertionRule::AppendDataType { data_type: expected_data_type }) if data_type == expected_data_type => {},
                    _ => panic!("Insertion rule does not match expected: insertion_rule = {insertion_rule:?}, expected_insertion_rule = {expected_insertion_rule:?}"),
                }
            }
        }
    }

    #[test]
    fn insertion_rules_without_optionals_result_in_correct_struct_hierarchy() {
        let data_type = Type::Verbatim(Default::default());
        let insertion_rules = vec![
            InsertionRule::BeginStruct,
            InsertionRule::InsertField {
                name: "a".to_string(),
            },
            InsertionRule::BeginStruct,
            InsertionRule::InsertField {
                name: "b".to_string(),
            },
            InsertionRule::BeginStruct,
            InsertionRule::InsertField {
                name: "c".to_string(),
            },
            InsertionRule::AppendDataType {
                data_type: data_type.clone(),
            },
        ];
        let mut hierarchy = StructHierarchy::default();
        hierarchy.insert(insertion_rules).unwrap();

        assert!(
            match &hierarchy {
                StructHierarchy::Struct { fields }
                    if fields.len() == 1
                        && match fields.get(&"a".to_string()) {
                            Some(a) => match a {
                                StructHierarchy::Struct { fields }
                                    if fields.len() == 1
                                        && match fields.get(&"b".to_string()) {
                                            Some(b) => match b {
                                                StructHierarchy::Struct { fields }
                                                    if fields.len() == 1
                                                        && match fields.get(&"c".to_string()) {
                                                            Some(c) => match c {
                                                                StructHierarchy::Field {
                                                                    data_type: matched_data_type,
                                                                } if &data_type
                                                                    == matched_data_type =>
                                                                {
                                                                    true
                                                                }
                                                                _ => false,
                                                            },
                                                            None => false,
                                                        } =>
                                                    true,
                                                _ => false,
                                            },
                                            None => false,
                                        } =>
                                    true,
                                _ => false,
                            },
                            None => false,
                        } =>
                    true,
                _ => false,
            },
            "{hierarchy:?} does not match"
        );
    }

    #[test]
    fn insertion_rules_with_one_optional_result_in_correct_struct_hierarchy() {
        let data_type = Type::Verbatim(Default::default());
        let insertion_rules = vec![
            InsertionRule::BeginStruct,
            InsertionRule::InsertField {
                name: "a".to_string(),
            },
            InsertionRule::BeginOptional,
            InsertionRule::BeginStruct,
            InsertionRule::InsertField {
                name: "b".to_string(),
            },
            InsertionRule::BeginStruct,
            InsertionRule::InsertField {
                name: "c".to_string(),
            },
            InsertionRule::AppendDataType {
                data_type: data_type.clone(),
            },
        ];
        let mut hierarchy = StructHierarchy::default();
        hierarchy.insert(insertion_rules).unwrap();

        assert!(
            match &hierarchy {
                StructHierarchy::Struct { fields }
                    if fields.len() == 1
                        && match fields.get(&"a".to_string()) {
                            Some(a) => match a {
                                StructHierarchy::Optional { child } => match &**child {
                                    StructHierarchy::Struct { fields }
                                        if fields.len() == 1
                                            && match fields.get(&"b".to_string()) {
                                                Some(b) => match b {
                                                    StructHierarchy::Struct { fields }
                                                        if fields.len() == 1
                                                            && match fields.get(&"c".to_string()) {
                                                                Some(c) => match c {
                                                                    StructHierarchy::Field {
                                                                        data_type: matched_data_type,
                                                                    } if &data_type
                                                                        == matched_data_type =>
                                                                    {
                                                                        true
                                                                    }
                                                                    _ => false,
                                                                },
                                                                None => false,
                                                            } =>
                                                        true,
                                                    _ => false,
                                                },
                                                None => false,
                                            } =>
                                        true,
                                    _ => false,
                                },
                                _ => false,
                            },
                            None => false,
                        } =>
                    true,
                _ => false,
            },
            "{hierarchy:?} does not match"
        );
    }

    #[test]
    fn insertion_rules_with_two_optionals_result_in_correct_struct_hierarchy() {
        let data_type = Type::Verbatim(Default::default());
        let insertion_rules = vec![
            InsertionRule::BeginStruct,
            InsertionRule::InsertField {
                name: "a".to_string(),
            },
            InsertionRule::BeginOptional,
            InsertionRule::BeginStruct,
            InsertionRule::InsertField {
                name: "b".to_string(),
            },
            InsertionRule::BeginOptional,
            InsertionRule::BeginStruct,
            InsertionRule::InsertField {
                name: "c".to_string(),
            },
            InsertionRule::AppendDataType {
                data_type: data_type.clone(),
            },
        ];
        let mut hierarchy = StructHierarchy::default();
        hierarchy.insert(insertion_rules).unwrap();

        assert!(
            match &hierarchy {
                StructHierarchy::Struct { fields }
                    if fields.len() == 1
                        && match fields.get(&"a".to_string()) {
                            Some(a) => match a {
                                StructHierarchy::Optional { child } => match &**child {
                                    StructHierarchy::Struct { fields }
                                        if fields.len() == 1
                                            && match fields.get(&"b".to_string()) {
                                                Some(b) => match b {
                                                    StructHierarchy::Optional { child } => match &**child {
                                                        StructHierarchy::Struct { fields }
                                                            if fields.len() == 1
                                                                && match fields.get(&"c".to_string()) {
                                                                    Some(c) => match c {
                                                                        StructHierarchy::Field {
                                                                            data_type: matched_data_type,
                                                                        } if &data_type
                                                                            == matched_data_type =>
                                                                        {
                                                                            true
                                                                        }
                                                                        _ => false,
                                                                    },
                                                                    None => false,
                                                                } =>
                                                            true,
                                                        _ => false,
                                                    },
                                                    _ => false,
                                                },
                                                None => false,
                                            } =>
                                        true,
                                    _ => false,
                                },
                                _ => false,
                            },
                            None => false,
                        } =>
                    true,
                _ => false,
            },
            "{hierarchy:?} does not match"
        );
    }

    #[test]
    fn insertion_rules_with_three_optionals_result_in_correct_struct_hierarchy() {
        let data_type = Type::Verbatim(Default::default());
        let insertion_rules = vec![
            InsertionRule::BeginStruct,
            InsertionRule::InsertField {
                name: "a".to_string(),
            },
            InsertionRule::BeginOptional,
            InsertionRule::BeginStruct,
            InsertionRule::InsertField {
                name: "b".to_string(),
            },
            InsertionRule::BeginOptional,
            InsertionRule::BeginStruct,
            InsertionRule::InsertField {
                name: "c".to_string(),
            },
            InsertionRule::BeginOptional,
            InsertionRule::AppendDataType {
                data_type: data_type.clone(),
            },
        ];
        let mut hierarchy = StructHierarchy::default();
        hierarchy.insert(insertion_rules).unwrap();

        assert!(
            match &hierarchy {
                StructHierarchy::Struct { fields }
                    if fields.len() == 1
                        && match fields.get(&"a".to_string()) {
                            Some(a) => match a {
                                StructHierarchy::Optional { child } => match &**child {
                                    StructHierarchy::Struct { fields }
                                        if fields.len() == 1
                                            && match fields.get(&"b".to_string()) {
                                                Some(b) => match b {
                                                    StructHierarchy::Optional { child } => match &**child {
                                                        StructHierarchy::Struct { fields }
                                                            if fields.len() == 1
                                                                && match fields.get(&"c".to_string()) {
                                                                    Some(c) => match c {
                                                                        StructHierarchy::Optional { child } => match &**child {
                                                                            StructHierarchy::Field {
                                                                                data_type: matched_data_type,
                                                                            } if &data_type
                                                                                == matched_data_type =>
                                                                            {
                                                                                true
                                                                            }
                                                                            _ => false,
                                                                        },
                                                                        _ => false,
                                                                    },
                                                                    None => false,
                                                                } =>
                                                            true,
                                                        _ => false,
                                                    },
                                                    _ => false,
                                                },
                                                None => false,
                                            } =>
                                        true,
                                    _ => false,
                                },
                                _ => false,
                            },
                            None => false,
                        } =>
                    true,
                _ => false,
            },
            "{hierarchy:?} does not match"
        );
    }
}
