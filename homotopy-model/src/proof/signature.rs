use std::{collections::VecDeque, str::FromStr};

use homotopy_common::tree::{Node, Tree};
use homotopy_core::{
    diagram::NewDiagramError,
    signature::{Invertibility, Signature as _, Symmetry},
    typecheck::{typecheck, Mode},
    Diagram, Diagram0, DiagramN, Generator,
};
use homotopy_graphics::style::{Color, SignatureStyleData, VertexShape};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::proof::generators::GeneratorInfo;

pub const COLORS: &[&str] = &[
    "#2980b9", // belize blue
    "#c0392b", // pomegranate
    "#f39c12", // orange
    "#8e44ad", // wisteria
    "#27ae60", // nephritis
    "#f1c40f", // sunflower
    "#f6f5f4", // white(ish)
    "#000000", // black
];

/// The largest direction of a [`Symmetry`], which is stored as a 64-bit mask.
pub const MAX_SYMMETRY_DIRECTION: usize = 64;

pub const VERTEX_SHAPES: &[VertexShape] = &[VertexShape::Circle, VertexShape::Square];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum SignatureItem {
    Folder(FolderInfo),
    Item(GeneratorInfo),
}

impl Default for SignatureItem {
    fn default() -> Self {
        Self::Folder(FolderInfo {
            id: 0,
            name: Default::default(),
            open: true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignatureItemEdit {
    Rename(String),
    Recolor(Color),
    Reshape(VertexShape),
    MakeOriented(bool),
    MakePlaceholder(bool),
    /// Set the placeholders the generator depends on beyond those its diagram mentions.
    SetDependsOn(Vec<Generator>),
    MakeInvertible(Invertibility),
    ShowSourceTarget(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignatureEdit {
    Edit(Node, SignatureItemEdit),
    MoveBefore(Node, Node),
    MoveInto(Node, Node),
    ToggleFolder(Node),
    /// Show or hide the instances of a generator (its children in the tree).
    ToggleInstances(Node),
    NewFolder(Node),
    Remove(Node),
    /// Declare a reflection symmetry of the theory. This only makes typechecking more permissive.
    AddSymmetry(Symmetry),
    /// Remove the symmetry at the given index, if the signature remains well-typed without it.
    RemoveSymmetry(usize),
}

#[derive(Debug, Error)]
pub enum SignatureError {
    #[error("generator cannot be marked as {0} because it is being used in an incompatible way")]
    Invertibility(Invertibility),
    #[error("invalid symmetry: {0}")]
    InvalidSymmetry(&'static str),
    #[error("symmetry cannot be removed because it is being used")]
    SymmetryInUse,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Signature {
    tree: Tree<SignatureItem>,
    symmetries: Vec<Symmetry>,
}

impl Signature {
    pub fn iter(&self) -> impl Iterator<Item = &GeneratorInfo> {
        self.tree.iter().filter_map(|(_, data)| match data.inner() {
            SignatureItem::Item(info) => Some(info),
            &SignatureItem::Folder(_) => None,
        })
    }

    pub fn folder_iter(&self) -> impl Iterator<Item = &FolderInfo> {
        self.tree.iter().filter_map(|(_, data)| match data.inner() {
            SignatureItem::Folder(info) => Some(info),
            SignatureItem::Item(_) => None,
        })
    }

    #[must_use]
    pub fn has_generators(&self) -> bool {
        self.iter().next().is_some()
    }

    pub(crate) fn next_generator_id(&self) -> usize {
        self.generators().map(|g| g.id).max().map_or(0, |id| id + 1)
    }

    fn next_folder_id(&self) -> usize {
        self.folder_iter()
            .map(|info| info.id)
            .max()
            .map_or(0, |id| id + 1)
    }

    #[must_use]
    pub fn filter_map<F>(&self, f: F) -> Self
    where
        F: Fn(&GeneratorInfo) -> Option<GeneratorInfo>,
    {
        Self {
            tree: self.as_tree().filter_map(|data| {
                if let SignatureItem::Item(info) = data {
                    Some(SignatureItem::Item(f(info)?))
                } else {
                    Some(data.clone())
                }
            }),
            symmetries: self.symmetries.clone(),
        }
    }

    pub(crate) fn insert(
        &mut self,
        generator: Generator,
        diagram: impl Into<Diagram>,
        name: &str,
        invertibility: Invertibility,
    ) {
        let diagram: Diagram = diagram.into();
        let info = GeneratorInfo {
            generator,
            name: format!("{name} {}", generator.id),
            oriented: false,
            invertibility,
            single_preview: true,
            color: Color::from_str(COLORS[generator.id % COLORS.len()]).unwrap(),
            shape: Default::default(),
            diagram,
            placeholder: false,
            instance: None,
            instances_open: false,
            depends_on: vec![],
        };

        self.tree
            .push_onto(self.tree.root(), SignatureItem::Item(info));
    }

    pub fn insert_item(&mut self, item: SignatureItem) {
        self.tree.push_onto(self.tree.root(), item);
    }

    pub(crate) fn insert_item_under(&mut self, node: Node, item: SignatureItem) {
        self.tree.push_onto(node, item);
    }

    pub(crate) fn find_node(&self, generator: Generator) -> Option<Node> {
        self.tree
            .iter()
            .find_map(|(node, item)| match item.inner() {
                SignatureItem::Item(info) if info.generator == generator => Some(node),
                _ => None,
            })
    }

    pub(crate) fn find_generator(&self, node: Node) -> Option<Generator> {
        self.tree
            .with(node, |n| match n.inner() {
                SignatureItem::Item(info) => Some(info.generator),
                SignatureItem::Folder(_) => None,
            })
            .flatten()
    }

    fn edit(&mut self, node: Node, edit: SignatureItemEdit) {
        use SignatureItem::{Folder, Item};
        use SignatureItemEdit::{
            MakeInvertible, MakeOriented, MakePlaceholder, Recolor, Rename, Reshape, SetDependsOn,
            ShowSourceTarget,
        };
        self.tree
            .with_mut(node, move |n| match (n.inner_mut(), edit) {
                (Item(info), Rename(name)) => info.name = name,
                (Item(info), Recolor(color)) => info.color = color,
                (Item(info), Reshape(shape)) => info.shape = shape,
                (Item(info), MakeOriented(true)) => info.oriented = true,
                (Item(info), MakePlaceholder(placeholder)) => info.placeholder = placeholder,
                (Item(info), SetDependsOn(depends_on)) => info.depends_on = depends_on,
                (Item(info), MakeInvertible(invertibility)) => info.invertibility = invertibility,
                (Item(info), ShowSourceTarget(show)) => info.single_preview = !show,
                (Folder(info), Rename(name)) => info.name = name,
                (_, _) => {}
            });
    }

    #[must_use]
    pub fn has_descendents_in(&self, node: Node, diagram: &Diagram) -> bool {
        self.tree.descendents_of(node).any(|node| {
            self.tree
                .with(node, |n| {
                    if let SignatureItem::Item(info) = n.inner() {
                        diagram.generators().contains_key(&info.generator)
                    } else {
                        false
                    }
                })
                .unwrap_or_default()
        })
    }

    pub fn create_generator_zero(&mut self, name: &str) -> Diagram0 {
        let id = self.next_generator_id();
        let generator = Generator::new(id, 0);
        let diagram = Diagram0::from(generator);
        self.insert(generator, diagram, name, Invertibility::Directed);
        diagram
    }

    pub fn create_generator(
        &mut self,
        source: Diagram,
        target: Diagram,
        name: &str,
        invertibility: Invertibility,
    ) -> Result<DiagramN, NewDiagramError> {
        let id = self.next_generator_id();
        let generator = Generator::new(id, source.dimension() + 1);
        let diagram = DiagramN::from_generator(generator, source, target)?;
        self.insert(generator, diagram.clone(), name, invertibility);
        Ok(diagram)
    }

    /// Re-number the directions of the symmetries after the signature has been suspended, which
    /// adds a new innermost direction. Directions that no longer fit in a mask are dropped.
    pub(crate) fn suspend_symmetries(&mut self) {
        for symmetry in &mut self.symmetries {
            symmetry.directions = (symmetry.directions.iter())
                .map(|j| j + 1)
                .filter(|j| *j <= MAX_SYMMETRY_DIRECTION)
                .collect();
        }
    }

    pub fn remove(&mut self, generator: Generator) {
        if let Some(node) = self.find_node(generator) {
            self.tree.remove(node);
        }
    }

    pub fn update(&mut self, edit: &SignatureEdit) -> Result<(), SignatureError> {
        match edit {
            SignatureEdit::Edit(node, edit) => {
                self.edit(*node, edit.clone());

                // Intercept edit in order to update the whole signature.
                if matches!(edit, SignatureItemEdit::MakeOriented(true)) {
                    if let Some(generator) = self.find_generator(*node) {
                        self.tree = self.tree.clone().map(|item| match item {
                            SignatureItem::Item(info) => SignatureItem::Item(GeneratorInfo {
                                diagram: info.diagram.remove_framing(generator),
                                ..info
                            }),
                            SignatureItem::Folder(_) => item,
                        });
                    }
                }
                if let SignatureItemEdit::MakeInvertible(invertibility) = edit {
                    if let Some(generator) = self.find_generator(*node) {
                        let info = self.generator_info(generator).unwrap();
                        if *invertibility < info.invertibility {
                            let mode = Mode {
                                directed: true,
                                dualisable: true,
                                simplices: false,
                                generator: Some(generator),
                            };
                            if self
                                .iter()
                                .any(|info| typecheck(&info.diagram, self, mode, true).is_err())
                            {
                                return Err(SignatureError::Invertibility(*invertibility));
                            }
                        }
                    }
                }
            }
            SignatureEdit::NewFolder(node) => {
                self.tree.push_onto(
                    *node,
                    SignatureItem::Folder(FolderInfo {
                        id: self.next_folder_id(),
                        name: "New folder".to_owned(),
                        open: true,
                    }),
                );
            }
            SignatureEdit::MoveBefore(from, to) => {
                if *from != self.tree.root()
                    && !self.tree.descendents_of(*from).any(|node| node == *to)
                {
                    self.tree.reparent_before(*from, *to);
                }
            }
            SignatureEdit::MoveInto(from, to) => {
                if *from != self.tree.root()
                    && !self.tree.descendents_of(*from).any(|node| node == *to)
                {
                    self.tree.reparent_under(*from, *to);
                }
            }
            SignatureEdit::ToggleInstances(node) => {
                self.tree.with_mut(*node, |n| {
                    if let SignatureItem::Item(info) = n.inner_mut() {
                        info.instances_open = !info.instances_open;
                    }
                });
            }
            SignatureEdit::ToggleFolder(node) => {
                self.tree.with_mut(*node, |n| {
                    if let SignatureItem::Folder(info) = n.inner_mut() {
                        info.open = !info.open;
                    }
                });
            }
            SignatureEdit::AddSymmetry(symmetry) => {
                let mut symmetry = symmetry.clone();
                symmetry.name = symmetry.name.trim().into();
                symmetry.directions.sort_unstable();
                symmetry.directions.dedup();
                if symmetry.name.is_empty() {
                    return Err(SignatureError::InvalidSymmetry("the name is empty"));
                }
                if symmetry.directions.is_empty() {
                    return Err(SignatureError::InvalidSymmetry("there are no directions"));
                }
                if !symmetry
                    .directions
                    .iter()
                    .all(|j| (1..=MAX_SYMMETRY_DIRECTION).contains(j))
                {
                    return Err(SignatureError::InvalidSymmetry(
                        "directions must be between 1 and 64",
                    ));
                }
                if self.symmetries.iter().any(|s| s.name == symmetry.name) {
                    return Err(SignatureError::InvalidSymmetry(
                        "a symmetry with this name already exists",
                    ));
                }
                self.symmetries.push(symmetry);
            }
            SignatureEdit::RemoveSymmetry(index) => {
                if *index < self.symmetries.len() {
                    let mut signature = self.clone();
                    signature.symmetries.remove(*index);
                    // Removing a symmetry makes typechecking stricter, so every generator must
                    // still be well-typed without it.
                    let mode = Mode {
                        directed: true,
                        dualisable: true,
                        simplices: false,
                        generator: None,
                    };
                    if self
                        .iter()
                        .any(|info| typecheck(&info.diagram, &signature, mode, true).is_err())
                    {
                        return Err(SignatureError::SymmetryInUse);
                    }
                    *self = signature;
                }
            }
            SignatureEdit::Remove(node) => {
                // Prepare to remove all of the descendents of the deleted node
                let mut to_remove: VecDeque<_> = self.tree.descendents_of(*node).collect();
                // So long as we have something left to delete
                while let Some(removing) = to_remove.pop_front() {
                    let mut implied = self
                        .tree
                        .iter()
                        .filter_map(|(node, item)| {
                            // Find all of the generators in the signature
                            if let SignatureItem::Item(info) = item.inner() {
                                if self.has_descendents_in(removing, &info.diagram) {
                                    return Some(node);
                                }
                            }

                            None
                        })
                        .collect();

                    // Queue these for deletion
                    to_remove.append(&mut implied);
                    // Delete the node
                    self.tree.remove(removing);
                }
            }
        }

        Ok(())
    }

    #[must_use]
    pub fn as_tree(&self) -> Tree<SignatureItem> {
        self.tree.clone()
    }

    #[must_use]
    pub fn into_tree(self) -> Tree<SignatureItem> {
        self.tree
    }

    /// Replace the symmetries of the signature, e.g. when loading a saved file.
    #[must_use]
    pub fn with_symmetries(mut self, symmetries: Vec<Symmetry>) -> Self {
        self.symmetries = symmetries;
        self
    }
}

impl SignatureStyleData for Signature {
    type Style = GeneratorInfo;

    fn generator_style(&self, g: Generator) -> Option<&Self::Style> {
        self.generator_info(g)
    }
}

impl homotopy_core::signature::Signature for Signature {
    type Info = GeneratorInfo;

    fn generators(&self) -> impl Iterator<Item = Generator> {
        self.iter().map(|info| info.generator)
    }

    fn generator_info(&self, g: Generator) -> Option<&GeneratorInfo> {
        self.iter().find(|info| info.generator == g)
    }

    fn symmetries(&self) -> &[Symmetry] {
        &self.symmetries
    }
}

impl From<Tree<SignatureItem>> for Signature {
    fn from(tree: Tree<SignatureItem>) -> Self {
        Self {
            tree,
            symmetries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Metadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub abstr: Option<String>,
}

impl Metadata {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            title: None,
            author: None,
            abstr: None,
        }
    }

    pub fn edit(&mut self, edit: MetadataEdit) {
        match edit {
            MetadataEdit::Title(title) => self.title = Some(title),
            MetadataEdit::Author(author) => self.author = Some(author),
            MetadataEdit::Abstract(abstr) => self.abstr = Some(abstr),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetadataEdit {
    Title(String),
    Author(String),
    Abstract(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataViewMessage {
    Edit(MetadataEdit),
    //SwitchView(Mode),
    Noop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderInfo {
    pub id: usize,
    pub open: bool,
    pub name: String,
}
