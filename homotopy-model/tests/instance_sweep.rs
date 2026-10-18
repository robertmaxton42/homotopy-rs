//! Every instance of a schema that attaches by itself (or inversely) at a point of the tangent
//! categories project is also inferred from its schema, with the exceptions that are known (below),
//! and reading the replacements off the diagram finds what searching for them finds. Needs
//! `HOMOTOPY_TANGENT_FIXTURE=<path to the .hom>`, in release; without it the tests do nothing.
//! `SWEEP_MS` is the time for a generator, `SWEEP_INTERIOR=1` and `SWEEP_STRIDE` add points of the
//! whole workspace to those of its boundaries, and `SWEEP_SCHEMA=<id>` narrows the first test to
//! one schema.
use homotopy_core::{
    common::BoundaryPath, Boundary, Diagram, DiagramN, Generator, Height, SliceIndex,
};
use homotopy_model::{
    history::Proof,
    job::Job,
    proof::{Action, AttachOptionsJob, InferenceLimits},
};

fn fixture() -> Option<Proof> {
    let path = std::env::var("HOMOTOPY_TANGENT_FIXTURE").ok()?;
    let bytes = std::fs::read(path).unwrap();
    let mut state = Proof::default();
    state.update(&Action::ImportProof(bytes.into())).unwrap();
    Some(state)
}

fn run_job(mut job: AttachOptionsJob) -> homotopy_model::proof::AttachOutcome {
    loop {
        if let Some(o) = job.step() {
            return o;
        }
    }
}

fn capped(ms: u32) -> InferenceLimits {
    InferenceLimits {
        generator_ms: ms,
        ..InferenceLimits::default()
    }
}

fn all_points(state: &Proof, interior: bool) -> Vec<Vec<SliceIndex>> {
    let w = state.workspace.as_ref().unwrap();
    let d = DiagramN::try_from(w.diagram.clone()).unwrap();
    let mut points = vec![];
    for b in [Boundary::Source, Boundary::Target] {
        let side = DiagramN::try_from(d.boundary(BoundaryPath(b, 0)).unwrap()).unwrap();
        points.push(vec![SliceIndex::Boundary(b)]);
        for h in 0..=2 * side.size() {
            points.push(vec![
                SliceIndex::Boundary(b),
                SliceIndex::Interior(Height::from(h)),
            ]);
        }
    }
    if interior {
        let stride: usize = var("SWEEP_STRIDE").unwrap_or(1);
        for h5 in (0..=2 * d.size()).step_by(stride) {
            let Some(Diagram::DiagramN(s)) = d.slice(Height::from(h5)) else {
                continue;
            };
            for h4 in (0..=2 * s.size()).step_by(stride + 2) {
                points.push(vec![
                    SliceIndex::Interior(Height::from(h5)),
                    SliceIndex::Interior(Height::from(h4)),
                ]);
            }
        }
    }
    points
}

fn sig_eq(a: &[(Generator, Diagram)], b: &[(Generator, Diagram)]) -> bool {
    let mut a = a.to_vec();
    let mut b = b.to_vec();
    a.sort_by_key(|(p, _)| (p.dimension, p.id));
    b.sort_by_key(|(p, _)| (p.dimension, p.id));
    a == b
}

fn var<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok().and_then(|s| s.parse().ok())
}

type Wanted = (
    Generator,
    Vec<(Generator, Diagram)>,
    bool,
    Vec<usize>,
    String,
);

#[test]
fn existing_instances_are_inferred() {
    let Some(state) = fixture() else { return };
    let limit: u32 = var("SWEEP_MS").unwrap_or(20_000);
    let interior = std::env::var("SWEEP_INTERIOR").is_ok();
    let only_schema: Option<usize> = var("SWEEP_SCHEMA");
    let instances: Vec<_> = state
        .signature
        .iter()
        .filter(|i| i.instance.is_some())
        .filter(|i| only_schema.is_none_or(|s| i.instance.as_ref().unwrap().schema.id == s))
        .collect();
    let (mut checked, mut found, mut missed, mut timeouts) = (0, 0, 0, 0);
    let mut unexpected = 0;
    for point in all_points(&state, interior) {
        let mut wanted: Vec<Wanted> = vec![];
        for info in &instances {
            let made = info.instance.as_ref().unwrap();
            let Diagram::DiagramN(d) = &info.diagram else {
                continue;
            };
            let Ok(job) =
                AttachOptionsJob::new(&state, &point, false, capped(2_000), Some(info.generator))
            else {
                continue;
            };
            let out = run_job(job);
            for o in &out.options {
                if o.substitution.is_some() || o.reflection {
                    continue;
                }
                let inverse = if o.diagram == *d {
                    false
                } else if o.diagram == d.inverse() {
                    true
                } else {
                    continue;
                };
                wanted.push((
                    made.schema,
                    made.substitution.clone(),
                    inverse,
                    o.embedding.iter().copied().collect(),
                    info.name.clone(),
                ));
            }
        }
        if wanted.is_empty() {
            continue;
        }
        let mut schemas: Vec<Generator> = wanted.iter().map(|w| w.0).collect();
        schemas.sort_by_key(|g| (g.dimension, g.id));
        schemas.dedup();
        for schema in schemas {
            let t = std::time::Instant::now();
            let job =
                AttachOptionsJob::new(&state, &point, false, capped(limit), Some(schema)).unwrap();
            let out = run_job(job);
            let cut = !out.inference.cut_short.is_empty();
            println!(
                "{point:?} schema {schema:?}: {} options in {:?}{}",
                out.options.len(),
                t.elapsed(),
                if cut { " CUT" } else { "" }
            );
            for (_, subst, inverse, embedding, name) in wanted.iter().filter(|w| w.0 == schema) {
                checked += 1;
                let hit = out.options.iter().any(|o| {
                    o.substitution.as_ref().is_some_and(|x| sig_eq(x, subst))
                        && o.inverse == *inverse
                        && o.embedding.iter().copied().collect::<Vec<_>>() == *embedding
                });
                if hit {
                    found += 1;
                } else if cut {
                    timeouts += 1;
                    println!("   TIMEOUT {name} (inverse {inverse}) {embedding:?}");
                } else {
                    missed += 1;
                    println!("   MISSED {name} (inverse {inverse}) {embedding:?}");
                    unexpected += usize::from(!KNOWN_GAPS.contains(&schema.id));
                }
            }
        }
    }
    println!("checked {checked}: found {found}, missed {missed}, timed out {timeouts}");
    assert_eq!(
        unexpected, 0,
        "instances made by hand that are not inferred"
    );
}

/// The schemas that instances are made of by hand for and that are not inferred, on purpose: `ζ[η̇]`
/// and `σ[η̇]` (and the instance of `ζ` that is `ζ[η̇′]`) depend on a transformation `η̇` that the
/// cells they attach along do not mention (`ζ[η̇] : Ġ ⇒ Ḟ`), so nothing in the diagram says which
/// of the transformations there are is meant; and `−₍₂₎ congr` has placeholders (`ι₁`, `ι₂`, `ḣ`)
/// that only the instance made by hand says anything about.
const KNOWN_GAPS: [usize; 4] = [29, 38, 47, 97];

/// How many options a click on the whole workspace has, by what they are.
#[test]
fn what_a_click_on_the_whole_workspace_offers() {
    let Some(state) = fixture() else {
        return;
    };
    for seeding in [false, true] {
        let limits = InferenceLimits {
            generator_ms: 150,
            seeding,
            ..InferenceLimits::default()
        };
        let job = AttachOptionsJob::new(&state, &[], false, limits, None).unwrap();
        let t = std::time::Instant::now();
        let out = run_job(job);
        let mut by_schema = std::collections::BTreeMap::new();
        for o in &out.options {
            *by_schema
                .entry((o.generator.id, o.substitution.is_some(), o.inverse))
                .or_insert(0) += 1;
        }
        println!(
            "seeding {seeding}: {} options in {:?}: {by_schema:?}",
            out.options.len(),
            t.elapsed()
        );
    }
}

/// Reading the replacements off the instances in the diagram finds what the search for them
/// finds, and is quicker. For every schema of the project at every point (the limit for a
/// generator is `SWEEP_MS`, 5 s): a search that ran out of time is not compared.
#[test]
fn reading_replacements_off_the_diagram_finds_what_searching_finds() {
    use std::time::{Duration, Instant};

    let Some(state) = fixture() else {
        return;
    };
    let limit: u32 = var("SWEEP_MS").unwrap_or(5_000);
    let interior = std::env::var("SWEEP_INTERIOR").is_ok();
    let schemas: Vec<Generator> = state
        .signature
        .iter()
        .filter(|info| {
            info.instance.is_none() && !state.signature.parameters(info.generator).is_empty()
        })
        .filter(|info| !info.placeholder)
        .filter(|info| var::<usize>("SWEEP_SCHEMA").is_none_or(|id| info.generator.id == id))
        .map(|info| info.generator)
        .collect();
    let (mut compared, mut same, mut more, mut lost) = (0, 0, 0, 0);
    let (mut off_total, mut on_total) = (Duration::ZERO, Duration::ZERO);
    let (mut off_cut, mut on_cut) = (0, 0);
    for point in all_points(&state, interior) {
        for &schema in &schemas {
            let run = |seeding: bool| {
                let limits = InferenceLimits {
                    generator_ms: limit,
                    seeding,
                    ..InferenceLimits::default()
                };
                let Ok(job) = AttachOptionsJob::new(&state, &point, false, limits, Some(schema))
                else {
                    return None;
                };
                let t = Instant::now();
                let out = run_job(job);
                Some((out, t.elapsed()))
            };
            let (Some((off, t_off)), Some((on, t_on))) = (run(false), run(true)) else {
                continue;
            };
            off_total += t_off;
            on_total += t_on;
            let (cut_off, cut_on) = (
                !off.inference.cut_short.is_empty(),
                !on.inference.cut_short.is_empty(),
            );
            off_cut += usize::from(cut_off);
            on_cut += usize::from(cut_on);
            let instances = |out: &homotopy_model::proof::AttachOutcome| {
                let mut found: Vec<_> = out
                    .options
                    .iter()
                    .filter_map(|o| {
                        let mut s = o.substitution.clone()?;
                        s.sort_by_key(|(p, _)| (p.dimension, p.id));
                        Some((
                            s,
                            o.inverse,
                            o.embedding.iter().copied().collect::<Vec<_>>(),
                        ))
                    })
                    .collect();
                found.sort_by_key(|f| format!("{f:?}"));
                found
            };
            let (a, b) = (instances(&off), instances(&on));
            if t_off > Duration::from_millis(300) || t_on > Duration::from_millis(300) || a != b {
                println!(
                    "{point:?} {schema:?}: searching {:?}{} {} found, reading {:?}{} {} found",
                    t_off,
                    if cut_off { " (cut)" } else { "" },
                    a.len(),
                    t_on,
                    if cut_on { " (cut)" } else { "" },
                    b.len()
                );
            }
            if cut_off || cut_on {
                continue;
            }
            compared += 1;
            let missing = a.iter().filter(|f| !b.contains(f)).count();
            let extra = b.iter().filter(|f| !a.contains(f)).count();
            lost += missing;
            more += extra;
            same += usize::from(missing == 0 && extra == 0);
            if missing > 0 {
                println!("  LOST {missing} of {}", a.len());
            }
        }
    }
    println!(
        "{compared} searches compared: {same} the same, {more} instances more, {lost} lost; \
         searching {off_total:?} ({off_cut} cut), reading {on_total:?} ({on_cut} cut)"
    );
    assert_eq!(lost, 0);
}
