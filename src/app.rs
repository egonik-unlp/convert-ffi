use leptos::prelude::*;
use leptos::server_fn::codec::Json;
use leptos::task::spawn_local;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{Route, Router, Routes},
    StaticSegment,
};
use serde::{Deserialize, Serialize};

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <link rel="preconnect" href="https://fonts.googleapis.com" />
                <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin="" />
                <link
                    rel="stylesheet"
                    href="https://fonts.googleapis.com/css2?family=Bricolage+Grotesque:opsz,wght@12..96,400..800&family=Hanken+Grotesk:wght@400;500;600;700&family=Martian+Mono:wght@400;500;600&display=swap"
                />
                <AutoReload options=options.clone() />
                <HydrationScripts options />
                <MetaTags />
            </head>
            <body>
                <App />
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();
    view! {
        <Stylesheet id="leptos" href="/pkg/convert-ffi.css" />
        <Title text="convert-songs" />
        <Router>
            <main>
                <Routes fallback=|| "Page not found.".into_view()>
                    <Route path=StaticSegment("") view=HomePage />
                </Routes>
            </main>
        </Router>
    }
}

/// Locally-parsed tags for one file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackMatch {
    pub name: String,
    pub album: String,
    pub artist: String,
}

/// One candidate Spotify match for a track.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackCandidate {
    pub name: String,
    pub album: String,
    pub artist: String,
    pub uri: String,
    pub image: String,
}

/// One row in the UI: a picked file, its locally-parsed tags, the candidate
/// Spotify matches, and which candidate the user picked (None = skip).
#[derive(Clone, Debug, Default, PartialEq)]
struct Row {
    file: String,
    parsed: TrackMatch,
    candidates: Vec<TrackCandidate>,
    selected: Option<usize>,
    resolved: bool,
    error: Option<String>,
    /// Whether the alternatives list is open. Resolved rows collapse to their
    /// chosen match by default; the user expands only to change it.
    expanded: bool,
}

/// Which slice of rows the list is showing.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Filter {
    #[default]
    All,
    Review,
    Matched,
    Skipped,
}

/// The triage state of a single row, derived from its resolution.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Resolving,
    Matched,
    Skipped,
    Review,
}

fn kind(r: &Row) -> Kind {
    if r.error.is_some() {
        Kind::Review
    } else if !r.resolved {
        Kind::Resolving
    } else if r.candidates.is_empty() {
        Kind::Review
    } else if r.selected.is_some() {
        Kind::Matched
    } else {
        Kind::Skipped
    }
}

/// The four stages of the flow. Ordering matters: cast to i8 gives slide
/// direction (a higher target slides in from the right).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Add,
    Match,
    Preview,
    Done,
}

/// Turn a Spotify playlist URL or URI into its embeddable iframe src.
fn embed_src(url: &str) -> Option<String> {
    let id = url
        .rsplit(['/', ':'])
        .next()?
        .split(['?', '#'])
        .next()?;
    (!id.is_empty()).then(|| format!("https://open.spotify.com/embed/playlist/{id}"))
}

/// Resolve a batch of locally-parsed tags against Spotify via the native Zig
/// library, returning up to 5 candidates per track (index-aligned with input).
/// Only the tag strings cross the wire — never the audio.
#[server(name = ResolveTracks, prefix = "/api", input = Json, output = Json)]
pub async fn resolve_tracks(
    tracks: Vec<TrackMatch>,
) -> Result<Vec<Vec<TrackCandidate>>, ServerFnError> {
    use crate::ffi::query_songs_safe;
    // Blocking HTTP + TLS in Zig — keep it off the async reactor. One call for
    // the whole batch, so Zig authenticates with Spotify once.
    let resolved = tokio::task::spawn_blocking(move || {
        let queries: Vec<(String, String, String)> = tracks
            .into_iter()
            .map(|t| (t.name, t.album, t.artist))
            .collect();
        query_songs_safe(&queries)
    })
    .await
    .map_err(|e| ServerFnError::new(format!("resolve task failed: {e}")))?;
    Ok(resolved)
}

/// Whether the current session has a Spotify user token (set by /callback).
#[server(name = AuthStatus, prefix = "/api")]
pub async fn auth_status() -> Result<bool, ServerFnError> {
    Ok(cookie_value("sp_token").await.is_some())
}

/// Diagnostics surfaced in the UI to debug deployments where search returns no
/// matches. Reports whether the Spotify creds are present in the *server* process
/// environment (the same env the native `.so` reads), plus a live token-fetch
/// probe through the `.so`. Never echoes secret values — only presence/length.
#[server(name = Diagnostics, prefix = "/api")]
pub async fn diagnostics() -> Result<String, ServerFnError> {
    let present = |k: &str| match std::env::var(k) {
        Ok(v) if !v.is_empty() => format!("set ({} chars)", v.len()),
        Ok(_) => "set but EMPTY".to_string(),
        Err(_) => "MISSING".to_string(),
    };
    let id = present("SPOTIFY_CLIENT_ID");
    let secret = present("SPOTIFY_CLIENT_SECRET");
    let redirect = std::env::var("REDIRECT_URI").unwrap_or_else(|_| "(unset → loopback default)".into());
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "(unknown)".into());
    let dump = std::path::Path::new("dump.a").exists();
    // The probe does blocking HTTPS inside Zig — keep it off the async reactor.
    let probe = tokio::task::spawn_blocking(crate::ffi::debug_probe_safe)
        .await
        .unwrap_or_else(|e| format!("probe task failed: {e}"));
    Ok(format!(
        "SPOTIFY_CLIENT_ID: {id}\n\
         SPOTIFY_CLIENT_SECRET: {secret}\n\
         REDIRECT_URI: {redirect}\n\
         cwd: {cwd}\n\
         dump.a cached: {dump}\n\
         zig token probe: {probe}"
    ))
}

/// Create a Spotify playlist from the selected track URIs, using the user token
/// stashed in the session cookie.
#[server(name = CreatePlaylist, prefix = "/api", input = Json, output = Json)]
pub async fn create_playlist(name: String, uris: Vec<String>) -> Result<String, ServerFnError> {
    let token = cookie_value("sp_token")
        .await
        .ok_or_else(|| ServerFnError::new("Not connected to Spotify"))?;
    if uris.is_empty() {
        return Err(ServerFnError::new("No tracks selected"));
    }
    let url = tokio::task::spawn_blocking(move || {
        crate::ffi::create_playlist_safe(&token, &name, "Created with convert-songs", &uris)
    })
    .await
    .map_err(|e| ServerFnError::new(format!("create task failed: {e}")))?;
    url.ok_or_else(|| ServerFnError::new("Playlist creation failed"))
}

/// Read a cookie value off the incoming request (server only).
#[cfg(feature = "ssr")]
async fn cookie_value(name: &str) -> Option<String> {
    use leptos_axum::extract;
    let headers: axum::http::HeaderMap = extract().await.ok()?;
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    let prefix = format!("{name}=");
    raw.split(';')
        .map(|s| s.trim())
        .find_map(|kv| kv.strip_prefix(&prefix).map(|v| v.to_string()))
}

/// Parse audio tags (title/album/artist) from raw file bytes, in the browser.
#[cfg(feature = "hydrate")]
fn parse_metadata(bytes: &[u8]) -> TrackMatch {
    use lofty::file::TaggedFileExt;
    use lofty::probe::Probe;
    use lofty::tag::Accessor;
    use std::io::Cursor;

    let Ok(probe) = Probe::new(Cursor::new(bytes)).guess_file_type() else {
        return TrackMatch::default();
    };
    let Ok(tagged) = probe.read() else {
        return TrackMatch::default();
    };
    let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) else {
        return TrackMatch::default();
    };
    TrackMatch {
        name: tag.title().map(|c| c.to_string()).unwrap_or_default(),
        album: tag.album().map(|c| c.to_string()).unwrap_or_default(),
        artist: tag.artist().map(|c| c.to_string()).unwrap_or_default(),
    }
}

/// The brand mark: a tuner dial whose indicator sweeps while it scans.
#[component]
fn Mark(#[prop(default = 40)] size: u32) -> impl IntoView {
    view! {
        <svg
            class="mark"
            width=size
            height=size
            viewBox="0 0 48 48"
            fill="none"
            aria-hidden="true"
        >
            <circle class="ring" cx="24" cy="24" r="18" />
            <circle class="ring inner" cx="24" cy="24" r="11" />
            <g class="scan">
                <line class="tick" x1="24" y1="3.5" x2="24" y2="11" />
            </g>
            <circle class="hub" cx="24" cy="24" r="3" />
        </svg>
    }
}

#[component]
fn HomePage() -> impl IntoView {
    let rows = RwSignal::new(Vec::<Row>::new());
    let playlist_name = RwSignal::new(String::from("convert-songs playlist"));
    let authed = RwSignal::new(false);
    let creating = RwSignal::new(false);
    let create_result = RwSignal::new(None::<Result<String, String>>);
    // (current step, slide direction): one signal so a step change is atomic.
    let nav = RwSignal::new((Step::Add, 1i8));
    // Spotify's embed CDN lags a few seconds behind playlist creation, so a fresh
    // playlist renders empty if the iframe mounts immediately. Gate the embed on
    // this flag, flipped by a short timer once the playlist is created.
    let embed_ready = RwSignal::new(false);
    // Diagnostics panel output (None = not run yet).
    let diag = RwSignal::new(None::<String>);
    let run_diag = move |_: leptos::web_sys::MouseEvent| {
        diag.set(Some("running…".to_string()));
        spawn_local(async move {
            let out = diagnostics()
                .await
                .unwrap_or_else(|e| format!("diagnostics call failed: {e}"));
            diag.set(Some(out));
        });
    };

    // Check auth status once, client-side (Effects don't run during SSR).
    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(status) = auth_status().await {
                authed.set(status);
            }
        });
    });

    let on_change = move |ev: leptos::web_sys::Event| {
        // File reading + tag parsing happen only in the browser (wasm).
        #[cfg(feature = "hydrate")]
        {
            use wasm_bindgen::JsCast;

            let input: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
            let Some(file_list) = input.files() else {
                return;
            };

            let mut pending = Vec::new();
            for i in 0..file_list.length() {
                let Some(file) = file_list.get(i) else {
                    continue;
                };
                let id = rows.with_untracked(|r| r.len());
                rows.update(|r| {
                    r.push(Row {
                        file: file.name(),
                        ..Default::default()
                    })
                });
                pending.push((id, gloo_file::Blob::from(file)));
            }

            // Adding files carries the user into the matching step.
            if !pending.is_empty() {
                nav.set((Step::Match, 1));
            }

            spawn_local(async move {
                let mut ids = Vec::new();
                let mut queries = Vec::new();
                for (id, blob) in pending {
                    match gloo_file::futures::read_as_bytes(&blob).await {
                        Ok(bytes) => {
                            let parsed = parse_metadata(&bytes);
                            rows.update(|r| {
                                if let Some(row) = r.get_mut(id) {
                                    row.parsed = parsed.clone();
                                }
                            });
                            ids.push(id);
                            queries.push(parsed);
                        }
                        Err(e) => rows.update(|r| {
                            if let Some(row) = r.get_mut(id) {
                                row.error = Some(format!("read error: {e:?}"));
                                row.resolved = true;
                            }
                        }),
                    }
                }
                if queries.is_empty() {
                    return;
                }

                match resolve_tracks(queries).await {
                    Ok(results) => rows.update(move |r| {
                        for (id, cands) in ids.into_iter().zip(results) {
                            if let Some(row) = r.get_mut(id) {
                                row.selected = if cands.is_empty() { None } else { Some(0) };
                                row.candidates = cands;
                                row.resolved = true;
                            }
                        }
                    }),
                    Err(e) => rows.update(move |r| {
                        let msg = e.to_string();
                        for id in ids {
                            if let Some(row) = r.get_mut(id) {
                                row.error = Some(msg.clone());
                                row.resolved = true;
                            }
                        }
                    }),
                }
            });
        }
        #[cfg(not(feature = "hydrate"))]
        {
            let _ = ev;
            let _ = rows;
        }
    };

    let on_create = move |_: leptos::web_sys::MouseEvent| {
        let name = playlist_name.get_untracked();
        let uris: Vec<String> = rows.with_untracked(|rs| {
            rs.iter()
                .filter_map(|row| {
                    row.selected
                        .and_then(|i| row.candidates.get(i))
                        .map(|c| c.uri.clone())
                })
                .collect()
        });
        if uris.is_empty() {
            create_result.set(Some(Err("Select at least one track first".to_string())));
            return;
        }
        creating.set(true);
        create_result.set(None);
        spawn_local(async move {
            let res = create_playlist(name, uris).await.map_err(|e| e.to_string());
            creating.set(false);
            if res.is_ok() {
                nav.set((Step::Done, 1));
                // Hold the embed back until Spotify has indexed the new playlist.
                embed_ready.set(false);
                set_timeout(
                    move || embed_ready.set(true),
                    std::time::Duration::from_secs(4),
                );
            }
            create_result.set(Some(res));
        });
    };

    let filter = RwSignal::new(Filter::All);

    let total = move || rows.with(|rs| rs.len());
    // (matched, to-review, skipped, resolving)
    let counts = move || {
        rows.with(|rs| {
            let mut c = (0usize, 0usize, 0usize, 0usize);
            for r in rs {
                match kind(r) {
                    Kind::Matched => c.0 += 1,
                    Kind::Review => c.1 += 1,
                    Kind::Skipped => c.2 += 1,
                    Kind::Resolving => c.3 += 1,
                }
            }
            c
        })
    };

    let step = move || nav.get().0;
    let dir = move || nav.get().1;
    let go = move |t: Step| {
        let cur = nav.get_untracked().0 as i8;
        nav.set((t, if (t as i8) >= cur { 1 } else { -1 }));
    };
    let matched_count = move || counts().0;
    let resolving_count = move || counts().3;
    let reset = move |_: leptos::web_sys::MouseEvent| {
        rows.set(Vec::new());
        create_result.set(None);
        nav.set((Step::Add, -1));
    };

    view! {
        <header class="masthead">
            <div class="brand">
                <Mark size=40 />
                <span class="wordmark">"convert"<b>"songs"</b></span>
            </div>
        </header>

        {move || {
            let cur = step();
            let has_rows = total() > 0;
            let has_match = matched_count() > 0;
            let is_done = create_result.get().map(|r| r.is_ok()).unwrap_or(false);
            let item = move |s: Step, n: &'static str, label: &'static str, enabled: bool| {
                let active = cur == s;
                let done = (s as i8) < (cur as i8);
                view! {
                    <button
                        class="stepper-item"
                        class:active=active
                        class:done=done
                        disabled=!enabled
                        on:click=move |_: leptos::web_sys::MouseEvent| { if enabled { go(s); } }
                    >
                        <span class="stepper-n">{n}</span>
                        <span class="stepper-label">{label}</span>
                    </button>
                }
            };
            view! {
                <nav class="stepper" aria-label="Progress">
                    {item(Step::Add, "1", "Add", true)}
                    {item(Step::Match, "2", "Match", has_rows)}
                    {item(Step::Preview, "3", "Preview", has_match)}
                    {item(Step::Done, "4", "Done", is_done)}
                </nav>
            }
        }}

        <div class="viewport">

        {move || {
            (step() == Step::Add)
                .then(|| {
                    let pcls = if dir() >= 0 { "pane fwd" } else { "pane back" };
                    view! {
                        <section class=pcls>
                            <p class="lede">
                                "The music sitting on your drive, back in rotation. Point Convert
                                Songs at your local files, it finds each track on Spotify, and you
                                pick the right matches — then it builds the playlist."
                            </p>
                            <label class="picker">
                                <input type="file" multiple accept="audio/*" on:change=on_change />
                                <span class="picker-cta">"Choose music files"</span>
                                <span class="picker-hint">
                                    "MP3 · FLAC · M4A — pick as many as you like"
                                </span>
                            </label>
                            {move || {
                                if authed.get() {
                                    view! { <p class="connect-hint ok">"✓ Spotify connected"</p> }
                                        .into_any()
                                } else {
                                    view! {
                                        <p class="connect-hint">
                                            "Saving to your account? "
                                            <a href="/login" rel="external">"Connect Spotify"</a>
                                            " now, before you add files."
                                        </p>
                                    }
                                        .into_any()
                                }
                            }}
                            <details class="diag">
                                <summary>"Diagnostics"</summary>
                                <button class="diag-run" on:click=run_diag>
                                    "Run server diagnostics"
                                </button>
                                {move || {
                                    diag.get()
                                        .map(|t| view! { <pre class="diag-out">{t}</pre> })
                                }}
                            </details>
                        </section>
                    }
                })
        }}

        {move || {
            (step() == Step::Preview)
                .then(|| {
                    let pcls = if dir() >= 0 { "pane fwd" } else { "pane back" };
                    view! {
                        <section class=pcls>
                            <div class="pane-head">
                                <h2 class="pane-title">"Review your playlist"</h2>
                                <p class="pane-sub">
                                    {move || {
                                        let m = matched_count();
                                        let ex = total().saturating_sub(m);
                                        if ex > 0 {
                                            format!("{m} tracks · {ex} not included")
                                        } else {
                                            format!("{m} tracks")
                                        }
                                    }}
                                </p>
                            </div>
                            <div class="field">
                                <label for="pl-name-input">"Playlist name"</label>
                                <input
                                    id="pl-name-input"
                                    class="pl-name"
                                    type="text"
                                    prop:value=move || playlist_name.get()
                                    on:input=move |ev| playlist_name.set(event_target_value(&ev))
                                />
                            </div>
                            <ol class="preview-list">
                                {move || {
                                    rows.get()
                                        .into_iter()
                                        .filter(|r| kind(r) == Kind::Matched)
                                        .filter_map(|r| {
                                            r.selected.and_then(|i| r.candidates.get(i).cloned()).map(|c| {
                                                let sub = [c.artist.clone(), c.album.clone()]
                                                    .into_iter()
                                                    .filter(|x| !x.is_empty())
                                                    .collect::<Vec<_>>()
                                                    .join(" · ");
                                                let art = if c.image.is_empty() {
                                                    view! { <span class="art art-ph" aria-hidden="true">"♪"</span> }
                                                        .into_any()
                                                } else {
                                                    view! { <img class="art" src=c.image.clone() alt="" /> }
                                                        .into_any()
                                                };
                                                view! {
                                                    <li class="pv-row">
                                                        {art}
                                                        <div class="song-meta">
                                                            <span class="name">{c.name.clone()}</span>
                                                            <span class="sub">{sub}</span>
                                                        </div>
                                                    </li>
                                                }
                                                    .into_any()
                                            })
                                        })
                                        .collect_view()
                                }}
                            </ol>
                            {move || {
                                create_result
                                    .get()
                                    .and_then(|r| r.err())
                                    .map(|e| view! { <p class="result error">{format!("Error: {e}")}</p> })
                            }}
                            <div class="wizard-nav">
                                <button
                                    class="btn-ghost"
                                    on:click=move |_: leptos::web_sys::MouseEvent| go(Step::Match)
                                >
                                    "← Back to matches"
                                </button>
                                {move || {
                                    if authed.get() {
                                        view! {
                                            <button
                                                class="btn-primary"
                                                on:click=on_create
                                                disabled=move || creating.get()
                                            >
                                                {move || {
                                                    if creating.get() { "Creating…" } else { "Create playlist" }
                                                }}
                                            </button>
                                        }
                                            .into_any()
                                    } else {
                                        view! {
                                            <a class="btn-primary" href="/login" rel="external">
                                                "Connect Spotify to create it"
                                            </a>
                                        }
                                            .into_any()
                                    }
                                }}
                            </div>
                        </section>
                    }
                })
        }}

        {move || {
            (step() == Step::Match && total() > 0)
                .then(|| {
                    let (m, rev, sk, res) = counts();
                    let mut parts: Vec<String> = Vec::new();
                    if res > 0 {
                        parts.push(format!("{res} resolving"));
                    }
                    parts.push(format!("{m} matched"));
                    if rev > 0 {
                        parts.push(format!("{rev} to review"));
                    }
                    if sk > 0 {
                        parts.push(format!("{sk} skipped"));
                    }
                    let summary_text = parts.join(" · ");
                    let chip = move |f: Filter, label: &'static str, n: usize| {
                        view! {
                            <button
                                class="chip"
                                class:active=move || filter.get() == f
                                on:click=move |_| filter.set(f)
                            >
                                {label}
                                <span class="chip-n">{n}</span>
                            </button>
                        }
                    };
                    let pcls = if dir() >= 0 { "summary pane fwd" } else { "summary pane back" };
                    view! {
                        <div class=pcls>
                            <span class="summary-counts">{summary_text}</span>
                            <div class="filters" role="group" aria-label="Filter tracks">
                                {chip(Filter::All, "All", m + rev + sk + res)}
                                {chip(Filter::Review, "To review", rev)}
                                {chip(Filter::Matched, "Matched", m)}
                                {chip(Filter::Skipped, "Skipped", sk)}
                            </div>
                        </div>
                    }
                })
        }}

        {move || {
            (step() == Step::Match)
                .then(|| {
                    let pcls = if dir() >= 0 { "songs pane fwd" } else { "songs pane back" };
                    view! {
                        <div class=pcls>
            {move || {
                let active = filter.get();
                rows.get()
                    .into_iter()
                    .enumerate()
                    .filter_map(|(id, row)| {
                        let k = kind(&row);
                        let show = match active {
                            Filter::All => true,
                            Filter::Review => k == Kind::Review,
                            Filter::Matched => k == Kind::Matched,
                            Filter::Skipped => k == Kind::Skipped,
                        };
                        if !show {
                            return None;
                        }

                        let display_name = if row.parsed.name.is_empty() {
                            row.file.clone()
                        } else {
                            row.parsed.name.clone()
                        };
                        let sub = [row.parsed.artist.clone(), row.parsed.album.clone()]
                            .into_iter()
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join(" · ");

                        let expandable = matches!(k, Kind::Matched | Kind::Skipped);
                        let expanded = row.expanded;

                        // Collapsed header reflects the chosen match, not the file.
                        let (art_url, primary, secondary) = match k {
                            Kind::Matched => {
                                match row.selected.and_then(|i| row.candidates.get(i)) {
                                    Some(c) => {
                                        let s = [c.artist.clone(), c.album.clone()]
                                            .into_iter()
                                            .filter(|x| !x.is_empty())
                                            .collect::<Vec<_>>()
                                            .join(" · ");
                                        (c.image.clone(), c.name.clone(), s)
                                    }
                                    None => (String::new(), display_name.clone(), sub.clone()),
                                }
                            }
                            _ => (String::new(), display_name.clone(), sub.clone()),
                        };

                        let art = if art_url.is_empty() {
                            view! { <span class="art art-ph" aria-hidden="true">"♪"</span> }
                                .into_any()
                        } else {
                            view! { <img class="art" src=art_url alt="" /> }.into_any()
                        };

                        let tag = match k {
                            Kind::Matched => {
                                view! { <span class="tag tag-ok">"✓ matched"</span> }.into_any()
                            }
                            Kind::Skipped => {
                                view! { <span class="tag tag-muted">"skipped"</span> }.into_any()
                            }
                            Kind::Resolving => {
                                view! { <span class="tag tag-work">"resolving…"</span> }.into_any()
                            }
                            Kind::Review => {
                                if let Some(e) = row.error.clone() {
                                    view! { <span class="pill pill-err" title=e>"error"</span> }
                                        .into_any()
                                } else {
                                    view! { <span class="pill pill-amber">"no match"</span> }
                                        .into_any()
                                }
                            }
                        };

                        let chev = expandable.then(|| {
                            view! {
                                <svg
                                    class="chev"
                                    class:open=expanded
                                    width="16"
                                    height="16"
                                    viewBox="0 0 24 24"
                                    fill="none"
                                    aria-hidden="true"
                                >
                                    <path d="M6 9l6 6 6-6" />
                                </svg>
                            }
                        });

                        let inner = view! {
                            {art}
                            <div class="song-meta">
                                <span class="name">{primary}</span>
                                <span class="sub">{secondary}</span>
                            </div>
                            <span class="file">{row.file.clone()}</span>
                            {tag}
                            {chev}
                        };

                        let header = if expandable {
                            let ae = if expanded { "true" } else { "false" };
                            view! {
                                <button
                                    class="song-row"
                                    class:open=expanded
                                    aria-expanded=ae
                                    on:click=move |_: leptos::web_sys::MouseEvent| {
                                        rows.update(|rs| {
                                            if let Some(r) = rs.get_mut(id) {
                                                r.expanded = !r.expanded;
                                            }
                                        });
                                    }
                                >
                                    {inner}
                                </button>
                            }
                                .into_any()
                        } else {
                            view! { <div class="song-row static">{inner}</div> }.into_any()
                        };

                        let alts = (expandable && expanded).then(|| {
                            let selected = row.selected;
                            let mut items: Vec<AnyView> = row
                                .candidates
                                .iter()
                                .cloned()
                                .enumerate()
                                .map(|(ci, c)| {
                                    let checked = selected == Some(ci);
                                    let image = c.image.clone();
                                    let label = format!("{} — {} — {}", c.name, c.artist, c.album);
                                    view! {
                                        <label class="cand" class:selected=checked>
                                            <input
                                                type="radio"
                                                name=format!("row-{id}")
                                                prop:checked=checked
                                                on:change=move |_: leptos::web_sys::Event| {
                                                    rows.update(|rs| {
                                                        if let Some(r) = rs.get_mut(id) {
                                                            r.selected = Some(ci);
                                                            r.expanded = false;
                                                        }
                                                    });
                                                }
                                            />
                                            {(!image.is_empty())
                                                .then(|| view! { <img class="art" src=image.clone() alt="" /> })}
                                            <span class="cand-text">{label}</span>
                                        </label>
                                    }
                                        .into_any()
                                })
                                .collect();
                            let skip_checked = selected.is_none();
                            items.push(
                                view! {
                                    <label class="cand skip" class:selected=skip_checked>
                                        <input
                                            type="radio"
                                            name=format!("row-{id}")
                                            prop:checked=skip_checked
                                            on:change=move |_: leptos::web_sys::Event| {
                                                rows.update(|rs| {
                                                    if let Some(r) = rs.get_mut(id) {
                                                        r.selected = None;
                                                        r.expanded = false;
                                                    }
                                                });
                                            }
                                        />
                                        <span class="cand-text">"Skip this track"</span>
                                    </label>
                                }
                                    .into_any(),
                            );
                            view! { <div class="alternatives">{items}</div> }
                        });

                        let is_review = matches!(k, Kind::Review);
                        Some(
                            view! {
                                <div class="song" class:open=expanded class:flag=is_review>
                                    {header}
                                    {alts}
                                </div>
                            }
                            .into_any(),
                        )
                    })
                    .collect_view()
            }}
                        </div>
                    }
                })
        }}

        {move || {
            (step() == Step::Match)
                .then(|| {
                    let pcls = if dir() >= 0 { "wizard-nav pane fwd" } else { "wizard-nav pane back" };
                    view! {
                        <div class=pcls>
                            <button
                                class="btn-ghost"
                                on:click=move |_: leptos::web_sys::MouseEvent| go(Step::Add)
                            >
                                "← Add more"
                            </button>
                            <button
                                class="btn-primary"
                                disabled=move || (resolving_count() > 0 || matched_count() == 0)
                                on:click=move |_: leptos::web_sys::MouseEvent| go(Step::Preview)
                            >
                                {move || {
                                    if resolving_count() > 0 {
                                        "Finding matches…"
                                    } else {
                                        "Continue to preview →"
                                    }
                                }}
                            </button>
                        </div>
                    }
                })
        }}

        {move || {
            (step() == Step::Done)
                .then(|| {
                    let pcls = if dir() >= 0 { "pane fwd" } else { "pane back" };
                    let link = create_result.get().and_then(|r| r.ok());
                    let embed = link.clone().and_then(|u| embed_src(&u));
                    view! {
                        <section class=pcls>
                            <div class="done">
                                <h2 class="pane-title">"Your playlist is live"</h2>
                                <p class="pane-sub">"Added to your Spotify account."</p>
                                {embed
                                    .map(|e| {
                                        move || {
                                            if embed_ready.get() {
                                                view! {
                                                    <iframe
                                                        class="sp-embed"
                                                        src=e.clone()
                                                        width="100%"
                                                        height="380"
                                                    ></iframe>
                                                }
                                                    .into_any()
                                            } else {
                                                view! {
                                                    <div class="sp-embed sp-embed-loading">
                                                        <Mark size=28 />
                                                        <span>"Preparing your playlist preview…"</span>
                                                    </div>
                                                }
                                                    .into_any()
                                            }
                                        }
                                    })}
                                <div class="wizard-nav center">
                                    {link
                                        .map(|u| {
                                            view! {
                                                <a class="btn-ghost" href=u target="_blank" rel="noreferrer">
                                                    "Open in Spotify"
                                                </a>
                                            }
                                        })}
                                    <button class="btn-primary" on:click=reset>"Convert another"</button>
                                </div>
                            </div>
                        </section>
                    }
                })
        }}

        </div>
    }
}
