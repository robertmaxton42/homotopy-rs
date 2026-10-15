use std::cell::RefCell;

use homotopy_common::{
    hash::{FastHashMap, FastHashSet},
    tex::seal,
};
use homotopy_core::{common::Generator, signature::Signature as _};
use yew::prelude::*;

use crate::{
    app::tex::TexSpan,
    components::search_status::SearchStatus,
    model::{
        proof::{self, AttachOption, Signature},
        Action, Definitions, Listing, Selectables,
    },
};

#[derive(Debug, Clone, PartialEq, Properties)]
pub struct Props {
    pub dispatch: Callback<Action>,
    pub options: Selectables,
    /// If the first options are the definitions of the cell that was clicked (a shortcut: the
    /// search for the rest may not have been done).
    #[prop_or_default]
    pub definitions: Option<Definitions>,
    /// Whether the list is complete, still being filled in by a search that is going on, or what
    /// a search that was stopped had found.
    #[prop_or_default]
    pub listing: Listing,
    pub signature: Signature,
}

impl Props {
    /// Whether picking an option ends a search that is going on first.
    fn searching(&self) -> bool {
        self.listing == Listing::Searching
    }
}

#[function_component]
pub fn AttachView(props: &Props) -> Html {
    match &props.options {
        Selectables::Attach(att) => {
            // The definitions, if they were offered first, are the first of the list; the rest
            // is grouped as it always was.
            let shown = props
                .definitions
                .as_ref()
                .map_or(0, |definitions| definitions.count.min(att.len()));
            let (defined, att) = att.split_at(shown);
            let entries = group_instances(att, |option| {
                option.substitution.is_some().then_some(option.generator)
            });
            let keys = option_keys(att);
            let searching = props.searching();
            html! {
                <>
                    // While the search goes on: what it is doing, and how to stop it. The list
                    // under it only grows.
                    if searching {
                        <SearchStatus on_stop={props.dispatch.reform(|()| Action::StopSearch)} />
                    } else if props.listing == Listing::Stopped {
                        <li class="attach__note attach__stopped">
                            {"The search was stopped before it was done: these are the options \
                              it had found, and there may be others."}
                        </li>
                    }
                    if let Some(definitions) = props.definitions.as_ref().filter(|_| shown > 0) {
                        { view_definitions(props, definitions, defined, !att.is_empty()) }
                    }
                    { for entries.into_iter().map(|entry| match entry {
                        Entry::Item(index) => view_option(
                            &props.dispatch,
                            &props.signature,
                            &att[index],
                            "attach__option",
                            searching,
                            keys[index].clone(),
                        ),
                        Entry::Group(schema, indices) => html! {
                            <InstanceGroup
                                key={format!("g-{}-{}", schema.id, schema.dimension)}
                                dispatch={props.dispatch.clone()}
                                signature={props.signature.clone()}
                                schema={schema}
                                searching={searching}
                                options={indices.into_iter().map(|index| (keys[index].clone(), att[index].clone())).collect::<Vec<_>>()}
                            />
                        },
                    }) }
                </>
            }
        }
        Selectables::Merge(from, tos) => html! {
            { for tos.iter().map(|to| view_merge_option(props, *from, *to)) }
        },
    }
}

/// The definitions that were offered first: under a heading that says whose they are and that
/// they are a shortcut, then (until everything has been searched for) the way to search for the
/// rest.
fn view_definitions(
    props: &Props,
    definitions: &Definitions,
    defined: &[AttachOption],
    others_follow: bool,
) -> Html {
    let name = props
        .signature
        .generator_info(definitions.of)
        .map_or_else(String::new, |info| seal(&info.name));
    let search = props
        .dispatch
        .reform(|_: MouseEvent| Action::SearchEverything);
    let gave_up = (!definitions.cut_short.is_empty()).then(|| {
        format!(
            "Gave up looking for a definition after the time for a generator ({}). What is \
             listed is valid but may be missing some.",
            definitions
                .cut_short
                .iter()
                .map(|name| seal(name))
                .collect::<Vec<_>>()
                .join(", ")
        )
    });
    html! {
        <>
            <li class="attach__heading">
                <span>{"Definitions of"}</span>
                <TexSpan
                    class="attach__heading-name"
                    error_color="#c004"
                    raw_tex={name}
                />
            </li>
            { for defined.iter().enumerate().map(|(index, option)| view_option(
                &props.dispatch,
                &props.signature,
                option,
                "attach__option attach__option--definition",
                props.searching(),
                format!("definition-{index}"),
            )) }
            if let Some(gave_up) = gave_up {
                <li class="attach__note">
                    <TexSpan class="attach__option-name" error_color="#c004" raw_tex={gave_up} />
                </li>
            }
            if props.searching() {
                // (What is found comes below, and the line at the top says it is going on.)
                if others_follow {
                    <li class="attach__heading">{"Other options"}</li>
                }
            } else if definitions.more.is_some() {
                <li
                    class="attach__option attach__more"
                    role="button"
                    title="Looks at every generator; this can take a while"
                    onclick={search}
                >
                    <span class="attach__option-name">
                        {if others_follow {
                            "Search everything again"
                        } else {
                            "Search everything for more options"
                        }}
                    </span>
                </li>
                <li class="attach__note">
                    {"Definitions are offered first as a shortcut (a heuristic): other things may attach here."}
                </li>
                if others_follow {
                    <li class="attach__heading">{"Other options (found before the search was stopped)"}</li>
                }
            } else if others_follow {
                <li class="attach__heading">{"Other options"}</li>
            } else {
                <li class="attach__note">{"Nothing else attaches here."}</li>
            }
        </>
    }
}

/// An entry of the list of options as it is shown: an option by itself (its index), or the
/// instances of a schema (its generator and the indices of its options).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Item(usize),
    Group(Generator, Vec<usize>),
}

/// How the options are shown: the plain ones, in their order, then the instances that were
/// inferred, in a group for each schema (in the order that its first instance came in), so that
/// all the instances of a schema that is not the one being looked for can be skipped at once.
///
/// `instance_of` says which schema an option is an instance of, if it is one.
pub fn group_instances<T>(
    options: &[T],
    instance_of: impl Fn(&T) -> Option<Generator>,
) -> Vec<Entry> {
    let mut entries: Vec<Entry> = vec![];
    let mut groups: Vec<(Generator, Vec<usize>)> = vec![];
    for (index, option) in options.iter().enumerate() {
        match instance_of(option) {
            None => entries.push(Entry::Item(index)),
            Some(schema) => match groups.iter_mut().find(|(other, _)| *other == schema) {
                Some((_, indices)) => indices.push(index),
                None => groups.push((schema, vec![index])),
            },
        }
    }
    entries.extend(
        groups
            .into_iter()
            .map(|(schema, indices)| Entry::Group(schema, indices)),
    );
    entries
}

thread_local! {
    /// The schemas whose instances were opened in this session, so that they are not shut again
    /// by the next click that has some of them. Everything else is closed.
    static OPENED: RefCell<FastHashSet<Generator>> = RefCell::default();
}

#[derive(Debug, Clone, PartialEq, Properties)]
struct GroupProps {
    dispatch: Callback<Action>,
    signature: Signature,
    schema: Generator,
    /// Whether a search is still going on (picking an option ends it).
    searching: bool,
    /// The options with their keys, which stay what they are as more arrive.
    options: Vec<(String, AttachOption)>,
}

/// The instances of one schema that were inferred, behind a header with the schema's name and how
/// many there are. Closed unless it was opened before in the session; *closed, nothing under it is
/// rendered* (typesetting the formulas of the options is what takes the time), and only the name
/// is.
#[function_component]
fn InstanceGroup(props: &GroupProps) -> Html {
    let open = use_state(|| OPENED.with(|opened| opened.borrow().contains(&props.schema)));
    let schema = props.schema;
    let toggle = {
        let open = open.clone();
        Callback::from(move |_: MouseEvent| {
            let now = !*open;
            OPENED.with(|opened| {
                if now {
                    opened.borrow_mut().insert(schema);
                } else {
                    opened.borrow_mut().remove(&schema);
                }
            });
            open.set(now);
        })
    };
    let info = props.signature.generator_info(props.schema);
    let name = info.map_or_else(String::new, |info| seal(&info.name));
    let count = props.options.len();
    html! {
        <>
            <li
                class="attach__option attach__group"
                role="button"
                aria-expanded={open.to_string()}
                onclick={toggle}
            >
                <span
                    class="attach__option-color"
                    style={format!("background: {}", info.map_or_else(String::new, |info| info.color.to_string()))}
                />
                <span class="attach__group-toggle">{if *open { "\u{25be}" } else { "\u{25b8}" }}</span>
                <TexSpan
                    class="attach__option-name"
                    error_color="#c004"
                    raw_tex={name}
                />
                <span class="attach__group-count">
                    {format!("{count} instance{}", if count == 1 { "" } else { "s" })}
                </span>
            </li>
            if *open {
                { for props.options.iter().map(|(key, option)| view_group_option(
                    &props.dispatch,
                    &props.signature,
                    option,
                    props.searching,
                    key.clone(),
                )) }
            }
        </>
    }
}

/// An instance in an open group: as any option, but it does not repeat the name of the schema.
fn view_group_option(
    dispatch: &Callback<Action>,
    signature: &Signature,
    option: &AttachOption,
    searching: bool,
    key: String,
) -> Html {
    view_option(
        dispatch,
        signature,
        option,
        "attach__option attach__option--grouped",
        searching,
        key,
    )
}

/// Keys for the options of a list that stay the same when more options are put into it, so that
/// those that are there are not made again: what the option is, and which of those like it.
fn option_keys(options: &[AttachOption]) -> Vec<String> {
    let mut seen: FastHashMap<String, usize> = FastHashMap::default();
    options
        .iter()
        .map(|option| {
            let base = format!(
                "{}-{}-{}-{:?}",
                option.generator.id,
                option.generator.dimension,
                option.label.as_deref().unwrap_or(""),
                option.embedding,
            );
            let count = seen.entry(base.clone()).or_insert(0);
            *count += 1;
            format!("{base}#{count}")
        })
        .collect()
}

/// An option of the list. While a search is going on (`searching`) picking one attaches it and
/// ends the search, which would otherwise refuse the attachment (see [`Action::PickStreamed`]).
fn view_option(
    dispatch: &Callback<Action>,
    signature: &Signature,
    option: &AttachOption,
    class: &'static str,
    searching: bool,
    key: String,
) -> Html {
    let info = signature.generator_info(option.generator).unwrap();

    let onclick = dispatch.reform({
        let option = option.clone();
        move |_| {
            if searching {
                Action::PickStreamed(option.clone())
            } else {
                Action::Proof(proof::Action::Attach(option.clone()))
            }
        }
    });

    let onmouseenter = dispatch.reform({
        let option = option.clone();
        move |_| Action::HighlightAttachment(Some(option.clone()))
    });

    let onmouseleave = dispatch.reform(|_| Action::HighlightAttachment(None));

    // TODO: Maybe extract a common component for this and the signature.
    // TODO: How should highlights work on touch devices?
    html! {
        <li
            key={key}
            class={class}
            onclick={onclick}
            onmouseenter={onmouseenter}
            onmouseleave={onmouseleave}
        >
            <span
                class="attach__option-color"
                style={format!("background: {}", info.color)}
            />
            <TexSpan
                class="attach__option-name"
                error_color="#c004"
                raw_tex={option.label.clone().unwrap_or_else(|| seal(&info.name))}
            />
        </li>
    }
}

fn view_merge_option(props: &Props, from: Generator, to: Generator) -> Html {
    let info = props.signature.generator_info(to).unwrap();

    let onclick = props
        .dispatch
        .reform(move |_| Action::Proof(proof::Action::Merge(from, to)));

    html! {
        <li
            class="attach__option"
            onclick={onclick}
        >
            <span
                class="attach__option-color"
                style={format!("background: {}", info.color)}
            />
            <TexSpan
                class="attach__option-name"
                error_color="#c004"
                raw_tex={
                    info.name.clone()
                }
            />
        </li>
    }
}

#[cfg(test)]
mod tests {
    use homotopy_core::common::Generator;

    use super::{group_instances, Entry};

    /// Options as (schema if an instance of it, anything): the plain ones stay as they are and
    /// first; each schema has one group, where its first instance was.
    #[test]
    fn instances_are_grouped_by_schema_after_the_plain_options() {
        let (a, b) = (Generator::new(1, 5), Generator::new(2, 5));
        let options = [Some(a), None, Some(b), Some(a), None, Some(a), Some(b)];
        assert_eq!(
            group_instances(&options, |option| *option),
            vec![
                Entry::Item(1),
                Entry::Item(4),
                Entry::Group(a, vec![0, 3, 5]),
                Entry::Group(b, vec![2, 6]),
            ]
        );
        assert_eq!(group_instances::<Option<Generator>>(&[], |o| *o), vec![]);
        // Nothing to group: the same list.
        assert_eq!(
            group_instances(&[None::<Generator>, None], |o| *o),
            vec![Entry::Item(0), Entry::Item(1)]
        );
    }
}
