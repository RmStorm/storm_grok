use serde_wasm_bindgen::from_value;
use shared_types::{RequestCycle, TrafficLog};
use sycamore::{
    futures::JsFuture,
    prelude::*,
    rt::{JsCast, JsValue},
    suspense::Suspense,
};
use web_sys::{window, Response};

async fn fetch_data() -> Result<TrafficLog, JsValue> {
    let window = window().expect("no global `window` exists");

    let response: Response = JsFuture::from(window.fetch_with_str("/api/traffic_log"))
        .await?
        .dyn_into()
        .expect("Failed to convert to Response");

    if !response.ok() {
        return Err(JsValue::from_str("Fetch failed!"));
    }

    let json = JsFuture::from(response.json()?).await?;
    let deserialized_data: TrafficLog = from_value(json)?;
    Ok(deserialized_data)
}

fn request_card_view<G: Html>(cx: Scope<'_>, request: RequestCycle) -> View<G> {
    let visible = create_signal(cx, false);

    view! {cx,
        div(class="card") {
            header(class="card-header") {
                p(class="card-header-title") {
                    (request.timestamp_in.to_string())
                }
                a(class="card-header-icon", role="button", on:click=move |_| {
                    visible.set(!*visible.get());
                }) {
                    span(class="icon") {
                        i(class="fas fa-angle-down", aria-hidden="true") {}
                    }
                }
            }
            div(class=format!("card-content {}", if *visible.get() { "" } else { "is-hidden" })) {
                div(class="content") {
                    p() { (request.request_head.method) }
                    p() { (request.request_head.uri) }
                    pre() { (base64::encode(&request.response_body)) }
                }
            }
        }
    }
}

#[component]
async fn DataDisplayer<G: Html>(cx: Scope<'_>) -> View<G> {
    let data = match fetch_data().await {
        Ok(d) => d,
        Err(_) => return view! { cx, "Error fetching data" },
    };

    view! { cx,
        div(class="container") {
            Keyed(
                iterable=create_signal(cx, data.requests),
                view=|cx, request| request_card_view(cx, request),
                key=|request| request.timestamp_in.timestamp() // Assuming timestamp_in is unique for each request
            )
        }
    }
}

#[component]
fn App<G: Html>(cx: Scope) -> View<G> {
    let name = create_signal(cx, String::new());

    let displayed_name = || {
        if name.get().is_empty() {
            "World".to_string()
        } else {
            name.get().as_ref().clone()
        }
    };

    view! { cx,
        section(class="section"){
            div(class="container") {
                h1(class="title") {
                    "Hello "
                    (displayed_name())
                    "!"
                }
                input(placeholder="What is your name??", bind:value=name)
                Suspense(fallback=view! { cx, "Loading..." }) {
                    DataDisplayer {}
                }
            }
        }
    }
}

fn main() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Debug).unwrap();

    sycamore::render(|cx| view! { cx, App {} });
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
    #[test]
    fn exploration() {
        assert_eq!(2 + 2, 4);
    }
    #[test]
    #[should_panic]
    fn another() {
        panic!("Make this test fail");
    }
    #[test]
    fn results_works() -> Result<(), String> {
        if 2 + 2 == 4 {
            Ok(())
        } else {
            Err(String::from("two plus two does not equal four"))
        }
    }
}
