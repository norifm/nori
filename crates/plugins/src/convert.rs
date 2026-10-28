//! nori's records as the plugin API's (`wit/types.wit`), and back.

use nori_model::{Album, Artist, ArtistRef, ExplicitStatus, Playlist, ReplayGain, SearchResult, Song};

use crate::runtime::types as t;

fn explicit_in(e: t::Explicit) -> ExplicitStatus {
    match e {
        t::Explicit::Unknown => ExplicitStatus::Unmarked,
        t::Explicit::Clean => ExplicitStatus::Clean,
        t::Explicit::Explicit => ExplicitStatus::Explicit,
    }
}

fn explicit_out(e: ExplicitStatus) -> t::Explicit {
    match e {
        ExplicitStatus::Unmarked => t::Explicit::Unknown,
        ExplicitStatus::Clean => t::Explicit::Clean,
        ExplicitStatus::Explicit => t::Explicit::Explicit,
    }
}

pub fn song_in(s: t::Song) -> Song {
    Song {
        id: s.id,
        title: s.title,
        artist: s.artist,
        artist_id: s.artist_id,
        artists: s.artists.into_iter().map(|a| ArtistRef { id: a.id.unwrap_or_default(), name: a.name }).collect(),
        album: s.album,
        album_id: s.album_id,
        cover_art: s.cover,
        duration: s.duration,
        track: s.track,
        disc_number: s.disc,
        year: s.year,
        genre: s.genre,
        starred: s.starred,
        is_external: s.external,
        explicit_status: explicit_in(s.explicit),
        replay_gain: s.replay_gain.map(|g| ReplayGain { track_gain: g.track_db, album_gain: g.album_db, track_peak: g.track_peak, album_peak: g.album_peak, ..Default::default() }),
        bpm: s.bpm,
        user_rating: s.rating,
        ..Default::default()
    }
    .dressed()
}

pub fn song_out(s: &Song) -> t::Song {
    t::Song {
        id: s.id.clone(),
        title: s.title.clone(),
        artist: s.artist.clone(),
        artist_id: s.artist_id.clone(),
        artists: s.artists.iter().map(|a| t::ArtistRef { id: (!a.id.is_empty()).then(|| a.id.clone()), name: a.name.clone() }).collect(),
        album: s.album.clone(),
        album_id: s.album_id.clone(),
        cover: s.cover_art.clone(),
        duration: s.duration,
        track: s.track,
        disc: s.disc_number,
        year: s.year,
        genre: s.genre.clone(),
        starred: s.starred,
        external: s.is_external,
        explicit: explicit_out(s.explicit_status),
        replay_gain: s.replay_gain.as_ref().map(|g| t::ReplayGain { track_db: g.track_gain, album_db: g.album_gain, track_peak: g.track_peak, album_peak: g.album_peak }),
        bpm: s.bpm,
        rating: s.user_rating,
    }
}

pub fn album_in(a: t::Album) -> Album {
    Album {
        id: a.id,
        subtitle: nori_model::lines::album_subtitle(&a.artist, a.year),
        name: a.name,
        artist: a.artist,
        artist_id: a.artist_id,
        cover_art: a.cover,
        song_count: a.song_count,
        duration: a.duration,
        year: a.year,
        genre: a.genre,
        starred: a.starred,
        is_external: a.external,
        ..Default::default()
    }
}

pub fn album_out(a: &Album) -> t::Album {
    t::Album {
        id: a.id.clone(),
        name: a.name.clone(),
        artist: a.artist.clone(),
        artist_id: a.artist_id.clone(),
        cover: a.cover_art.clone(),
        song_count: a.song_count,
        duration: a.duration,
        year: a.year,
        genre: a.genre.clone(),
        starred: a.starred,
        external: a.is_external,
    }
}

pub fn artist_in(a: t::Artist) -> Artist {
    Artist { id: a.id, name: a.name, cover_art: a.cover, artist_image_url: a.image_url, album_count: a.album_count, starred: a.starred, is_external: a.external }
}

pub fn artist_out(a: &Artist) -> t::Artist {
    t::Artist { id: a.id.clone(), name: a.name.clone(), cover: a.cover_art.clone(), image_url: a.artist_image_url.clone(), album_count: a.album_count, starred: a.starred, external: a.is_external }
}

pub fn playlist_in(p: t::Playlist) -> Playlist {
    Playlist { id: p.id, name: p.name, comment: p.comment, owner: p.owner, public: true, song_count: p.song_count, duration: p.duration, cover_art: p.cover }
}

pub fn playlist_out(p: &Playlist) -> t::Playlist {
    t::Playlist { id: p.id.clone(), name: p.name.clone(), comment: p.comment.clone(), owner: p.owner.clone(), song_count: p.song_count, duration: p.duration, cover: p.cover_art.clone() }
}

pub fn found_in(f: t::Found) -> SearchResult {
    SearchResult {
        songs: f.songs.into_iter().map(song_in).collect(),
        albums: f.albums.into_iter().map(album_in).collect(),
        artists: f.artists.into_iter().map(artist_in).collect(),
        playlists: f.playlists.into_iter().map(playlist_in).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_song_keeps_through_the_plugin_api() {
        let s = Song {
            id: "yt-1".into(),
            title: "T".into(),
            artist: "A".into(),
            artists: vec![ArtistRef { id: "ar".into(), name: "A".into() }, ArtistRef { id: String::new(), name: "B".into() }],
            album_id: Some("al".into()),
            cover_art: Some("https://x/c={size}".into()),
            duration: 200,
            explicit_status: ExplicitStatus::Explicit,
            replay_gain: Some(ReplayGain { track_gain: Some(-3.0), ..Default::default() }),
            starred: true,
            ..Default::default()
        }
        .dressed();
        assert_eq!(song_in(song_out(&s)), s);
        let a = Album { id: "al".into(), name: "N".into(), artist: "A".into(), year: 2001, ..Default::default() };
        assert_eq!(album_in(album_out(&a)), Album { subtitle: nori_model::lines::album_subtitle("A", 2001), ..a });
    }
}
