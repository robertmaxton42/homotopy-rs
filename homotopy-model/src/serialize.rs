use homotopy_common::tree::Tree;
use homotopy_core::{
    common::{Generator, SliceIndex},
    serialize::{Key, Store},
    signature::{Invertibility, Signature as _, Symmetry},
    Diagram,
};
use homotopy_graphics::style::Color;
use im::Vector;
use obake::AnyVersion;

use super::proof::{
    generators::{GeneratorInfo, Instance},
    FolderInfo, Metadata, Signature, SignatureItem, View, Workspace,
};

#[obake::versioned]
#[obake(version("0.1.2"))]
#[obake(derive(serde::Serialize, serde::Deserialize))]
#[obake(serde(untagged))]
#[derive(Debug, PartialEq, Eq, Clone, serde::Serialize, serde::Deserialize)]
struct WorkspaceData {
    diagram: Key<Diagram>,
    path: Vector<SliceIndex>,
    view: View,
}

#[obake::versioned]
#[obake(version("0.1.3"))]
#[obake(derive(serde::Serialize, serde::Deserialize))]
#[obake(serde(tag = "version"))]
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Data {
    store: Store,
    signature: Tree<SignatureData>,
    workspace: Option<WorkspaceData>,
    metadata: Metadata,
    /// Reflection symmetries of the theory. Absent in files saved before symmetries were
    /// introduced, and omitted when there are none so that older versions, which ignore unknown
    /// fields, can still load files that do not use them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    symmetries: Vec<Symmetry>,
}

impl std::fmt::Debug for Data {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Data").finish()
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum SignatureData {
    Folder(String, bool),
    Item(GeneratorData),
}

impl Default for SignatureData {
    fn default() -> Self {
        Self::Folder(Default::default(), true)
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct GeneratorData {
    generator: Generator,
    name: String,
    color: Color,
    oriented: bool,
    invertible: bool,
    /// `Some(k)` for k-dualisable generators (which also have `invertible` set, so that older
    /// versions load them as invertible). Absent in files saved before duals were introduced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dualisable: Option<usize>,
    diagram: Key<Diagram>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    placeholder: bool,
    /// Older versions ignore this, and treat instances as ordinary generators.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    instance: Option<InstanceData>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct InstanceData {
    schema: Generator,
    substitution: Vec<(Generator, Key<Diagram>)>,
}

#[must_use]
pub fn serialize(
    signature: Signature,
    workspace: Option<Workspace>,
    metadata: Metadata,
) -> Vec<u8> {
    let mut data = Data {
        store: Default::default(),
        signature: Default::default(),
        workspace: Default::default(),
        metadata,
        symmetries: signature.symmetries().to_vec(),
    };

    let mut signature = signature.into_tree();
    // Remove noise from signature tree
    signature.clean_up();
    // Pack signature data
    data.signature = signature.map(|item| match item {
        SignatureItem::Folder(info) => SignatureData::Folder(info.name, info.open),
        SignatureItem::Item(info) => SignatureData::Item(GeneratorData {
            generator: info.generator,
            diagram: data.store.pack_diagram(&info.diagram),
            name: info.name,
            color: info.color,
            oriented: info.oriented,
            invertible: info.invertibility.is_invertible(),
            dualisable: match info.invertibility {
                Invertibility::Dualisable(k) => Some(k),
                _ => None,
            },
            placeholder: info.placeholder,
            instance: info.instance.map(|instance| InstanceData {
                schema: instance.schema,
                substitution: instance
                    .substitution
                    .iter()
                    .map(|(g, d)| (*g, data.store.pack_diagram(d)))
                    .collect(),
            }),
        }),
    });

    if let Some(workspace) = workspace {
        data.workspace = Some(WorkspaceData {
            diagram: data.store.pack_diagram(&workspace.diagram),
            path: workspace.path,
            view: workspace.view,
        });
    }

    // Tag data with version
    let data: AnyVersion<Data> = data.into();
    // Serialize
    rmp_serde::encode::to_vec_named(&data).unwrap()
}

pub fn deserialize(data: &[u8]) -> Option<((Signature, Option<Workspace>), Metadata)> {
    // Deserialize with version tag
    let data: AnyVersion<Data> = match rmp_serde::decode::from_slice(data) {
        Err(error) => {
            tracing::error!("Error while deserializing: {}", error);
            None
        }
        Ok(data) => Some(data),
    }?;
    // Migrate to current version
    let data: Data = data.into();
    let mut store = data.store;

    let mut folder_index = 0;
    let signature = data
        .signature
        .map(|s| {
            Some(match s {
                SignatureData::Folder(name, open) => {
                    folder_index += 1;
                    SignatureItem::Folder(FolderInfo {
                        id: folder_index,
                        name,
                        open: open.to_owned(),
                    })
                }
                SignatureData::Item(gd) => SignatureItem::Item(GeneratorInfo {
                    generator: gd.generator,
                    name: gd.name,
                    color: gd.color,
                    shape: Default::default(),
                    diagram: store.unpack_diagram(gd.diagram)?,
                    oriented: gd.oriented,
                    invertibility: gd
                        .dualisable
                        .map_or_else(|| gd.invertible.into(), Invertibility::Dualisable),
                    // TODO: `single_preview` should be properly serialized
                    single_preview: true,
                    placeholder: gd.placeholder,
                    instance: match gd.instance {
                        None => None,
                        Some(instance) => Some(Instance {
                            schema: instance.schema,
                            substitution: instance
                                .substitution
                                .into_iter()
                                .map(|(g, d)| Some((g, store.unpack_diagram(d)?)))
                                .collect::<Option<_>>()?,
                        }),
                    },
                    instances_open: false,
                }),
            })
        })
        .transpose()?;
    let signature = Signature::from(signature).with_symmetries(data.symmetries);

    let workspace = match data.workspace {
        Some(workspace_data) => Some(Workspace {
            diagram: store.unpack_diagram(workspace_data.diagram)?,
            path: workspace_data.path,
            view: workspace_data.view,
        }),
        None => None,
    };
    Some(((signature, workspace), data.metadata))
}

#[cfg(test)]
mod tests {
    use homotopy_core::signature::{Invertibility, Signature as _, Symmetry};
    use serde::de::IgnoredAny;

    use super::{deserialize, serialize};
    use crate::proof::{Signature, SignatureEdit};

    #[test]
    fn invertibility_roundtrip() {
        let mut signature = Signature::default();
        let x = signature.create_generator_zero("x");
        let invertibilities = [
            Invertibility::Directed,
            Invertibility::Invertible,
            Invertibility::Dualisable(0),
            Invertibility::Dualisable(2),
        ];
        for (i, &invertibility) in invertibilities.iter().enumerate() {
            signature
                .create_generator(x.into(), x.into(), &format!("f{i}"), invertibility)
                .unwrap();
        }

        let data = serialize(signature.clone(), None, Default::default());
        let ((loaded, _), _) = deserialize(&data).unwrap();

        let invertibility = |s: &Signature| s.iter().map(|g| g.invertibility).collect::<Vec<_>>();
        assert_eq!(invertibility(&loaded), invertibility(&signature));
    }

    fn symmetry(name: &str, directions: &[usize]) -> Symmetry {
        Symmetry {
            name: name.to_owned(),
            directions: directions.to_vec(),
        }
    }

    #[test]
    fn symmetries_roundtrip() {
        let mut signature = Signature::default();
        signature.create_generator_zero("x");
        let symmetries = [symmetry("op", &[1, 3]), symmetry("co", &[2])];
        for s in &symmetries {
            signature
                .update(&SignatureEdit::AddSymmetry(s.clone()))
                .unwrap();
        }

        let data = serialize(signature.clone(), None, Default::default());
        let ((loaded, _), _) = deserialize(&data).unwrap();

        assert_eq!(loaded.symmetries(), symmetries);
    }

    #[test]
    fn missing_symmetries_deserialize() {
        let mut signature = Signature::default();
        signature.create_generator_zero("x");

        // Without symmetries the field is omitted entirely, as in files from older versions.
        let data = serialize(signature, None, Default::default());
        assert!(!String::from_utf8_lossy(&data).contains("symmetries"));
        let ((loaded, _), _) = deserialize(&data).unwrap();
        assert!(loaded.symmetries().is_empty());
        assert!(loaded.has_generators());
    }

    #[test]
    fn symmetries_are_ignored_by_older_versions() {
        /// The shape of `Data` before symmetries were introduced.
        #[derive(serde::Deserialize)]
        #[serde(tag = "version")]
        #[allow(dead_code)]
        enum OldData {
            #[serde(rename = "Data_v0_1_3")]
            V0_1_3 {
                store: IgnoredAny,
                signature: IgnoredAny,
                workspace: IgnoredAny,
                metadata: IgnoredAny,
            },
        }

        let mut signature = Signature::default();
        signature.create_generator_zero("x");
        signature
            .update(&SignatureEdit::AddSymmetry(symmetry("op", &[1])))
            .unwrap();

        let data = serialize(signature, None, Default::default());
        rmp_serde::decode::from_slice::<OldData>(&data).unwrap();
    }

    #[test]
    fn placeholders_and_instances_roundtrip() {
        use homotopy_core::Diagram;

        use crate::proof::SignatureItemEdit;

        let mut signature = Signature::default();
        let x = signature.create_generator_zero("x");
        let p = signature
            .create_generator(x.into(), x.into(), "P", Invertibility::Directed)
            .unwrap();
        let q = signature
            .create_generator(x.into(), x.into(), "Q", Invertibility::Directed)
            .unwrap();
        let e = signature
            .create_generator(
                p.clone().into(),
                p.clone().into(),
                "e",
                Invertibility::Directed,
            )
            .unwrap();
        let (p, e) = (p.max_generator().generator, e.max_generator().generator);
        signature
            .update(&SignatureEdit::Edit(
                signature.find_node(p).unwrap(),
                SignatureItemEdit::MakePlaceholder(true),
            ))
            .unwrap();
        let instance = signature.instantiate(e, &[(p, Diagram::from(q))]).unwrap();

        let data = serialize(signature.clone(), None, Default::default());
        let ((loaded, _), _) = deserialize(&data).unwrap();

        assert!(loaded.generator_info(p).unwrap().placeholder);
        assert_eq!(
            loaded.generator_info(instance).unwrap().instance,
            signature.generator_info(instance).unwrap().instance
        );
        // The instance is still listed under its schema.
        let tree = loaded.as_tree();
        let parent = tree
            .get(loaded.find_node(instance).unwrap())
            .unwrap()
            .parent();
        assert_eq!(parent, loaded.find_node(e));
    }
}
