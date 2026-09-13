use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

static SEEDS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"👤\s*(\d+)").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Quality {
    Unknown = 0,
    K480 = 1,
    K720 = 2,
    K1080 = 3,
    K4 = 4,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stream {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, rename = "infoHash")]
    pub info_hash: Option<String>,
    #[serde(default, rename = "fileIdx")]
    pub file_idx: Option<u32>,
    #[serde(default)]
    pub sources: Vec<String>,
    /// Addon que lo aportó. Se rellena al federar (no viene del addon).
    #[serde(default)]
    pub source_addon: Option<String>,
}

impl Stream {
    pub fn text(&self) -> String {
        format!(
            "{} {}",
            self.name.clone().unwrap_or_default(),
            self.title.clone().unwrap_or_default()
        )
    }

    pub fn quality(&self) -> Quality {
        let hay = self.text().to_lowercase();
        if hay.contains("2160p") || hay.contains("4k") {
            Quality::K4
        } else if hay.contains("1080p") {
            Quality::K1080
        } else if hay.contains("720p") {
            Quality::K720
        } else if hay.contains("480p") {
            Quality::K480
        } else {
            Quality::Unknown
        }
    }

    pub fn seeds(&self) -> Option<u32> {
        SEEDS_RE
            .captures(&self.text())
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse().ok())
    }

    pub fn is_playable(&self) -> bool {
        (self.info_hash.is_some() || self.url.is_some())
            && self.seeds().map(|s| s > 0).unwrap_or(self.url.is_some())
    }
}

/// Ordena: calidad desc, luego seeds desc, luego los reproducibles primero.
pub fn rank_streams(streams: &mut [Stream]) {
    streams.sort_by(|a, b| {
        b.quality()
            .cmp(&a.quality())
            .then_with(|| b.seeds().unwrap_or(0).cmp(&a.seeds().unwrap_or(0)))
            .then_with(|| b.is_playable().cmp(&a.is_playable()))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: &str) -> Stream {
        Stream {
            name: Some(name.into()),
            title: None,
            url: None,
            info_hash: Some("abc".into()),
            file_idx: None,
            sources: vec![],
            source_addon: None,
        }
    }

    #[test]
    fn infers_quality() {
        assert_eq!(s("Movie 2160p HDR").quality(), Quality::K4);
        assert_eq!(s("Movie 1080p").quality(), Quality::K1080);
        assert_eq!(s("Movie 720p").quality(), Quality::K720);
        assert_eq!(s("Movie").quality(), Quality::Unknown);
    }

    #[test]
    fn parses_seeds_emoji() {
        let mut st = s("Torrent 🧲");
        st.title = Some("👤 421 💾 12 GB".into());
        assert_eq!(st.seeds(), Some(421));
    }

    #[test]
    fn ranks_4k_above_1080p() {
        let mut v = vec![s("Movie 1080p"), s("Movie 2160p"), s("Movie 720p")];
        rank_streams(&mut v);
        assert_eq!(v[0].quality(), Quality::K4);
        assert_eq!(v[1].quality(), Quality::K1080);
    }
}
