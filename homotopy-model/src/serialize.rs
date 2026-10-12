use homotopy_common::tree::Tree;
use homotopy_core::{
    common::{Generator, SliceIndex},
    serialize::{Key, Store},
    signature::Invertibility,
    Diagram,
};
use homotopy_graphics::style::Color;
use im::Vector;
use obake::AnyVersion;

use super::proof::{
    generators::GeneratorInfo, FolderInfo, Metadata, Signature, SignatureItem, View, Workspace,
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
                }),
            })
        })
        .transpose()?
        .into();

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
    use homotopy_core::signature::Invertibility;

    use super::{deserialize, serialize};
    use crate::proof::Signature;

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
}
