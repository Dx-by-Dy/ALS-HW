//! Партнёры, которым можно передать тикет, и категории, которыми каждый занимается.

use crate::domain::partner::PartnerId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partner {
    pub partner_id: PartnerId,
    pub name: String,
    /// Категории тикетов, которые партнёр берёт в работу.
    pub categories: Vec<String>,
}

impl Partner {
    pub fn new(partner_id: PartnerId, name: &str, categories: &[&str]) -> Self {
        Self {
            partner_id,
            name: name.to_owned(),
            categories: categories.iter().map(|&c| c.to_owned()).collect(),
        }
    }

    /// Берёт ли партнёр тикеты этой категории. Регистр и пробелы по краям не важны.
    pub fn serves(&self, category: &str) -> bool {
        let category = category.trim();
        self.categories
            .iter()
            .any(|c| c.eq_ignore_ascii_case(category))
    }
}

/// Справочник партнёров.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartnerRoster {
    partners: Vec<Partner>,
}

impl PartnerRoster {
    pub fn new(partners: Vec<Partner>) -> Self {
        Self { partners }
    }

    /// Первый партнёр, который берёт тикеты этой категории.
    pub fn for_category(&self, category: &str) -> Option<&Partner> {
        self.partners.iter().find(|p| p.serves(category))
    }

    pub fn get(&self, partner_id: PartnerId) -> Option<&Partner> {
        self.partners.iter().find(|p| p.partner_id == partner_id)
    }

    pub fn all(&self) -> &[Partner] {
        &self.partners
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roster() -> PartnerRoster {
        PartnerRoster::new(vec![
            Partner::new(PartnerId(1), "FixIt", &["Hardware"]),
            Partner::new(PartnerId(2), "NetCo", &["Network", "Hardware"]),
        ])
    }

    #[test]
    fn partner_is_chosen_by_category_ignoring_case() {
        let roster = roster();
        assert_eq!(
            roster.for_category(" hardware ").map(|p| p.partner_id),
            Some(PartnerId(1))
        );
        assert_eq!(
            roster.for_category("NETWORK").map(|p| p.partner_id),
            Some(PartnerId(2))
        );
        assert_eq!(roster.for_category("Billing"), None);
    }

    #[test]
    fn partner_is_found_by_id() {
        assert_eq!(roster().get(PartnerId(2)).unwrap().name, "NetCo");
        assert_eq!(roster().get(PartnerId(3)), None);
    }
}
