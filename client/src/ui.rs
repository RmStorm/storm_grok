use parking_lot::RwLock;
use std::sync::Arc;

use app::App;
use async_trait::async_trait;
use axum::response::Response as AxumResponse;

use axum::{
    body::Body,
    extract::State,
    http::{Request, Response, StatusCode, Uri},
    response::IntoResponse,
    routing::get,
    Router,
};
use leptos::*;
use shared_types::TrafficLog;
use tower::ServiceExt;
use tower_http::services::ServeDir;

use leptos_axum::{generate_route_list, LeptosRoutes};

pub async fn file_and_error_handler(
    uri: Uri,
    State(options): State<LeptosOptions>,
    req: Request<Body>,
) -> AxumResponse {
    let root = options.site_root.clone();
    let res = get_static_file(uri.clone(), &root).await.unwrap();

    if res.status() == StatusCode::OK {
        res.into_response()
    } else {
        let handler =
            leptos_axum::render_app_to_stream(options.to_owned(), move || view! { <App/> });
        handler(req).await.into_response()
    }
}

async fn get_static_file(uri: Uri, root: &str) -> Result<Response<Body>, (StatusCode, String)> {
    let req = Request::builder()
        .uri(uri.clone())
        .body(Body::empty())
        .unwrap();
    // `ServeDir` implements `tower::Service` so we can call it with `tower::ServiceExt::oneshot`
    // This path is relative to the cargo root
    match ServeDir::new(root).oneshot(req).await {
        Ok(res) => Ok(res.map(Body::new)),
        Err(err) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {err}"),
        )),
    }
}

pub struct UiServer {
    name: String,
    traffic_log: Arc<RwLock<TrafficLog>>,
}

pub fn configure_ui_client(traffic_log: Arc<RwLock<TrafficLog>>) -> UiServer {
    UiServer {
        name: "uiserver".to_owned(),
        traffic_log,
    }
}

#[async_trait]
impl pingora::services::Service for UiServer {
    async fn start_service(
        &mut self,
        _fds: Option<pingora::server::ListenFds>,
        shutdown: pingora::server::ShutdownWatch,
    ) {
        log::info!("starting service {} {:?}", self.name(), shutdown);
        // Setting get_configuration(None) means we'll be using cargo-leptos's env values
        // For deployment these variables are:
        // <https://github.com/leptos-rs/start-axum#executing-a-server-on-a-remote-machine-without-the-toolchain>
        // Alternately a file can be specified such as Some("Cargo.toml")
        // The file would need to be included with the executable when moved to deployment
        let conf = get_configuration(None).await.unwrap();
        let leptos_options = conf.leptos_options;
        let addr = leptos_options.site_addr;
        let routes = generate_route_list(App);

        // intermediate variable is neccesary to prevent self from moving into the closure in the context
        let tl = self.traffic_log.clone();
        // build our application with a route
        let axum_app = Router::new()
            .leptos_routes_with_context(
                &leptos_options,
                routes,
                move || provide_context(tl.clone()),
                App,
            )
            .route("/oida", get(|| async { "Hello, World!" }))
            .fallback(file_and_error_handler)
            .with_state(leptos_options);

        // run our app with hyper
        log::info!("listening on http://{}", &addr);
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        log::info!("listener {:?}", &listener);
        axum::serve(listener, axum_app.into_make_service())
            .await
            .unwrap();
    }

    fn name(&self) -> &str {
        &self.name
    }
}
