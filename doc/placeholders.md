# Placeholders and axiom schemas

A **placeholder** is a generator that stands for "any cell of this type". Axioms and theorems
that mention placeholders are **schemas**, such as `∀P. φ(P)`. They can be **instantiated** by
replacing each placeholder with an actual diagram of the same type, without leaving any trace
of the substitution in your proofs.

## Using them

1. Create a generator as usual, and turn on *Placeholder* in its settings. Placeholders are
   drawn in a style that ordinary generators can't have.
2. Use it like any other generator when building further generators. An axiom whose diagram
   mentions a placeholder is a schema. So is anything that depends on one: if `e : P ⇒ P` and
   `A : e ; e ⇛ e`, both `e` and `A` are schemas in `P`.
3. To instantiate a schema, open its settings. Put the diagram that should replace each
   placeholder in the workspace, click *Use workspace* in that placeholder's slot, then click
   *Create instance*. Replacements that follow from the ones you choose are filled in for you:
   a cone `α₀ : X₀ ∘ ! ⇒ D₀` can only be a cone over `D₀` with apex `X₀`, so choosing it for `α`
   also fills `X` and `D`. They are shown dimmed and marked *inferred*. Choose another diagram
   for one to override it, or clear it to leave it empty (it stays empty until what it was
   inferred from changes). Nothing you chose yourself is ever replaced.
4. The instance, e.g. `A[Q/P]`, is listed under its schema in the signature. The schema shows as
   a collapsed folder, so instances stay out of the way until you open it. An instance is an
   ordinary generator from then on: attach it, invert it, use it in proofs.

**Theorem schemas need no extra machinery.** A theorem whose statement mentions a placeholder
is itself a schema. To prove `∀P. φ(P)`, prove `φ(P)` with `P` a placeholder.

## What the kernel checks

- **Replacement types.** A placeholder `P : S ⇒ T` may be replaced by any diagram `X : σ(S) ⇒
  σ(T)`, where `σ(S)` is `S` with the same substitution applied. So placeholders can depend on
  other placeholders, e.g. `P : Q ⇒ Q` with `Q` a placeholder too.
- **Dependent generators.** A generator in the schema's boundary that mentions a replaced
  placeholder is instantiated along with it. In the example above, `A[X/P]` is built from
  `e[X/P]`.
- **Instances.** An instance is built from the schema's source and target with the
  substitution applied, and typechecked like any other generator. Asking for the same
  instance again reuses the existing one.
- **No retraction.** Nothing ever turns a diagram back into a placeholder. So instantiating
  never identifies two things that weren't already identified. This avoids the problem with
  the old workaround of an isomorphism `P ≅ X` for each `X`.

## How substitution works, and its limits

Substitution rebuilds a diagram one height at a time. Each height must attach a single
generator, possibly reflected. It is replayed with the replacement attached in its place, and
everything to the right of a replaced cell moves along as the cell grows or shrinks.

A height that does several things at once, such as `f ◫ g` with two cells side by side, is:

- **kept as it is**, if nothing in or around it is replaced, so the instance keeps the schema's
  layout where substitution doesn't care;
- **separated first**, if something in it is replaced. `f ◫ g` becomes `f ▷ _ ; _ ◁ g` (or the
  other order) by an expansion, which is an invertible homotopy, so the instance is still
  derivable from the schema. In that case the instance's boundary is drawn separated, even
  though the schema's was drawn contracted.

A height that cannot be separated this way is refused with an error. This happens when a
replaced cell is caught up in a genuinely higher-dimensional interchange, such as a braid-like
crossing in the boundary of a 4-cell or higher. Replacing a single cell there by a composite
would need new homotopies.

## Inferring instances when attaching

You usually don't need to instantiate by hand. When you click a point in the workspace, the
attach menu also offers instances of schemas whose placeholders can be filled in so that they
fit there. These are tagged with the substitution, e.g. `A [Q ; R/P]`. Choosing one creates the
instance and attaches it in one step, which is undone in one step too.

The search tries replacements made from the cells of the diagram you click on:

- for a 0-cell placeholder, its 0-cells;
- for a placeholder of dimension `k`, composites of up to three of its `k`-cells (and the
  inverses of invertible ones), including identities, with the right source and target.

Placeholders are solved from the lowest dimension up, so a placeholder's type is known when it
is solved. If a schema depends on another generator that mentions a placeholder, the instance
of that generator must already exist for the schema to match: it occurs in the diagram you
click on, after all. The search is bounded, so a very large diagram may not offer every
possibility. Instantiating by hand from the signature still works in that case.

## Filling in replacements in the Instantiate panel

When you choose a replacement, the panel looks for the other placeholders that it determines.
Each replacement chosen has a type in terms of other placeholders (the type of `α` is `X ∘ ! ⇒ D`),
so replacing `α` with `α₀` is only well typed for certain replacements of `X` and `D`. These are
found by matching the types, not by trying replacements:

- a type that is a placeholder is matched with the diagram chosen (`η : F ⇒ G` replaced by
  `p : T ⇒ U` makes `F` be `T` and `G` be `U`), and then the type of that placeholder is matched
  in turn with the boundary of what it was matched with, and so on down, until nothing new is
  bound. A long chain of placeholders, each the type of the next, is filled in from the top;
- otherwise the boundaries are matched, down the dimensions (`h : μ ; η' ⇛ η ; ν`, replaced by
  a cell from `p ; z` to `l ; l`, matches the boundaries of `μ ; η'` with those of `p ; z`,
  so `F` and `G'` are the ends of `p ; z` and `l ; l`);
- a composite of whole cells, like `μ ; η'`, is matched with the diagram cut at some heights. If
  there is only one way to cut it the pieces replace `μ` and `η'`. Otherwise each way is tried:
  a diagram `p ; z` can be `μ ; η'` in three ways (`μ` or `η'` may be an identity);
- a cell that depends on placeholders (the limit `L` of a diagram `D`), matched with an
  instance of it (`L[D₀]`), binds what the instance was made with (`D := D₀`). The inverse of
  an invertible cell is matched like the cell. If the cell is itself an instance made with
  placeholders (`(Ẋη̇)₍₂₎`, see below), what it was made with is matched with what the instance of
  the same schema was made with (`Ḟ ; Ẋ` with `T ; T`, and so on).

A placeholder is filled in if every way that makes the chosen replacements well typed agrees on it.
The matching is bounded, and when it has to stop early only what it has forced so far is filled
in, and the panel says so. Placeholders that the types of the replacements chosen do not mention
are left empty. The search is redone each time a replacement is chosen or cleared, so inferred
replacements never go out of date; what was inferred is only given up when it no longer follows,
and the panel says why if nothing fits.

**Ambiguity.** When the types leave several ways to fill in the rest, nothing of them is filled
in, and the panel lists the ways as *candidates*, each with a *Use* button, the ones without
identities among the cells that the diagram is cut into first. Using one fills in its
placeholders, marked *inferred* like the others: they can be overridden or cleared, and what
follows from them is filled in. Nothing is ever picked for you: a square may well have an
identity in it. Under the list the panel says where the choice is: "`μ ; η'` can be cut 3 ways
to make `p ; ζ[p]`; choose `μ` or `η'` to decide". Choosing one of those placeholders yourself
decides the cut, and then the others follow.

**Checking as you go.** A replacement is checked when it is chosen, against the replacements
chosen before it, on the types alone and without searching: its dimension first, and then its
boundaries against what the others require. If it does not fit, it is kept, in red, with what is
wrong and what it clashes with ("`Ḟ` conflicts with `η̇`: `η̇` needs `Ḟ` to be `T`, not `Ġ`"; "`η̇`
conflicts with `ḣ`: `ḣ := k`: its target is `l ; l`, but the type requires `p ; ν̇` (`η̇ ; ν̇` with
`η̇ := p`) there"). A message names the replacement and says where in it the trouble is (its
source, its target, the target of its source, ...) and what the type requires there, with the
replacements chosen so far put in ("`ḣ := w`: its target is `D₀ ; t`, but the type of `ḣ` needs
`D[a ; t]` (`(Ẋη̇)₍₂₎` with `Ẋ := t`, `η̇ := a`)"). Composites in a message are written like those
in the names of diagrams (`f ; g`, see `doc/labels.md`), each as one formula whatever it is made
of, with a part that is a composite itself in parentheses. It is kept because it
is what you asked for, so you can see what is wrong with it and fix it or what it clashes with;
it takes no part in the inference, and *Create instance* waits until it is fixed or cleared. Of
two replacements that clash, the one chosen first counts.

**It can be cancelled.** What follows from a replacement is worked out as a job (see `CLAUDE.md`):
a step at a time, behind the busy indicator if it takes long, and `Esc` or a click on the
indicator cancels it. The replacements chosen are kept and nothing is inferred; the panel says
so and offers to infer again.

The placeholders a cell can be instantiated at include those that the types of the placeholders it
depends on mention, so a cell that depends on `α` is also instantiated at `X` and `D`.

## Instances that still depend on placeholders

An instance can be made with replacements that are themselves made of placeholders, such as
`(Ẋη̇)₍₂₎ = η̇₍₂₎[Ḟ ; Ẋ/Ḟ, Ġ ; Ẋ/Ġ, η̇ ▷ Ẋ/η̇]`: the cell `η̇₍₂₎` at the images of a cell `η̇` under
a functor `Ẋ`. Its diagram mentions none of those placeholders, but it is still a different cell
for every replacement of `Ẋ` and `η̇`, so it depends on the placeholders that the replacements it
was made with mention (its [`parameters`](../homotopy-model/src/proof/instances.rs)). Replacing
them gives the instance of the schema made with the replacements substituted: `(Ẋη̇)₍₂₎` at
`Ẋ := T`, `η̇ := p` is `η̇₍₂₎[T ; T/Ḟ, T/Ġ, p ▷ T/η̇]`, which is `(Tp)₍₂₎` if that exists. Without
this a schema whose placeholder has `(Ẋη̇)₍₂₎` in its type, like `ḣ : Ẋ(η̇₍₂₎) ≅ (Ẋη̇)₍₂₎`, would
treat it as a fixed cell that no replacement could have in its boundary.

The types of the placeholders are substituted in the same way: the generators in `ḣ`'s type
(not only in the diagram of the schema) that depend on placeholders are instantiated along with
them, and the replacement for `ḣ` must have exactly their instances in its boundary.

## Cells that depend on placeholders they don't mention

A generator is a schema in the placeholders its diagram mentions. Universal properties also
need cells that depend on a placeholder without mentioning it. For example, the mediating map
`u_α : X → L` of a limit depends on the cone `α`, but its type does not mention `α`. Without
that dependency there would be one `u` shared by every cone. In logical terms, an existential
under a universal needs a "Skolem function" `u(α)`.

For this, a generator's settings have a **Depends on** list: tick the placeholders it depends
on. Instances for different replacements of those placeholders are then different generators.
The dependency is inherited: any generator whose diagram mentions `u` also depends on `α`, so
instantiating it at a cone also instantiates `u` at that cone.

### Example: limits as right Kan extensions

Work one level up, with categories as 0-cells, functors as 1-cells and natural transformations
as 2-cells. Write `1` for the terminal category and `! : J → 1`. Then everything is globular:

| generator | type | |
|---|---|---|
| `D` | `J → C` | placeholder: the diagram |
| `L` | `1 → C` | depends on `D`: its limit |
| `π` | `! ; L ⇒ D` | the limit cone |
| `X` | `1 → C` | placeholder: an apex |
| `α` | `! ; X ⇒ D` | placeholder: a cone |
| `u` | `X ⇒ L` | depends on `α`: the mediating map |
| `β` | `α ⇛ (! ◁ u) ; π` | the factorisation |

Uniqueness is one more schema with placeholders `v : X ⇒ L` and `γ : α ⇛ (! ◁ v) ; π`: an
invertible cell `v ⇛ u` (compatible with `γ` and `β` if you want it strict).

To say "`C` has all limits of shape `J`", state these as axioms. To say that one particular
`L` is a limit of one particular `D`, make `D` an ordinary generator instead of a placeholder.

To use the axioms, click a cone `α₀ : ! ; X₀ ⇒ D₀` in your diagram. The attach menu offers
`β [D₀/D, X₀/X, α₀/α]`. Choosing it creates `L[D₀]`, `π[D₀]` and the mediating map `u[…α₀…]`,
and factors the cone through the limit. `L[D₀]` is shared by all cones over `D₀`, while each
cone gets its own mediating map. This example is a test in
`homotopy-model/src/proof/instances.rs`.

The same pattern gives internal homs of a closed (not necessarily compact) monoidal category:
a placeholder `f : A ⊗ X → B`, a currying `λf : X → [A, B]` that depends on `f`, and
`f ⇛ ev ∘ (A ⊗ λf)`. For compact closed categories, use duals instead (see
`doc/duals.md`).

## Not yet

- Inferring replacements longer than three cells, or replacements whose cells are contracted
  into one height (`f ◫ g`) in the diagram you click on.
