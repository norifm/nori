# innertube-music

YouTube Music over InnerTube, in Rust, without a JavaScript engine:

- search, albums, artists, playlists, Home and related-track radio through the `WEB_REMIX` client (music.youtube.com);
- audio streams through the visionOS client, whose URLs need no signature deciphering, n-transform or PO token.

The crate does no I/O. It builds requests and parses answers into plain records (`Song`, `Album`,
`Artist`, `Playlist`, `Stream`) that keep YouTube's own ids. You bring the HTTP client by implementing
the `Http` trait, on any async runtime; its only dependency is `serde_json`.

```rust
use innertube_music::{Http, InnerTube, Request, Response};

struct MyHttp;

impl Http for MyHttp {
    type Error = String;

    fn send(&self, r: Request) -> impl std::future::Future<Output = Result<Response, String>> + Send {
        async move {
            // POST `r.body` as application/json with `r.headers` when it is set, GET otherwise;
            // return the status and the whole body, error statuses included.
            todo!()
        }
    }
}

async fn play(yt: &InnerTube) -> Result<String, innertube_music::Error<String>> {
    let found = yt.search(&MyHttp, "daft punk one more time").await?;
    let song = found.songs.first().ok_or(innertube_music::Error::Parse("nothing found".into()))?;
    let stream = yt.player(&MyHttp, &song.video_id).await?;
    Ok(stream.url_from(0)) // a ranged URL: plain requests are throttled
}
```

One `InnerTube` value per session: it learns YouTube's visitor id from the first answer and sends it
with every later request (music videos refuse to play without one). Keep `take_new_visitor()` and give
it back with `set_visitor` in the next session: a new visitor is served a different Home.

`InnerTube::radio` returns the song's watch queue as metadata. It does not resolve or fetch audio;
call `player` for a track only when the user asks to play or download it.

Stream URLs are bound to the requesting IP address and expire after `Stream::expires_in`. Ask from byte
0 with `url_from(0)` and send a `Range` header for later bytes.

## Signing in

`InnerTube::set_cookies` signs a session in with a browser session's cookies (`auth::Cookies::parse`
reads a `Cookie` header or a cookies.txt). Signed-in requests to music.youtube.com carry them with a
`SAPISIDHASH` authorization; the stream client never does. Then `home` is the account's own,
and `library_playlists`, `library_albums`, `liked_songs` and `like` work.

Google rotates such a session's cookies, and stops honoring one nobody rotates. To stay signed in:

- call `rotate` before a call when `auth::rotation_due(last_rotated, now)` (every
  `auth::ROTATE_EVERY`, 10 minutes, while in use);
- after every call, store `take_changed()` when it returns new cookies (they come in rotations' and
  answers' `Set-Cookie`, so your `Http` should fill `Response::set_cookies`);
- treat only `Error::SignedOut` as the end of the session. A refused answer is retried once after a
  rotation inside the crate; network errors leave the session as it was.

The cookies should come from a private window that is closed right after: a browser that keeps the
session open keeps rotating it, and its copy and yours undo each other. An app can sign in in a web
view of its own instead: open `auth::SIGN_IN_PAGE`, read the view's cookies for each page it shows
until `auth::signed_in_at(address, cookies)`, take them, and clear the view's cookies and storage.

## Status

Best effort. This is not an official API; YouTube changes either client without notice, and unofficial
clients are against YouTube's terms of service. Age-restricted videos do not play through visionOS.

## Tests

`cargo test`. The fixtures in `tests/fixtures` are trimmed real answers, with tracking, session data,
visitor ids and addresses removed or replaced.

## License

MIT
