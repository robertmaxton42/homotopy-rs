use std::{collections::VecDeque, str::FromStr};

use homotopy_common::{
    hash::{FastHashMap, FastHashSet},
    tree::{Node, Tree},
};
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

/// How many diagrams' generators are remembered (see [`mentioned_by`]).
const MENTIONS_REMEMBERED: usize = 0x4000;

thread_local! {
    #[allow(clippy::mutable_key_type)] // `Diagram` is hash-consed; its caches are not hashed.
    static MENTIONS: std::cell::RefCell<FastHashMap<Diagram, std::rc::Rc<[Generator]>>> =
        std::cell::RefCell::new(FastHashMap::default());
}

/// The generators a diagram has cells of, and whether they had to be worked out (the answers are
/// remembered by the diagram, which is hash-consed: the same diagram anywhere in the signature
/// is the same key, and a rename or any other change that leaves the diagrams alone costs
/// nothing). Working them out visits all of the diagram.
#[allow(clippy::mutable_key_type)]
fn mentioned_by(diagram: &Diagram) -> (std::rc::Rc<[Generator]>, bool) {
    MENTIONS.with(|memo| {
        if let Some(found) = memo.borrow().get(diagram) {
            return (found.clone(), false);
        }
        let found: std::rc::Rc<[Generator]> = diagram.generators().into_keys().collect();
        let mut memo = memo.borrow_mut();
        if memo.len() >= MENTIONS_REMEMBERED {
            memo.clear();
        }
        memo.insert(diagram.clone(), found.clone());
        (found, true)
    })
}

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

impl SignatureEdit {
    /// Whether the edit only changes what is shown (whether a folder or the instances of a
    /// generator are open), and not the project: it is not a step of the history (undoing does
    /// not undo it), and a job that works on a copy of the project is not in its way.
    #[must_use]
    pub const fn is_view_only(&self) -> bool {
        matches!(self, Self::ToggleFolder(_) | Self::ToggleInstances(_))
    }
}

#[derive(Debug, Error)]
pub enum SignatureError {
    #[error("generator cannot be marked as {0} because it is being used in an incompatible way")]
    Invertibility(Invertibility),
    #[error(
        "{placeholder} cannot be marked as {level}: it is replaced with a diagram that is not, in \
         the instance {instance}"
    )]
    InvertibilityOfReplaced {
        placeholder: String,
        level: String,
        instance: String,
    },
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

    #[must_use]
    pub fn find_node(&self, generator: Generator) -> Option<Node> {
        self.tree
            .iter()
            .find_map(|(node, item)| match item.inner() {
                SignatureItem::Item(info) if info.generator == generator => Some(node),
                _ => None,
            })
    }

    #[must_use]
    pub fn find_generator(&self, node: Node) -> Option<Generator> {
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

    /// Whether the placeholder at a node can be made invertible or dualisable to `level`: not if
    /// an instance of a schema was made with a replacement for it that is not as invertible
    /// (see [`check_invertibility`](Self::check_invertibility)).
    fn check_replaced_as_invertible(
        &self,
        node: Node,
        level: Invertibility,
    ) -> Result<(), SignatureError> {
        let Some(generator) = self.find_generator(node) else {
            return Ok(());
        };
        let Some(info) = self.generator_info(generator) else {
            return Ok(());
        };
        if generator.dimension == 0 || level <= info.invertibility {
            return Ok(());
        }
        for instance in self.iter() {
            let Some(made) = &instance.instance else {
                continue;
            };
            for (p, replacement) in &made.substitution {
                if *p == generator && replacement.falls_short_of(level, self).is_some() {
                    return Err(SignatureError::InvertibilityOfReplaced {
                        placeholder: info.name.clone(),
                        level: level.describe(),
                        instance: instance.name.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Every node that removing `node` removes, `node` and all its descendants first (a
    /// folder's contents, a generator's instances) and then what the removal forces, in the
    /// order it is found. This is the one definition of the cascade: [`SignatureEdit::Remove`]
    /// removes exactly these nodes, and the preview of a deletion
    /// ([`removal_cascade`](Self::removal_cascade)) is made from them.
    ///
    /// A generator is removed with another when
    /// - its diagram mentions it (or any generator removed with it, such as its instances),
    /// - it is an instance whose substitution mentions it, as a placeholder that was replaced or
    ///   in a replacement (an instance of a schema made with a replacement that mentions a
    ///   placeholder depends on that placeholder, see
    ///   [`instance_parameters`](Self::instance_parameters)), or
    /// - it lists it as an explicit dependency ([`GeneratorInfo::depends_on`]) **and it is a
    ///   placeholder**. Explicit dependencies mean "there always exists a cell for each
    ///   replacement of this placeholder", which a deleted placeholder cannot have. Nothing else
    ///   can be depended on: the dependencies in the settings are placeholders, and for a
    ///   generator that is not a placeholder (an ordinary generator, an instance) the entry in
    ///   the list is not looked at by [`parameters`](Self::parameters). So deleting an instance
    ///   removes only what mentions it, not what lists it as a dependency. (Instances of
    ///   *templates*, which are not made yet, would cascade, since "there always exists" would
    ///   no longer be true of an instance that is gone.)
    ///
    /// The cost is one pass over the diagrams of the signature, whatever is removed (the
    /// generators of each diagram are remembered, see [`removal_preview`](Self::removal_preview)).
    #[must_use]
    pub fn removal_nodes(&self, node: Node) -> Vec<Node> {
        let dependents = self.dependents_index(None).unwrap_or_default();
        self.cascade(&dependents, node)
    }

    /// [`removal_nodes`](Self::removal_nodes) for showing before the deletion, which may not
    /// take long however large the signature: finding out which generators a diagram mentions
    /// is what costs, so at most `budget` diagrams that have not been looked at yet (the answer
    /// is remembered, so that it is not asked again, and survives renaming and the like) are
    /// looked at, and if there are more the answer is `None`; asking again continues where this
    /// left off, and ends. The nodes are exactly those of `removal_nodes`.
    #[must_use]
    pub fn removal_preview(&self, node: Node, budget: usize) -> Option<Vec<Node>> {
        let dependents = self.dependents_index(Some(budget))?;
        Some(self.cascade(&dependents, node))
    }

    fn cascade(&self, dependents: &FastHashMap<Generator, Vec<Node>>, node: Node) -> Vec<Node> {
        let mut removed = FastHashSet::default();
        let mut order = vec![];
        let mut queue = VecDeque::from([node]);
        while let Some(root) = queue.pop_front() {
            for removing in self.tree.descendents_of(root) {
                if !removed.insert(removing) {
                    continue;
                }
                order.push(removing);
                if let Some(generator) = self.find_generator(removing) {
                    if let Some(implied) = dependents.get(&generator) {
                        queue.extend(implied.iter().copied());
                    }
                }
            }
        }
        order
    }

    /// The generators other than `generator` that deleting it would delete: the instances of
    /// it, what mentions it or them, and what lists it as an explicit dependency if it is a
    /// placeholder, transitively (see [`removal_nodes`](Self::removal_nodes), which this is
    /// made from, so that it can never disagree with the deletion). In the order the cascade
    /// finds them. Empty for a generator that is not in the signature.
    #[must_use]
    pub fn removal_cascade(&self, generator: Generator) -> Vec<Generator> {
        let Some(node) = self.find_node(generator) else {
            return vec![];
        };
        self.removal_nodes(node)
            .into_iter()
            .filter_map(|n| self.find_generator(n))
            .filter(|&g| g != generator)
            .collect()
    }

    /// The generators that removing `node` removes, itself included, as a set.
    #[must_use]
    pub fn removal_generators(&self, node: Node) -> FastHashSet<Generator> {
        self.removal_nodes(node)
            .into_iter()
            .filter_map(|n| self.find_generator(n))
            .collect()
    }

    /// For each generator, the nodes whose removal it forces: those that mention it in their
    /// diagram or in the substitution of an instance, or that list it as an explicit dependency
    /// when it is a placeholder.
    ///
    /// With a `budget`, `None` if more than that many diagrams have to be looked at that were not
    /// before.
    fn dependents_index(&self, budget: Option<usize>) -> Option<FastHashMap<Generator, Vec<Node>>> {
        let mut looked_at = 0;
        let placeholders: FastHashSet<Generator> = self
            .iter()
            .filter(|info| info.placeholder && info.instance.is_none())
            .map(|info| info.generator)
            .collect();
        let mut index: FastHashMap<Generator, Vec<Node>> = FastHashMap::default();
        for (node, data) in self.tree.iter() {
            let SignatureItem::Item(info) = data.inner() else {
                continue;
            };
            let mut mentions: FastHashSet<Generator> = FastHashSet::default();
            let mut mention = |diagram: &Diagram, mentions: &mut FastHashSet<Generator>| {
                let (found, fresh) = mentioned_by(diagram);
                looked_at += usize::from(fresh);
                mentions.extend(found.iter().copied());
            };
            mention(&info.diagram, &mut mentions);
            mentions.extend(
                info.depends_on
                    .iter()
                    .filter(|g| placeholders.contains(g))
                    .copied(),
            );
            if let Some(made) = &info.instance {
                for (placeholder, replacement) in &made.substitution {
                    mentions.insert(*placeholder);
                    mention(replacement, &mut mentions);
                }
            }
            if budget.is_some_and(|budget| looked_at > budget) {
                return None;
            }
            mentions.remove(&info.generator);
            for g in mentions {
                index.entry(g).or_default().push(node);
            }
        }
        Some(index)
    }

    /// Drops the explicit dependencies on generators that are no longer in the signature, which
    /// would otherwise stay in the list (an instance can be deleted with no cascade) and be
    /// taken for a new generator that is given the same number.
    fn forget_missing_dependencies(&mut self) {
        let present: FastHashSet<Generator> = self.iter().map(|info| info.generator).collect();
        let stale: Vec<Node> = self
            .tree
            .iter()
            .filter_map(|(node, data)| match data.inner() {
                SignatureItem::Item(info)
                    if info.depends_on.iter().any(|g| !present.contains(g)) =>
                {
                    Some(node)
                }
                _ => None,
            })
            .collect();
        for node in stale {
            self.tree.with_mut(node, |n| {
                if let SignatureItem::Item(info) = n.inner_mut() {
                    info.depends_on.retain(|g| present.contains(g));
                }
            });
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
                // A placeholder that is made more invertible than the diagrams it was replaced
                // with are would make those instances wrong.
                if let SignatureItemEdit::MakeInvertible(level) = edit {
                    self.check_replaced_as_invertible(*node, *level)?;
                }
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
                let removed = self.removal_nodes(*node);
                for removing in removed {
                    self.tree.remove(removing);
                }
                self.forget_missing_dependencies();
            }
        }

        Ok(())
    }

    /// Whether the signatures are the same down to the numbering of the nodes of the tree, which
    /// `==` does not look at. (Actions that refer to nodes mean something else in a signature that
    /// is only equal to the one they were made for.)
    #[must_use]
    pub fn identical(&self, other: &Self) -> bool {
        self.tree.identical(&other.tree) && self.symmetries == other.symmetries
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
