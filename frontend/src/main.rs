use log::info;
use sycamore::{
    futures::JsFuture,
    prelude::*,
    rt::{JsCast, JsValue},
    suspense::Suspense,
};
use web_sys::{window, Response};

async fn fetch_data() -> Result<JsValue, JsValue> {
    let window = window().expect("no global `window` exists");

    let response_promise = window.fetch_with_str("/api/traffic_log");
    let response = JsFuture::from(response_promise).await?;
    let response: Response = response.dyn_into().expect("Failed to convert to Response");

    if !response.ok() {
        return Err(JsValue::from_str("Fetch failed!"));
    }

    let json_promise = response.json()?;
    let json = JsFuture::from(json_promise).await?;
    Ok(json)
}

#[component]
async fn DataDisplayer<G: Html>(cx: Scope<'_>) -> View<G> {
    let data = match fetch_data().await {
        Ok(d) => {
            info!("{:?}", d);
            d.as_string().unwrap_or_default()
        }
        Err(_) => String::from("Error fetching data"),
    };

    view! { cx,
        div(class="container") { (data) }
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
