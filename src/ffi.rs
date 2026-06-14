//! Bindings to the native Zig library (`libconvert-rs.so`).
//!
//! This module is compiled only for the server (`ssr`) build: the symbols live
//! in a native shared object and cannot exist in the wasm/hydrate target.
//! Layouts mirror the `extern struct`s in `convert-songs/src/root.zig`.
use crate::app::TrackCandidate;

#[repr(C)]
#[derive(Debug)]
pub struct Str {
    pub ptr: *const u8,
    pub len: usize,
}

impl From<&str> for Str {
    fn from(value: &str) -> Self {
        Str {
            ptr: value.as_ptr(),
            len: value.len(),
        }
    }
}

impl Str {
    fn to_owned_string(&self) -> String {
        if self.ptr.is_null() || self.len == 0 {
            return String::new();
        }
        let bytes = unsafe { std::slice::from_raw_parts(self.ptr, self.len) };
        String::from_utf8_lossy(bytes).into_owned()
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct Query {
    pub name: Str,
    pub album: Str,
    pub artist: Str,
}

impl Query {
    fn new(name: &str, album: &str, artist: &str) -> Self {
        Query {
            name: name.into(),
            album: album.into(),
            artist: artist.into(),
        }
    }
}

/// One possible Spotify match (mirrors Zig `Candidate`).
#[repr(C)]
#[derive(Debug)]
struct Candidate {
    name: Str,
    album: Str,
    artist: Str,
    uri: Str,
    image: Str,
}

/// Candidates for a single input track (mirrors Zig `CandidateList`).
#[repr(C)]
struct CandidateList {
    ptr: *mut Candidate,
    len: usize,
}

/// One `CandidateList` per input track (mirrors Zig `TrackResults`).
#[repr(C)]
struct TrackResults {
    ptr: *mut CandidateList,
    len: usize,
}

/// Borrowed slice of queries (mirrors Zig `QueryList`).
#[repr(C)]
struct QueryList {
    ptr: *const Query,
    len: usize,
}

/// Borrowed slice of strings, e.g. track URIs (mirrors Zig `StrList`).
#[repr(C)]
struct StrList {
    ptr: *const Str,
    len: usize,
}

unsafe extern "C" {
    fn query_songs(list: QueryList) -> TrackResults;
    fn spotify_authorize_url(redirect_uri: Str, state: Str) -> Str;
    fn exchange_code(code: Str, redirect_uri: Str) -> Str;
    fn create_playlist(token: Str, name: Str, description: Str, uris: StrList) -> Str;
}

/// Resolve a batch of tracks; each result is a list of up to 5 candidate
/// matches. The returned outer vector is index-aligned with `tracks`.
pub fn query_songs_safe(tracks: &[(String, String, String)]) -> Vec<Vec<TrackCandidate>> {
    // Each `Query` borrows the input strings; keep them (and `queries`) alive
    // until after the FFI call returns.
    let queries: Vec<Query> = tracks
        .iter()
        .map(|(name, album, artist)| Query::new(name, album, artist))
        .collect();
    let list = QueryList {
        ptr: queries.as_ptr(),
        len: queries.len(),
    };

    let mut out: Vec<Vec<TrackCandidate>> = vec![Vec::new(); tracks.len()];
    let results = unsafe { query_songs(list) };
    if !results.ptr.is_null() && results.len == tracks.len() {
        let lists = unsafe { std::slice::from_raw_parts(results.ptr, results.len) };
        for (dst, cl) in out.iter_mut().zip(lists) {
            if cl.ptr.is_null() || cl.len == 0 {
                continue;
            }
            let cands = unsafe { std::slice::from_raw_parts(cl.ptr, cl.len) };
            *dst = cands
                .iter()
                .map(|c| TrackCandidate {
                    name: c.name.to_owned_string(),
                    album: c.album.to_owned_string(),
                    artist: c.artist.to_owned_string(),
                    uri: c.uri.to_owned_string(),
                    image: c.image.to_owned_string(),
                })
                .collect();
        }
    }
    drop(queries); // keep borrowed strings alive across the call above
    out
}

/// Build the Spotify authorization URL for the authorization-code flow.
pub fn authorize_url(redirect_uri: &str, state: &str) -> String {
    let url = unsafe { spotify_authorize_url(redirect_uri.into(), state.into()) };
    url.to_owned_string()
}

/// Exchange an authorization code for a user access token. `None` on failure.
pub fn exchange_code_safe(code: &str, redirect_uri: &str) -> Option<String> {
    let token = unsafe { exchange_code(code.into(), redirect_uri.into()) };
    let token = token.to_owned_string();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

/// Create a playlist with the given track URIs. Returns its URL, `None` on failure.
pub fn create_playlist_safe(
    token: &str,
    name: &str,
    description: &str,
    uris: &[String],
) -> Option<String> {
    // The `Str`s borrow the input URIs; keep `strs` (and `uris`) alive across
    // the call.
    let strs: Vec<Str> = uris.iter().map(|u| u.as_str().into()).collect();
    let list = StrList {
        ptr: strs.as_ptr(),
        len: strs.len(),
    };
    let url = unsafe { create_playlist(token.into(), name.into(), description.into(), list) };
    let url = url.to_owned_string();
    drop(strs);
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}
