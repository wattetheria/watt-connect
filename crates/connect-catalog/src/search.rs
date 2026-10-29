//! In-memory search over the catalog by name, title, description and category.

use crate::schema::{Catalog, ServerEntry, ServiceStatus};

/// Return matching entries, best first. An empty query matches everything.
/// Revoked services are never returned.
#[must_use]
pub fn search<'a>(
    catalog: &'a Catalog,
    query: &str,
    category: Option<&str>,
) -> Vec<&'a ServerEntry> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let category = category.map(str::to_lowercase);

    let mut scored: Vec<(u32, &ServerEntry)> = catalog
        .services
        .iter()
        .filter(|entry| entry.connect().status != ServiceStatus::Revoked)
        .filter(|entry| {
            category.as_deref().is_none_or(|wanted| {
                entry
                    .connect()
                    .category
                    .iter()
                    .any(|have| have.to_lowercase() == wanted)
            })
        })
        .filter_map(|entry| {
            if terms.is_empty() {
                return Some((0, entry));
            }
            let mut total = 0;
            for term in &terms {
                let score = score_term(entry, term);
                if score == 0 {
                    return None;
                }
                total += score;
            }
            Some((total, entry))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored.into_iter().map(|(_, entry)| entry).collect()
}

fn score_term(entry: &ServerEntry, term: &str) -> u32 {
    let mut score = 0;
    if entry.name.as_str().to_lowercase().contains(term) {
        score += 5;
    }
    if entry
        .title
        .as_deref()
        .is_some_and(|title| title.to_lowercase().contains(term))
    {
        score += 5;
    }
    if entry
        .connect()
        .category
        .iter()
        .any(|category| category.to_lowercase() == term)
    {
        score += 4;
    }
    if entry.description.to_lowercase().contains(term) {
        score += 2;
    }
    if entry
        .connect()
        .tools
        .iter()
        .any(|tool| tool.name.to_lowercase().contains(term))
    {
        score += 1;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundled::bundled_catalog;

    #[test]
    fn finds_by_title_category_and_name() {
        let catalog = bundled_catalog();
        let names = |query: &str, category: Option<&str>| -> Vec<String> {
            search(&catalog, query, category)
                .into_iter()
                .map(|entry| entry.name.to_string())
                .collect()
        };
        assert_eq!(names("瑞幸", None), vec!["cn.lkcoffee/order"]);
        assert_eq!(names("", Some("coffee")), vec!["cn.lkcoffee/order"]);
        assert!(names("taxi", None).contains(&"cn.didichuxing/ride".to_owned()));
        assert!(names("mcd", None).contains(&"cn.mcd/order".to_owned()));
        assert!(names("no-such-thing", None).is_empty());
        assert_eq!(names("", None).len(), catalog.services.len());
    }
}
