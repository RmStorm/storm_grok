use std::{cmp::min, ops::Deref};

use crate::error_template::{AppError, ErrorTemplate};

use leptos::{logging, *};
use leptos_meta::*;
use leptos_router::*;
use shared_types::{RequestCycle, TrafficLog};

pub mod error_template;

const MAX_BODY_LEN: usize = 100000;

#[component]
pub fn App() -> impl IntoView {
    // Provides context that manages stylesheets, titles, meta tags, etc.
    provide_meta_context();
    view! {
        <Stylesheet id="leptos" href="/pkg/start-axum-workspace.css"/>
        <Title text="Welcome to Leptos"/>
        <Router fallback=|| {
            let mut outside_errors = Errors::default();
            outside_errors.insert_with_default_key(AppError::NotFound);
            view! { <ErrorTemplate outside_errors/> }.into_view()
        }>
            <main>
                <Routes>
                    <Route path="" view=HomePage/>
                </Routes>
            </main>
        </Router>
    }
}

#[component]
fn HomePage() -> impl IntoView {
    view! {
        <section class="text-gray-700 body-font">
            <div class="container mx-auto px-4">
                <h1 class="text-2xl p-4">"Sgrok Request Overview"</h1>
                <TrafficLogSuspense/>
            </div>
        </section>
    }
}

#[server]
pub async fn get_traffic_log() -> Result<TrafficLog, ServerFnError> {
    let state = expect_context::<std::sync::Arc<parking_lot::RwLock<TrafficLog>>>();
    let traffic_log = { state.deref().read().clone() };
    Ok(traffic_log)
}

#[component]
fn TrafficLogSuspense() -> impl IntoView {
    // new resource with no dependencies (it will only called once)
    let traffic_log = create_resource(|| (), |_| async { get_traffic_log().await });
    // logging::log!("displaying trafficlog");
    view! {
        <Suspense fallback=move || view! { <p>"Loading Trafficlog"</p> }>
            <ErrorBoundary fallback=|_e| {
                view! { <p>"Could not load trafficlog"</p> }
            }>
                {move || {
                    traffic_log
                        .get()
                        .map(move |x| {
                            x.map(move |y| {
                                TrafficLogRequests(TrafficLogRequestsProps {
                                    reqs: y.requests,
                                })
                            })
                        })
                }}

            </ErrorBoundary>
        </Suspense>
    }
}

#[component]
fn TrafficLogRequests(
    reqs: Vec<RequestCycle>) -> impl IntoView {
    let (tlog, _set_tlog) = create_signal(reqs);
    view! {
        <div class="space-y-4">
            <For
                each=move || tlog.get()
                key=|cycle| cycle.timestamp_in.timestamp()
                children=move |cycle: shared_types::RequestCycle| {
                    view! { <RequestsCycleView cycle=cycle/> }
                }
            />

        </div>
    }
}

#[component]
fn RequestsCycleView(cycle: RequestCycle) -> impl IntoView {
    let collapsed  = create_rw_signal(false);
    let hidden_req_headers = create_rw_signal(true);
    let hidden_resp_headers = create_rw_signal(true);
    let hidden_body = create_rw_signal(true);

    let body_len = cycle.response_body.len();
    let response_str = String::from_utf8_lossy(&cycle.response_body[..min(body_len, MAX_BODY_LEN)]).into_owned();
    view! {
        <div class="border p-4 rounded-lg shadow">
            <button
                class="w-full text-left"
                on:click=move |_| {
                    logging::log!("collapsed: {:?}", collapsed());
                    collapsed.update(|b| *b = !*b);
                }
            >

                <p class="font-bold">{"Timestamp: "} {cycle.timestamp_in.to_string()}</p>
                <p>{cycle.request_head.uri}</p>
            </button>
            <div class:hidden=move || collapsed()>
                <div>
                    <button
                        class="font-semibold"
                        on:click=move |_| { hidden_req_headers.update(|b| *b = !*b) }
                    >
                        "Display request headers"
                    </button>
                    <div class:hidden=move || hidden_req_headers()>
                        <Headers headers=cycle.request_head.headers/>
                    </div>
                </div>
                <div>
                    <button
                        class="font-semibold"
                        on:click=move |_| { hidden_resp_headers.update(|b| *b = !*b) }
                    >
                        "Display response headers"
                    </button>
                    <div class:hidden=move || hidden_resp_headers()>
                        <Headers headers=cycle.response_head.headers/>
                    </div>
                </div>
                <div class:hidden=move || body_len == 0>
                    <button
                        class="font-semibold"
                        on:click=move |_| { hidden_body.update(|b| *b = !*b) }
                    >
                        "Display Request Body"
                    </button>
                    <div class:hidden=move || hidden_body()>
                        <p class="pl-4 text-sm">
                            {response_str}
                            <span class:hidden=move || body_len < MAX_BODY_LEN class="text-red-500">
                                (response truncated)
                            </span>
                        </p>
                    </div>
                </div>
            </div>
        </div>
    }
}

#[component]
fn Headers(
    headers: Vec<(String, String)>) -> impl IntoView {
    view! { <ul>{headers.into_iter().map(|(k, v)| view! { <li>{k} : {v}</li> }).collect_view()}</ul> }
}

/// Shows progress toward a goal.
#[component]
fn ProgressBar(
    /// The maximum value of the progress bar.
    #[prop(default = 100)]
    max: u16,
    /// How much progress should be displayed.
    #[prop(into)]
    progress: Signal<i32>,
) -> impl IntoView {
    view! {
        <progress
            max=max
            // now this works
            value=progress
        ></progress>
    }
}

