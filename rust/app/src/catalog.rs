use crate::api::{CatalogItemData, CatalogSectionData};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CatalogSort {
    #[default]
    NameAscending,
    NameDescending,
    DateAddedNewest,
    DateAddedOldest,
    ReleaseDateNewest,
    ReleaseDateOldest,
    RatingHighest,
}

impl CatalogSort {
    pub const ALL: [Self; 7] = [
        Self::NameAscending,
        Self::NameDescending,
        Self::DateAddedNewest,
        Self::DateAddedOldest,
        Self::ReleaseDateNewest,
        Self::ReleaseDateOldest,
        Self::RatingHighest,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::NameAscending => "name_asc",
            Self::NameDescending => "name_desc",
            Self::DateAddedNewest => "date_added_desc",
            Self::DateAddedOldest => "date_added_asc",
            Self::ReleaseDateNewest => "release_date_desc",
            Self::ReleaseDateOldest => "release_date_asc",
            Self::RatingHighest => "rating_desc",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::NameAscending => "Name A-Z",
            Self::NameDescending => "Name Z-A",
            Self::DateAddedNewest => "Date added newest",
            Self::DateAddedOldest => "Date added oldest",
            Self::ReleaseDateNewest => "Release date newest",
            Self::ReleaseDateOldest => "Release date oldest",
            Self::RatingHighest => "Rating highest",
        }
    }

    pub fn query(self) -> (&'static str, &'static str) {
        match self {
            Self::NameAscending => ("SortName", "Ascending"),
            Self::NameDescending => ("SortName", "Descending"),
            Self::DateAddedNewest => ("DateCreated", "Descending"),
            Self::DateAddedOldest => ("DateCreated", "Ascending"),
            Self::ReleaseDateNewest => ("PremiereDate", "Descending"),
            Self::ReleaseDateOldest => ("PremiereDate", "Ascending"),
            Self::RatingHighest => ("CommunityRating,SortName", "Descending,Ascending"),
        }
    }

    pub fn from_key(value: Option<&str>) -> Self {
        value
            .and_then(|value| Self::ALL.into_iter().find(|sort| sort.key() == value))
            .unwrap_or_default()
    }

    pub fn from_index(index: i32) -> Option<Self> {
        usize::try_from(index)
            .ok()
            .and_then(|index| Self::ALL.get(index).copied())
    }

    pub fn index(self) -> i32 {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CatalogDensity {
    Compact,
    #[default]
    Normal,
    Large,
}

impl CatalogDensity {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Normal, Self::Large];

    pub fn card_width(self) -> i32 {
        match self {
            Self::Compact => 220,
            Self::Normal => 260,
            Self::Large => 320,
        }
    }

    pub fn from_card_width(value: Option<i32>) -> Self {
        let value = value.unwrap_or(Self::Normal.card_width());
        Self::ALL
            .into_iter()
            .min_by_key(|density| (density.card_width() - value).abs())
            .unwrap_or_default()
    }

    pub fn from_index(index: i32) -> Option<Self> {
        usize::try_from(index)
            .ok()
            .and_then(|index| Self::ALL.get(index).copied())
    }

    pub fn index(self) -> i32 {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogSectionRange {
    pub id: String,
    pub title: String,
    pub start_index: usize,
    pub count: usize,
}

#[derive(Clone, Debug)]
pub struct FlattenedCatalog {
    pub items: Vec<CatalogItemData>,
    pub sections: Vec<CatalogSectionRange>,
}

pub fn flatten_sections(sections: &[CatalogSectionData]) -> FlattenedCatalog {
    let mut items = Vec::new();
    let mut ranges = Vec::with_capacity(sections.len());
    for section in sections {
        if section.items.is_empty() {
            continue;
        }
        let start_index = items.len();
        items.extend(section.items.iter().cloned());
        ranges.push(CatalogSectionRange {
            id: section.id.clone(),
            title: section.title.clone(),
            start_index,
            count: section.items.len(),
        });
    }
    FlattenedCatalog {
        items,
        sections: ranges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str) -> CatalogItemData {
        CatalogItemData {
            id: id.into(),
            title: id.into(),
            subtitle: String::new(),
            kind: "Movie".into(),
            image_item_id: id.into(),
            image_tag: None,
            watched: false,
            progress: 0.0,
            is_container: false,
            playable: true,
        }
    }

    #[test]
    fn flattening_preserves_section_ranges_and_duplicate_items() {
        let sections = vec![
            CatalogSectionData {
                id: "one".into(),
                title: "One".into(),
                items: vec![item("a"), item("shared")],
            },
            CatalogSectionData {
                id: "empty".into(),
                title: "Empty".into(),
                items: Vec::new(),
            },
            CatalogSectionData {
                id: "two".into(),
                title: "Two".into(),
                items: vec![item("shared"), item("b")],
            },
        ];

        let flattened = flatten_sections(&sections);
        assert_eq!(
            flattened
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "shared", "shared", "b"]
        );
        assert_eq!(
            flattened.sections,
            [
                CatalogSectionRange {
                    id: "one".into(),
                    title: "One".into(),
                    start_index: 0,
                    count: 2,
                },
                CatalogSectionRange {
                    id: "two".into(),
                    title: "Two".into(),
                    start_index: 2,
                    count: 2,
                },
            ]
        );
    }

    #[test]
    fn catalog_preferences_use_kotlin_compatible_values() {
        assert_eq!(
            CatalogDensity::from_card_width(Some(220)),
            CatalogDensity::Compact
        );
        assert_eq!(
            CatalogDensity::from_card_width(Some(360)),
            CatalogDensity::Large
        );
        assert_eq!(CatalogDensity::default().card_width(), 260);
        assert_eq!(
            CatalogSort::from_key(Some("rating_desc")),
            CatalogSort::RatingHighest
        );
        assert_eq!(
            CatalogSort::from_key(Some("runtime_asc")),
            CatalogSort::NameAscending
        );
        assert_eq!(
            CatalogSort::RatingHighest.query(),
            ("CommunityRating,SortName", "Descending,Ascending")
        );
    }
}
