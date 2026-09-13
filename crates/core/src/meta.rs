use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaItem {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub poster: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaDetail {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "releaseInfo")]
    pub release_info: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub runtime: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_meta_item_and_detail() {
        let item: MetaItem = serde_json::from_str(
            r#"{"id":"tt1160419","type":"movie","name":"Dune","poster":"p.jpg"}"#,
        )
        .unwrap();
        assert_eq!(item.kind, "movie");
        let detail: MetaDetail = serde_json::from_str(
            r#"{"id":"tt1160419","type":"movie","name":"Dune","genres":["Sci-Fi"],"releaseInfo":"2021"}"#,
        )
        .unwrap();
        assert_eq!(detail.genres, vec!["Sci-Fi".to_string()]);
        assert_eq!(detail.release_info.as_deref(), Some("2021"));
    }
}
