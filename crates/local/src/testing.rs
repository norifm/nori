//! Music files for tests: FLAC files with tags and pictures.

/// A FLAC file of a second of `hz` with `comments` (Vorbis comments, "KEY=value") and, with
/// `picture`, an embedded front cover holding those bytes.
pub fn flac(hz: f64, comments: &[&str], picture: Option<&[u8]>) -> Vec<u8> {
    let plain = nori_player::files::flac(8000, &nori_player::files::sine(8000, hz, 1.0, 8000.0), None);
    // fLaC, then STREAMINFO (4-byte header, 34 bytes) marked last; the blocks go after it.
    let (head, frames) = plain.split_at(4 + 4 + 34);
    let mut out = head.to_vec();
    out[4] &= 0x7f;
    let mut vorbis = Vec::new();
    vorbis.extend(4u32.to_le_bytes());
    vorbis.extend(b"nori");
    vorbis.extend((comments.len() as u32).to_le_bytes());
    for c in comments {
        vorbis.extend((c.len() as u32).to_le_bytes());
        vorbis.extend(c.as_bytes());
    }
    let mut blocks = vec![(4u8, vorbis)];
    if let Some(bytes) = picture {
        let mut p = Vec::new();
        p.extend(3u32.to_be_bytes());
        p.extend((b"image/png".len() as u32).to_be_bytes());
        p.extend(b"image/png");
        p.extend(0u32.to_be_bytes());
        p.extend([0u8; 16]);
        p.extend((bytes.len() as u32).to_be_bytes());
        p.extend(bytes);
        blocks.push((6, p));
    }
    let last = blocks.len() - 1;
    for (k, (kind, body)) in blocks.into_iter().enumerate() {
        out.push(kind | if k == last { 0x80 } else { 0 });
        out.extend(&(body.len() as u32).to_be_bytes()[1..]);
        out.extend(body);
    }
    out.extend(frames);
    out
}
