use std::collections::BTreeMap;

use dash_api::model::{
    ModelCustomResourceDefinitionRefSpec, ModelFieldKindSpec, ModelFieldSpec, ModelFieldsSpec,
    ModelSpec,
};
use ipis::{
    core::anyhow::{bail, Result},
    itertools::Itertools,
};
use kiss_api::{
    k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::{
        CustomResourceDefinition, CustomResourceDefinitionVersion, JSONSchemaProps,
    },
    kube::{Api, Client},
};

pub struct ModelValidator<'a> {
    pub kube: &'a Client,
}

impl<'a> ModelValidator<'a> {
    pub async fn validate(&self, spec: &ModelSpec) -> Result<ModelFieldsSpec> {
        match spec {
            ModelSpec::Fields(spec) => self.validate_fields(spec),
            ModelSpec::CustomResourceDefinitionRef(spec) => {
                self.validate_custom_resource_definition_ref(spec).await
            }
        }
    }

    fn validate_fields(&self, spec: &ModelFieldsSpec) -> Result<ModelFieldsSpec> {
        // TODO: to be implemented
        Ok(spec.clone())
    }

    async fn validate_custom_resource_definition_ref(
        &self,
        spec: &ModelCustomResourceDefinitionRefSpec,
    ) -> Result<ModelFieldsSpec> {
        let (crd_name, version) = {
            let mut attrs: Vec<_> = spec.name.split('/').collect();
            if attrs.len() != 2 {
                bail!("CRD name is invalid; expected name/version");
            }

            let version = attrs.pop().unwrap();
            let crd_name = attrs.pop().unwrap();
            (crd_name, version)
        };

        let api = Api::<CustomResourceDefinition>::all(self.kube.clone());
        let crd = api.get(crd_name).await?;

        match crd.spec.versions.iter().find(|def| def.name == version) {
            Some(def) => {
                let mut parser = ModelFieldsParser::default();
                parser.parse_custom_resource_definition(def)?;
                self.validate_fields(&parser.finalize())
            }
            None => bail!(
                "CRD version is invalid; expected one of {:?}, but given {version}",
                crd.spec.versions.iter().map(|def| &def.name).join(","),
            ),
        }
    }
}

#[derive(Debug, Default)]
struct ModelFieldsParser {
    map: BTreeMap<String, ModelFieldSpec>,
}

impl ModelFieldsParser {
    fn parse_custom_resource_definition(
        &mut self,
        def: &CustomResourceDefinitionVersion,
    ) -> Result<()> {
        match def
            .schema
            .as_ref()
            .and_then(|schema| schema.open_api_v3_schema.as_ref())
        {
            Some(prop) => self.parse_json_property(None, "", prop),
            None => Ok(()),
        }
    }

    fn parse_json_property(
        &mut self,
        parent: Option<&str>,
        name: &str,
        prop: &JSONSchemaProps,
    ) -> Result<()> {
        let (name, name_raw) = (convert_name(parent, name)?, name);
        if self.map.contains_key(&name) {
            bail!("conflicted field name: {name} ({name_raw})");
        }

        let kind = match prop.type_.as_ref().map(AsRef::as_ref) {
            Some("integer") => {
                let minimum = prop.minimum.as_ref().copied().map(|e| e.round() as i64);
                let maximum = prop.maximum.as_ref().copied().map(|e| e.round() as i64);

                let default = prop.default.as_ref().and_then(|e| e.0.as_i64()).or(minimum);

                Some(ModelFieldKindSpec::Integer {
                    default,
                    minimum,
                    maximum,
                })
            }
            Some("number") => {
                let minimum = prop.minimum;
                let maximum = prop.maximum;

                let default = prop.default.as_ref().and_then(|e| e.0.as_f64()).or(minimum);

                Some(ModelFieldKindSpec::Number {
                    default,
                    minimum,
                    maximum,
                })
            }
            Some("string") => match prop.format.as_ref().map(AsRef::as_ref) {
                Some("date-time") => Some(ModelFieldKindSpec::DateTime { default: None }),
                Some("ip") => Some(ModelFieldKindSpec::Ip {}),
                Some("uuid") => Some(ModelFieldKindSpec::Uuid {}),
                None => match &prop.enum_ {
                    Some(enum_) => {
                        let default = prop
                            .default
                            .as_ref()
                            .and_then(|e| e.0.as_str())
                            .map(ToString::to_string);
                        let choices = enum_
                            .iter()
                            .filter_map(|e| e.0.as_str())
                            .map(ToString::to_string)
                            .collect();

                        Some(ModelFieldKindSpec::OneOfStrings { default, choices })
                    }
                    None => {
                        let default = prop
                            .default
                            .as_ref()
                            .and_then(|e| e.0.as_str())
                            .map(ToString::to_string);

                        Some(ModelFieldKindSpec::String { default })
                    }
                },
                Some(format) => bail!("unknown string format of {name:?}: {format:?}"),
            },
            Some("object") => {
                if let Some(children_props) = &prop.properties {
                    let parent = Some(name.as_ref());
                    for (name, prop) in children_props {
                        self.parse_json_property(parent, name, prop)?;
                    }
                }
                None
            }
            type_ => bail!("unknown type of {name:?}: {type_:?}"),
        };

        match kind {
            Some(kind) => {
                let spec = ModelFieldSpec {
                    name: name.clone(),
                    kind,
                    nullable: prop.nullable.unwrap_or_default(),
                };

                self.map.insert(name, spec);
                Ok(())
            }
            None => Ok(()),
        }
    }

    fn finalize(self) -> ModelFieldsSpec {
        self.map.into_values().collect()
    }
}

fn convert_name(parent: Option<&str>, name: &str) -> Result<String> {
    // TODO: validate name (i.e. special charactors)
    let name = name.to_string();

    match parent {
        Some(parent) => Ok(format!("{parent}{name}/")),
        None => Ok(format!("/{name}")),
    }
}
