use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogRequest {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

impl CatalogRequest {
    pub fn search(kind: &str, id: &str, query: &str) -> Self {
        let mut extra = HashMap::new();
        extra.insert("search".to_string(), query.to_string());
        Self { kind: kind.to_string(), id: id.to_string(), extra }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_request_carries_query() {
        let req = CatalogRequest::search("movie", "top", "dune");
        assert_eq!(req.kind, "movie");
        assert_eq!(req.extra.get("search").unwrap(), "dune");
    }
}
