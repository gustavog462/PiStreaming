use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub resources: Vec<Resource>,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub catalogs: Vec<CatalogEntry>,
    #[serde(default, rename = "idPrefixes")]
    pub id_prefixes: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Resource {
    Simple(String),
    Full {
        name: String,
        #[serde(default)]
        types: Vec<String>,
        #[serde(default, rename = "idPrefixes")]
        id_prefixes: Option<Vec<String>>,
    },
}

impl Resource {
    pub fn name(&self) -> &str {
        match self {
            Resource::Simple(n) => n,
            Resource::Full { name, .. } => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogEntry {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub extra: Vec<ExtraProp>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtraProp {
    pub name: String,
    #[serde(default, rename = "isRequired")]
    pub is_required: bool,
    #[serde(default)]
    pub options: Vec<String>,
}

impl Manifest {
    /// ¿El addon declara el recurso dado? (p.ej. "stream")
    pub fn supports(&self, resource: &str) -> bool {
        self.resources.iter().any(|r| r.name() == resource)
    }

    /// Catálogos que sirven para búsqueda por texto.
    pub fn search_catalogs(&self) -> Vec<&CatalogEntry> {
        self.catalogs
            .iter()
            .filter(|c| c.extra.iter().any(|e| e.name == "search"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_string_resources_and_catalogs() {
        let raw = r#"{
            "id": "org.torrentio",
            "version": "0.0.1",
            "name": "Torrentio",
            "resources": ["catalog", "meta", "stream"],
            "types": ["movie", "series"],
            "catalogs": [{"type": "movie", "id": "top", "name": "Top"}]
        }"#;
        let m: Manifest = serde_json::from_str(raw).unwrap();
        assert_eq!(m.id, "org.torrentio");
        assert_eq!(m.resources.len(), 3);
        assert_eq!(m.resources[0], Resource::Simple("catalog".into()));
        assert_eq!(m.catalogs[0].kind, "movie");
        assert_eq!(m.catalogs[0].name.as_deref(), Some("Top"));
    }

    #[test]
    fn parses_full_resource_and_id_prefixes() {
        let raw = r#"{
            "id": "org.cinemeta", "version": "3.0.0", "name": "Cinemeta",
            "resources": [{"name": "catalog", "types": ["movie"], "idPrefixes": ["tt"]}],
            "types": ["movie"],
            "idPrefixes": ["tt"],
            "catalogs": [{"type": "movie", "id": "top", "extra": [{"name": "search", "isRequired": true}]}]
        }"#;
        let m: Manifest = serde_json::from_str(raw).unwrap();
        match &m.resources[0] {
            Resource::Full { name, types, id_prefixes } => {
                assert_eq!(name, "catalog");
                assert_eq!(types, &vec!["movie".to_string()]);
                assert_eq!(id_prefixes.as_deref(), Some(&["tt".to_string()][..]));
            }
            _ => panic!("esperaba Resource::Full"),
        }
        assert!(m.catalogs[0].extra[0].is_required);
    }
}
