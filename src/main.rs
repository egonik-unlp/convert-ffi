#[cfg(feature = "ssr")]
fn main() {
    // The Zig library does blocking HTTPS (Spotify token refresh + search), and
    // Zig's TLS client puts very large buffers on the stack — far more than the
    // default 2 MiB tokio thread stack, which overflows. Give every runtime
    // thread (workers and the blocking pool) a generous stack.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(32 * 1024 * 1024)
        .build()
        .expect("failed to build tokio runtime");
    runtime.block_on(serve());
}

// Must exactly match a Redirect URI registered in the Spotify app dashboard.
// In production set REDIRECT_URI (e.g. https://<app>.onrender.com/callback).
// Falls back to the 127.0.0.1 loopback for local dev — Spotify allows http on
// the loopback (but not "localhost"), so browse the app at http://127.0.0.1:5000.
#[cfg(feature = "ssr")]
fn redirect_uri() -> String {
    std::env::var("REDIRECT_URI").unwrap_or_else(|_| "http://127.0.0.1:5000/callback".into())
}

#[cfg(feature = "ssr")]
async fn serve() {
    use axum::routing::get;
    use axum::Router;
    use convert_ffi::app::*;
    use leptos::logging::log;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};

    let conf = get_configuration(None).unwrap();
    // Hosts like Render assign the port via $PORT and expect the process to bind
    // 0.0.0.0:$PORT. When set, it overrides the site_addr from Cargo.toml.
    let mut addr = conf.leptos_options.site_addr;
    if let Ok(port) = std::env::var("PORT") {
        if let Ok(port) = port.parse::<u16>() {
            addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
        }
    }
    let leptos_options = conf.leptos_options;
    // Generate the list of routes in the Leptos app (server functions included).
    let routes = generate_route_list(App);

    let app = Router::new()
        .route("/login", get(login_handler))
        .route("/callback", get(callback_handler))
        .leptos_routes(&leptos_options, routes, {
            let leptos_options = leptos_options.clone();
            move || shell(leptos_options.clone())
        })
        .fallback(leptos_axum::file_and_error_handler(shell))
        .with_state(leptos_options);

    log!("listening on http://{}", &addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

/// `GET /login` — start the OAuth authorization-code flow by redirecting to
/// Spotify's consent screen.
#[cfg(feature = "ssr")]
async fn login_handler() -> axum::response::Redirect {
    use std::time::{SystemTime, UNIX_EPOCH};
    // A throwaway anti-CSRF state. Fine for a local single-user dev tool.
    let state = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_default();
    let url = convert_ffi::ffi::authorize_url(&redirect_uri(), &state);
    axum::response::Redirect::to(&url)
}

#[cfg(feature = "ssr")]
#[derive(serde::Deserialize)]
struct CallbackParams {
    code: Option<String>,
    #[allow(dead_code)]
    state: Option<String>,
    error: Option<String>,
}

/// `GET /callback` — Spotify redirects here with an authorization code. Exchange
/// it for a user token, stash it in an HttpOnly cookie, and return to the app.
#[cfg(feature = "ssr")]
async fn callback_handler(
    axum::extract::Query(params): axum::extract::Query<CallbackParams>,
) -> axum::response::Response {
    use axum::http::{header::SET_COOKIE, HeaderValue, StatusCode};
    use axum::response::{IntoResponse, Redirect};

    if let Some(err) = params.error {
        return (StatusCode::FORBIDDEN, format!("Spotify authorization denied: {err}"))
            .into_response();
    }
    let Some(code) = params.code else {
        return (StatusCode::BAD_REQUEST, "Missing authorization code").into_response();
    };

    // Token exchange does blocking HTTPS in Zig — keep it off the async reactor.
    let redirect = redirect_uri();
    let token = tokio::task::spawn_blocking(move || {
        convert_ffi::ffi::exchange_code_safe(&code, &redirect)
    })
    .await
    .ok()
    .flatten();

    let Some(token) = token else {
        return (StatusCode::BAD_GATEWAY, "Spotify token exchange failed").into_response();
    };

    // Short-lived (~1h) user token. HttpOnly so client JS can't read it.
    let cookie = format!("sp_token={token}; HttpOnly; Path=/; SameSite=Lax; Max-Age=3600");
    let mut resp = Redirect::to("/").into_response();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        resp.headers_mut().insert(SET_COOKIE, value);
    }
    resp
}

#[cfg(not(feature = "ssr"))]
pub fn main() {
    // The client entry point lives in lib.rs (`hydrate`); this binary is the
    // server. When built without `ssr` there is nothing to run here.
}
