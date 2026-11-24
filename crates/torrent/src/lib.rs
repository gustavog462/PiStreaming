//! Torrent engine (librqbit).
pub mod engine;

pub use engine::{add_magnet, open_session, pick_largest_video, torrent_stream, AddedTorrent};
