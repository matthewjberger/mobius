//! The live agent graph: nodes laid out on a ring, edges drawn between them, each
//! node colored by status and showing its latest output. Clicking a node opens it
//! in the inspector.

use std::collections::HashMap;

use leptos::prelude::*;
use protocol::UiCommand;

use crate::bus::{self, Bus};
use crate::presets::{self, PRESETS};
use crate::state::{MobiusState, status_class, truncate};

#[component]
pub fn GraphView(state: MobiusState, bus: Bus) -> impl IntoView {
    let goal = RwSignal::new(String::new());
    let analyze = {
        let bus = bus.clone();
        move || {
            let goal_text = goal.get_untracked().trim().to_string();
            state.analyzing.set(true);
            state.analyze_error.set(None);
            state.analyze_progress.set(String::new());
            state.suggestions.set(Vec::new());
            bus::publish_command(&bus, &UiCommand::Analyze { goal: goal_text });
        }
    };

    view! {
        <div class="graph">
            {move || {
                let snapshot = state.snapshot.get();
                if snapshot.nodes.is_empty() {
                    return start_screen(state, bus.clone(), goal, analyze.clone()).into_any();
                }

                let count = snapshot.nodes.len();
                let index: HashMap<String, usize> = snapshot
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(position, view)| (view.spec.id.clone(), position))
                    .collect();
                let place = move |slot: usize| -> (f64, f64) {
                    if count <= 1 {
                        return (50.0, 40.0);
                    }
                    let angle = std::f64::consts::TAU * (slot as f64) / (count as f64)
                        - std::f64::consts::FRAC_PI_2;
                    (50.0 + 33.0 * angle.cos(), 50.0 + 33.0 * angle.sin())
                };

                let pulse = state.pulse.get();
                let edges = snapshot
                    .edges
                    .iter()
                    .filter_map(|edge| {
                        let from = *index.get(&edge.from)?;
                        let to = *index.get(&edge.to)?;
                        let (x1, y1) = place(from);
                        let (x2, y2) = place(to);
                        let hx = x1 + (x2 - x1) * 0.7;
                        let hy = y1 + (y2 - y1) * 0.7;
                        let active = pulse
                            .as_ref()
                            .is_some_and(|(pf, pt)| pf == &edge.from && pt == &edge.to);
                        let line_class = if active { "edge pulse" } else { "edge" };
                        let head_class = if active { "edge-head pulse" } else { "edge-head" };
                        Some(view! {
                            <line
                                x1=format!("{x1:.2}%")
                                y1=format!("{y1:.2}%")
                                x2=format!("{x2:.2}%")
                                y2=format!("{y2:.2}%")
                                class=line_class
                            ></line>
                            <circle cx=format!("{hx:.2}%") cy=format!("{hy:.2}%") r="4" class=head_class></circle>
                        })
                    })
                    .collect_view();

                let cards = snapshot
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(slot, view)| {
                        let (x, y) = place(slot);
                        let id = view.spec.id.clone();
                        let click_id = id.clone();
                        let compare_id = id.clone();
                        let status = status_class(view.status);
                        let status_label = status_class(view.status);
                        let label = view.spec.label.clone();
                        let turns = view.turns;
                        let snippet = truncate(&view.last_output, 100);
                        let selected = state.selected;
                        view! {
                            <div
                                class=move || {
                                    let active = selected.get().as_deref() == Some(compare_id.as_str());
                                    format!("node {status} {}", if active { "selected" } else { "" })
                                }
                                style=format!("left:{x:.2}%;top:{y:.2}%")
                                on:click=move |_| selected.set(Some(click_id.clone()))
                            >
                                <div class="node-head">
                                    <span class="node-label">{label}</span>
                                    <span class=format!("pill {status}")>{status_label}</span>
                                </div>
                                <div class="node-meta">{format!("{turns} turns")}</div>
                                <div class="node-snippet">
                                    {if snippet.is_empty() {
                                        "waiting for output".to_string()
                                    } else {
                                        snippet
                                    }}
                                </div>
                            </div>
                        }
                    })
                    .collect_view();

                view! {
                    <svg class="edges">{edges}</svg>
                    {cards}
                }
                .into_any()
            }}
        </div>
    }
}

fn start_screen(
    state: MobiusState,
    bus: Bus,
    goal: RwSignal<String>,
    analyze: impl Fn() + Clone + 'static,
) -> impl IntoView {
    let preset_cards = PRESETS
        .iter()
        .map(|preset| {
            let bus = bus.clone();
            view! {
                <div class="preset-card">
                    <div class="preset-name">{preset.name}</div>
                    <div class="preset-blurb">{preset.blurb}</div>
                    <button
                        class="btn primary"
                        on:click=move |_| {
                            presets::apply(&bus, preset);
                            state.kickoff.set(preset.kickoff.to_string());
                        }
                    >
                        "Stage this loop"
                    </button>
                </div>
            }
        })
        .collect_view();

    let suggest_bus = bus.clone();
    let suggestions = move || {
        let graphs = state.suggestions.get();
        if graphs.is_empty() {
            return ().into_any();
        }
        let bus = suggest_bus.clone();
        let cards = graphs
            .into_iter()
            .map(|graph| {
                let bus = bus.clone();
                let staged = graph.clone();
                view! {
                    <div class="preset-card suggested">
                        <div class="preset-name">{graph.name.clone()}</div>
                        <div class="preset-blurb">{graph.rationale.clone()}</div>
                        <button
                            class="btn primary"
                            on:click=move |_| {
                                presets::apply_suggested(&bus, &staged);
                                state.kickoff.set(staged.kickoff.clone());
                            }
                        >
                            "Stage this"
                        </button>
                    </div>
                }
            })
            .collect_view();
        view! {
            <div class="suggest-title">"Suggested for your goal"</div>
            <div class="preset-grid">{cards}</div>
        }
        .into_any()
    };

    let analyze_enter = analyze.clone();
    let analyze_click = analyze.clone();

    view! {
        <div class="graph-empty">
            <div class="graph-empty-title">"Start with a loop"</div>
            <div class="graph-empty-sub">
                "Set your workspace up top, then analyze it for a goal or stage a template. Nothing runs until you press Execute. You can also ask the conductor on the right."
            </div>
            <div class="analyze-bar">
                <input
                    class="ti analyze-goal"
                    placeholder="Optional goal, e.g. add unit tests to the parser (or just Analyze)"
                    prop:value=move || goal.get()
                    on:input=move |event| goal.set(event_target_value(&event))
                    on:keydown=move |event| {
                        if event.key() == "Enter" {
                            analyze_enter();
                        }
                    }
                />
                <button class="btn primary" on:click=move |_| analyze_click()>
                    "Analyze repo"
                </button>
            </div>
            <Show when=move || state.analyzing.get() fallback=|| ()>
                <div class="analyze-status">
                    <div class="analyze-status-head">
                        <span class="spinner"></span>
                        "Reading the repository and drafting workflows..."
                    </div>
                    <div class="analyze-progress">{move || state.analyze_progress.get()}</div>
                </div>
            </Show>
            {move || {
                state
                    .analyze_error
                    .get()
                    .map(|error| view! { <div class="analyze-error">{error}</div> })
            }}
            {suggestions}
            <div class="suggest-title muted-title">"Or start from a template"</div>
            <div class="preset-grid">{preset_cards}</div>
        </div>
    }
}
