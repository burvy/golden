//! The few fields we read from the CoStar kit, exactly as the kit spells them.
//! Everything else in the records is ignored. Field docs: `models/*.md` in the kit.

use serde::Deserialize;

/// One line of `data/commercial-property.jsonl`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Property {
    pub id: u64,
    pub primary_address: Address,
    pub building_name: Option<String>,
    pub property_type: Option<u32>,
    pub building_height_in_feet: Option<f64>,
    pub num_stories_above_grade: Option<u32>,
    /// Current contacts of the building (owner, manager, leasing company...). No person names here.
    #[serde(default)]
    pub contacts: Vec<PropertyContact>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub street_number: Option<String>,
    pub street_name: Option<String>,
    pub street_type: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postcode: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyContact {
    /// Same id as `propertyContacts[].id` on a listing, which adds the person's name and email.
    pub id: u64,
    pub contact_role: u32,
    /// The company (office location); joins to `company-detail.jsonl` `id`.
    pub location_id: Option<u64>,
    pub contact_phone_number: Option<String>,
}

/// One line of `data/commercial-listing-summary.jsonl`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub property_id: Option<u64>,
    pub listing_status: Option<u32>,
    #[serde(default)]
    pub listing_contacts: Vec<ListingContact>,
    #[serde(default)]
    pub property_contacts: Vec<ListingPropertyContact>,
}

/// A broker or agent on a listing.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListingContact {
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub location_name: Option<String>,
    pub email_address: Option<String>,
    pub phone_number: Option<String>,
    pub contact_role: u32,
    pub display_in_product: Option<bool>,
}

/// The building's own contact, as copied onto a listing, with the person filled in.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListingPropertyContact {
    pub id: u64,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub location_name: Option<String>,
    pub email_address: Option<String>,
    pub phone_number: Option<String>,
}

/// One line of `data/company-detail.jsonl` (a company office).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Company {
    pub id: u64,
    pub name: Option<String>,
    pub location_phone: Option<String>,
}

/// `ListingStatus` 1 = Active (enums.json). Off-market listings have stale brokers.
pub const LISTING_ACTIVE: u32 = 1;
