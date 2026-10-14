# Attaching a specific generator

Clicking a point of the diagram in the workspace lists everything that can be attached there:
cells of the signature, their inverses, the cells raised from them, their reflections, and
instances of schemas that were found by a search (see [placeholders](placeholders.md)). With a
big signature that search takes a while, and you may already know what you want to attach. In
that case, say so:

- **Drag it.** Drag a generator from the signature onto the diagram. The part of the diagram
  under the pointer is outlined: that is where it would be attached. Drop it there.
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
list. While the search runs the busy pill is up, as for a click, and `Esc` cancels it. The
project is unchanged if you cancel.

A generator that cannot be attached anywhere in the diagram at all (it has more than one
dimension more than the diagram has, the cells that it starts and ends at are not in the
diagram, or it is a 0-cell and weak units are off) is not accepted by the diagram when you drag
it there, and the button tells you why. This is worked out when you pick the generator, from
dimensions and which cells there are; it never searches, so it cannot say that there is a way
to attach it at a particular point, only that there is no way at all.

## Instances of schemas are a heuristic

Finding the instances of a schema that attach at a point is a search over replacements of its
placeholders. It can miss instances that exist, and it stops when its time is up (Settings:
*Infer instances of schemas when attaching*, *Stop inferring after N s*, *Infer instances up to
dimension N*, see [placeholders](placeholders.md)); the instances it does offer are always
valid. When it gives up, a notice says so and says which schemas it gave up on, and you can
give it more time, or turn it off.

This is available in the 2D views (and the 1D and 0D ones); the 3D view does not take drops yet.
