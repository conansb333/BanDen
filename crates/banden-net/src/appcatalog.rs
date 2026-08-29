//! Application catalog: well-known apps with their network fingerprints.
//!
//! Each app is identified by the domain suffixes its endpoints use. The
//! forwarder matches DNS names and TLS SNI against these suffixes to
//! classify the target's flows per application.
//!
//! The catalog is embedded from `apps.json` (edit the JSON to add apps or
//! update endpoints without touching the engine).

use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize)]
pub struct AppDefinition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub emoji: String,
    /// Domain suffixes owned by the app: a DNS name or SNI matches when it
    /// equals the entry or ends with "." + entry.
    #[serde(default)]
    pub domains: Vec<String>,
    /// IP ranges announced by the app's network (covers traffic to cached
    /// endpoints that bypass DNS). "a.b.c.d/len" strings.
    #[serde(default)]
    pub cidrs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CatalogFile {
    apps: Vec<AppDefinition>,
}

pub fn catalog() -> &'static [AppDefinition] {
    static CATALOG: std::sync::OnceLock<Arc<[AppDefinition]>> = std::sync::OnceLock::new();
    CATALOG.get_or_init(|| {
        let raw = include_str!("../apps.json");
        match serde_json::from_str::<CatalogFile>(raw) {
            Ok(f) => f.apps.into(),
            Err(e) => {
                tracing::error!(error = %e, "apps.json is invalid - catalog disabled");
                Arc::from([])
            }
        }
    })
}

/// Look up a catalog definition by app id.
pub fn by_id(id: &str) -> Option<&'static AppDefinition> {
    catalog().iter().find(|a| a.id == id)
}

/// Does `name` (a DNS name or TLS SNI) belong to `app`?
pub fn name_matches(app: &AppDefinition, name: &str) -> bool {
    let n = name.trim_end_matches('.').to_ascii_lowercase();
    app.domains
        .iter()
        .any(|d| n == d.as_str() || n.ends_with(&format!(".{d}")))
}

/// Which catalog app does `name` belong to, if any?
pub fn classify_name(name: &str) -> Option<&'static AppDefinition> {
    catalog().iter().find(|a| name_matches(a, name))
}

/// Compile a list of app IDs into resolved definitions (unknown ids ignored).
pub fn resolve_ids(ids: &[String]) -> Vec<&'static AppDefinition> {
    ids.iter().filter_map(|id| by_id(id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_loads_and_has_popular_apps() {
        let c = catalog();
        assert!(c.len() >= 10);
        if let Some(app) = by_id("whatsapp") {
            assert_eq!(app.name, "WhatsApp");
        }
        assert!(by_id("youtube").is_some());
        assert!(by_id("nonexistent-app").is_none());
    }

    #[test]
    fn name_matching() {
        let wa = by_id("whatsapp").unwrap();
        assert!(name_matches(wa, "wa.me"));
        assert!(name_matches(wa, "media-fb.om.whatsapp.net"));
        assert!(name_matches(wa, "WHATSAPP.COM"));
        assert!(!name_matches(wa, "notwhatsapp.com")); // suffix must be dotted
        assert!(!name_matches(wa, "example.com"));

        let yt = by_id("youtube").unwrap();
        assert!(name_matches(yt, "rr3---sn-1.googlevideo.com"));
        assert!(name_matches(yt, "youtubei.googleapis.com"));
        assert!(!name_matches(yt, "maps.googleapis.com"));
    }

    #[test]
    fn resolve_ids_ignores_unknown() {
        let resolved = resolve_ids(&["whatsapp".into(), "junk".into()]);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].id, "whatsapp");
    }
}
