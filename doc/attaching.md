# Attaching a specific generator

Clicking a point of the diagram in the workspace lists everything that can be attached there:
cells of the signature, their inverses, the cells raised from them, their reflections, and
instances of schemas that were found by a search (see [placeholders](placeholders.md)). With a
big signature that search takes a while, and you may already know what you want to attach. In
that case, say so:

- **Drag it.** Drag a generator from the signature onto the diagram. A small copy of its row
  follows the pointer, and the part of the diagram under the pointer is outlined: that is where it
  would be attached. Drop it there. `Esc` (or letting go anywhere but on the diagram) puts it
  down without attaching anything. Start the drag towards the diagram, sideways: a drag of a
  row that starts up or down is the browser's drag, which moves the row in the signature, as it
  always did. Press on the pointing hand (see below) instead and the drag is to the diagram
  whichever way it starts, which is also how to drag with a finger. (A row dragged up or down
  and then on to the diagram can still be dropped on it, but the browser gives the page neither
  the wheel nor the keys during that kind of drag, so there is no choosing a sheet then; see
  *Sheets*.)
- **Or pick it.** Hover over a generator in the signature and press the pointing-hand button
  that appears at the end of its row. A chip at the top of the page says that it is waiting
  (*Attaching ... click a point in the diagram*); click a point of the diagram to attach it
  there. `Esc`, or clicking the chip or the button again, puts it down.

Either way only that generator is searched for, at that point: the generator itself, its
inverse, the cells raised from it, its reflections, and, if it is a schema, its instances.
(Other generators are not looked at, so this is a good deal quicker than a click, which looks
at all of them, and a schema gets all of the time that inferring instances is allowed rather
than a share of it.) What happens next is as for a click: if there is nothing to attach you are
told so, if there is one way to do it it is done, and if there are several you choose from a
list. While the search runs the busy pill is up, as for a click (see *The list fills in as the
search goes* below for what `Esc` does).

A generator that cannot be attached anywhere in the diagram at all (it has more than one
dimension more than the diagram has, the cells that it starts and ends at are not in the
diagram, or it is a 0-cell and weak units are off) is not accepted by the diagram when you drag
it there, and the button tells you why. This is worked out when you pick the generator, from
dimensions and which cells there are; it never searches, so it cannot say that there is a way
to attach it at a particular point, only that there is no way at all.

## The list fills in as the search goes

A search can take many seconds, and what you want is often found early. So the list of options
does not wait for the end: it opens when the **first option is found** and the rest are put into
it as the search finds them. A line at the top says **Still searching…** (what it is doing, and
how far along it is) with a **Stop** button.

- **The order is the final order.** The generators are searched cheapest first, but the options are
  listed in the order of the generators, as they always were in a finished list. A new option is
  put into its place, so the list grows in the middle as well as at the end, and an option that is
  there never moves. The keys `1` to `9` stand for places in the finished list, which is not known
  yet, so they do nothing while the search runs (the busy pill shakes); they work on the list
  once the search is over. (Nothing in the list is numbered on screen.)
- **Clicking an option while the search runs** means "this is the one": the search is stopped and
  that option is attached, as it would have been had the search finished. The search had not
  changed the project, so nothing else is affected.
- **`Esc`, the **Stop** button, or clicking the busy pill** stops the search and **keeps the
  options found so far**. The list says it was stopped and the notice says that there may be
  others; the project is unchanged (listing options does not change it). `Esc` again, or the
  close button, puts the list away. If nothing had been found, the list is as it was before.
  Closing the list while it is filling in stops the search and puts the list away.
- **One option is attached at once only if the search ends with exactly one.** The first option
  found is listed, not attached, since more may follow.
- With **definitions first**, *Search everything* fills in the list below the definitions in the
  same way, and a stopped search keeps both, with *Search everything again* to start over.

## Definitions first

When you click on **one cell** of the diagram, what you want next is nearly always that cell's
*definition*: a generator one dimension up that has the cell as its source or its target, such
as the unfolding that was made right after it. So the list does not wait for the search of
everything: it first offers the definitions, at the top, under "Definitions of X". They are found
by comparing boundaries (no search), checked by matching them where you clicked, so they all
attach; and a button, **Search everything for more options**, runs the full search, which keeps
the definitions at the top and lists the rest below them.

"One cell" means: you clicked on a vertex of the picture (not a wire or a region), and the cells
of the dimension of the diagram there are all of one generator. A vertex with several different
cells behind each other (see *Sheets*), a vertex where only lower cells are (a crossing), a wire,
a region and a click with a sheet chosen that is not a vertex are searched for as always, as are
clicks where the cell has no definition, or none of its definitions attaches there. The
generator after the cell in the signature is looked at first, then the others.

This is a shortcut and can miss. Instances of schemas are only offered if they already exist as
generators (they are not inferred or made), and the duals and other reflections of a definition
are not looked for: use *Search everything*. It never offers something that does not attach.
Settings: *Offer definitions first when clicking a cell* (on by default); turn it off to have
every click search everything. Picking a generator and clicking, or dropping one, always search
for that generator only, as before.

## Sheets: what is behind a position

In the 2D view of a diagram of more than two dimensions a position of the picture stands for
everything that the other dimensions have there, and the picture shows only the cell that is in
front. The others are behind it: they are the **sheets** of the position (one for each singular
height of the next dimension, `z₃`). A click is at a position, so it says nothing about which
sheet you mean, and the search looks at everything that reaches the position. If you know which
sheet you mean, say so, and the search only looks at attachments that reach that sheet.

Hold the pointer over a position that has several sheets and the tooltip lists them, with the
cell at each (`z₃=0 c`, `z₃=1 𝒞`, ...). Each also has a short name (`S1`, `S2`, ...) that is the
same wherever that sheet is: a path that you follow along the diagram, over a vertex, a wire or a
region, keeps its name, so you can see that two positions are the same sheet. The one marked
*front* is the sheet that is in front along the whole of its length: it is not hidden by another
sheet anywhere it overlaps one. (Where it is alone the picture shows it whether it is in front or
not; the sheet that is behind elsewhere is then marked *drawn here*.) Sheets that meet at a point
are not the same sheet. Nothing changes unless you choose one:

- **Shift + wheel**, or **`]`** and **`[`**, move down and up the list (round the ends). The
  chosen sheet is highlighted, and the wires that are not on it are drawn dim.
- **Tab** fixes the sheet you are on and goes one dimension further in, to the sheets inside it
  for the next coordinate (`z₃=1 › z₄=0`); the list is then of those. Tab again goes further in,
  as far as the diagram has dimensions. **Shift + Tab** takes the last one back. Nothing is ever
  fixed for you: what is in the breadcrumb is what you pressed.
- Moving the pointer to another part of the diagram forgets the choice. Where wires (or
  vertices) overlap on the screen, the list has the sheets of each of them, tagged *wire*,
  *vertex*; choosing one of those chooses that wire.

Then **click**, **drop** a generator from the signature, or click after **picking** one (the
pointing hand) as before: the point that is searched is the position and the sheets you chose
(the tooltip's breadcrumb), and what is offered is exactly the attachments of the position whose
cell reaches the sheet. An attachment that spans several sheets is still offered as long as it
covers the one you chose: choosing a sheet leaves out what cannot be at it, and never leaves out
what can. This is often much quicker, as the search does not look at the rest. You can choose the sheet
**during a drag**: hold the generator over the position and use Shift + wheel, `[`, `]` and Tab
as above; the tooltip, which follows the pointer, shows what is chosen, and dropping attaches at
that sheet. (This is so for a drag that starts towards the diagram, or from the pointing hand,
which are drags of the page's own; in the browser's drag of a row that was started up or down
there are neither keys nor wheel. Then choose the sheet before the drag, over the position you
will drop on: a drop uses the sheet that was chosen for the position it lands on, if there is
one, or pick the generator with the pointing hand, choose the sheet, and click.)

Dragging a wire sideways to expand it (a horizontal drag) acts on the sheet you chose at the
place the drag starts from, if you chose one there before pressing the button, and else on the
front one, as before. Contractions act on all the sheets and are the same whatever you chose.

Where a sheet **ends**, or several sheets **meet**, the 2D view marks the place (Settings: *Outline
where sheets end or meet*, on by default): a dashed ring, with the last stretch of the sheet dashed
up to it, where a sheet ends at a cell, and a double ring where three or more meet at one (the cell
where lines are joined), and the tooltip there says so. Only cells (points of the picture) are
marked: a line that simply runs along a wire, two strands that cross, or surfaces that are joined
along a line, are not, as the wire is drawn.

The 2D view also shades each **surface** by the number of sheets that pass through it
(Settings: *Shade surfaces by the sheets behind them*, on by default): the surface that has the
fewest in the diagram is drawn as it always was, and the others a little darker (in up to three
steps) the more they have, so that you can see where a lot is hidden behind a region. Hatched
(placeholder) surfaces keep their stripes.

Choose no sheet (leave the position and come back, or Shift + Tab all the way) and a click is
what it has always been. The 3D view has no sheets.

### Narrowing the instances to the sheet is a heuristic

Settings: *Heuristic: infer instances only from cells near the chosen sheet* (off by default).
When it is on and a sheet is chosen, the replacements tried for the placeholders of a schema are
made of the cells that occur near that sheet in the diagram (at the heights of the point and
those next to them) and not of all the cells of the diagram. This makes the inference quicker,
and it can miss instances that use cells from elsewhere in the diagram; it never offers one
that is not valid, because each is checked as before and only the pool of cells it is chosen
from is smaller. The notice that follows the search says that it was in effect and how many of
the diagram's cells it looked at.

## Each generator has a time limit

Finding where the cells of the signature (and their inverses, identities, bubbles and
reflections) attach is a matching of each against the diagram, which in high dimensions takes
long enough to wait for; for a schema it is also a search for the replacements of its
placeholders, which can have no end. So *any one generator* is given a time (Settings, *Give up
on a generator after N s*, 5 s unless you change it, 0 for no limit) for all of that: after it
the search gives up on that generator, keeps what it had found for it, and goes on to the next.
There is no limit on the search as a whole, only on each generator: a diagram where more generators
could attach takes longer, but a generator that takes long does not make the others wait. (`Esc`
ends the search and the project is as it was; the options found so far stay listed.)

What is offered is always valid, but may be missing some of what a generator would have had, and
a notice says which generators were given up on ("Gave up on 2 generators after 5 s each: ...";
"Instance inference gave up on a schema after 5 s: ..."). To look for just those, drag the
generator onto the diagram or pick it with the pointing hand (the search for one generator has
the same time, so raise the limit if you want to wait longer for it), or choose a sheet, which
narrows the search to the cells that reach it. The cheapest generators are looked at first.

If a step of the search does not look at the clock and runs on past the time (this should not
happen, but the page does not rely on it), the page ends the background worker after the time for
a generator and a short grace and offers what the worker had found before the generator it was in
the middle of; that generator is given up on, the ones after it are listed as not tried, and
the notice says the search was ended.

## Instances of schemas are a heuristic

Finding the instances of a schema that attach at a point is a search over replacements of its
placeholders. It can miss instances that exist, and it gives up on a schema when its time is
up (Settings: *Infer instances of schemas when attaching*, *Give up on a generator after N s*,
*Infer instances up to dimension N*, see [placeholders](placeholders.md)); the instances it does
offer are always valid. When it gives up, a notice says so and says which schemas it gave up on,
and you can give it more time, or turn it off. If two searches in a row give up on every schema
they look at, instances are no longer inferred for the rest of the session (turn the setting off
and on to start again).

This is available in the 2D views (and the 1D and 0D ones); the 3D view does not take drops yet.
