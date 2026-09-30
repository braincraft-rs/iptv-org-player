//! Domain types shared by sync, storage and the UI.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Country {
    pub code: String,
    pub name: String,
    pub flag: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub country_code: String,
    pub categories: Vec<String>,
    pub is_nsfw: bool,
    pub logo_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    pub channel_id: String,
    pub url: String,
    pub quality: Option<String>,
    pub referrer: Option<String>,
    pub user_agent: Option<String>,
}

/// Everything a sync produces, ready to be written to the database.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    pub countries: Vec<Country>,
    pub channels: Vec<Channel>,
    pub streams: Vec<Stream>,
}
