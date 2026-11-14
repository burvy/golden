//! Turn raw kit records into the contract's `Building`. Pure: no files, no network.

use std::collections::HashMap;

use contracts::building::FEET_TO_M;
use contracts::{Building, Contact, ContactRole, Matterport};

use crate::raw::{Company, ListingContact, ListingPropertyContact, Property};

/// At most this many contacts per building, so a JSON list of 100 buildings stays small (phone budget).
pub const MAX_CONTACTS: usize = 8;

/// Everything outside the property record that a building's contacts and tours come from.
pub struct Lookups<'a> {
    /// `company-detail` by id: company name and office phone.
    pub companies: &'a HashMap<u64, Company>,
    /// The person behind a property contact (from listings' `propertyContacts`), by property-contact id.
    pub people: &'a HashMap<u64, ListingPropertyContact>,
    /// Brokers and agents on the property's active listings, by property id.
    pub brokers: &'a HashMap<u64, Vec<ListingContact>>,
    /// Matterport model ids, by property id.
    pub matterport: &'a HashMap<u64, Vec<String>>,
}

/// One kit property as a contract `Building`. `None` if it has no coordinates.
/// `dist_m` and `roof_m` are left `None`: the caller sets the distance, the DSM gives the roof.
pub fn building(p: &Property, look: &Lookups) -> Option<Building> {
    let lat = p.primary_address.latitude?;
    let lon = p.primary_address.longitude?;
    Some(Building {
        id: format!("costar:{}", p.id),
        name: clean(p.building_name.as_deref()),
        address: address(p),
        lat,
        lon,
        dist_m: None,
        height_m: p.building_height_in_feet.map(feet_to_m),
        floors: p.num_stories_above_grade,
        roof_m: None,
        property_type: property_type(p.property_type).map(str::to_string),
        contacts: contacts(p, look),
        matterport: look
            .matterport
            .get(&p.id)
            .map(|ids| ids.iter().map(|id| matterport(id)).collect())
            .unwrap_or_default(),
    })
}

/// CoStar heights are in feet; the contract wants metres (to 0.1 m).
pub fn feet_to_m(feet: f64) -> f64 {
    (feet * FEET_TO_M * 10.0).round() / 10.0
}

pub fn matterport(id: &str) -> Matterport {
    Matterport { id: id.to_string(), url: format!("https://my.matterport.com/show/?m={id}") }
}

/// Who to call, best first: the building's manager, then leasing, then owners, then listing brokers.
///
/// 1. The building's own contacts (`commercial-property` `contacts[]`). These name a company, and
///    sometimes a person (when a listing copies the same contact with a name and email).
/// 2. Brokers on the building's active listings.
fn contacts(p: &Property, look: &Lookups) -> Vec<Contact> {
    let from_building = p.contacts.iter().filter_map(|c| {
        let role = building_role(c.contact_role)?;
        let person = look.people.get(&c.id);
        let company = c.location_id.and_then(|id| look.companies.get(&id));
        Some(Contact {
            role,
            name: person.and_then(|x| full_name(x.first_name.as_deref(), x.last_name.as_deref())),
            company: person
                .and_then(|x| clean(x.location_name.as_deref()))
                .or_else(|| company.and_then(|x| clean(x.name.as_deref()))),
            phone: clean(c.contact_phone_number.as_deref())
                .or_else(|| person.and_then(|x| clean(x.phone_number.as_deref())))
                .or_else(|| company.and_then(|x| clean(x.location_phone.as_deref()))),
            email: person.and_then(|x| clean(x.email_address.as_deref())),
        })
    });

    let from_listings = look.brokers.get(&p.id).into_iter().flatten().filter_map(|c| {
        if c.display_in_product == Some(false) {
            return None;
        }
        Some(Contact {
            role: listing_role(c.contact_role),
            name: full_name(c.first_name.as_deref(), c.last_name.as_deref()),
            company: clean(c.location_name.as_deref()),
            phone: clean(c.phone_number.as_deref()),
            email: clean(c.email_address.as_deref()),
        })
    });

    let mut out: Vec<Contact> = Vec::new();
    for c in from_building.chain(from_listings) {
        let reachable = c.name.is_some() || c.company.is_some() || c.phone.is_some() || c.email.is_some();
        if reachable && !out.contains(&c) {
            out.push(c);
        }
    }
    out.sort_by_key(|c| role_rank(c.role)); // stable: building contacts stay before brokers
    out.truncate(MAX_CONTACTS);
    out
}

/// `ContactRole` codes (kit `schemas/enums.json`) on the building record that can grant access.
/// Developers, architects, lenders and so on are skipped.
fn building_role(code: u32) -> Option<ContactRole> {
    match code {
        4 | 29 | 33 => Some(ContactRole::Manager), // PropertyManager, CenterManager, BuildingSuperintendent
        6 | 20 => Some(ContactRole::Leasing),      // PrimaryLeasingCompany, LeasingCompany
        3 | 7 => Some(ContactRole::Owner),         // RecordedOwner, TrueOwner
        _ => None,
    }
}

/// `ContactRole` codes on a listing's brokers.
fn listing_role(code: u32) -> ContactRole {
    match code {
        11 | 46 | 47 => ContactRole::Leasing, // ListingBroker, ListingAgent, LeasingAgent
        52 => ContactRole::Owner,             // OwnerListingContact
        _ => ContactRole::Other,
    }
}

fn role_rank(r: ContactRole) -> u8 {
    match r {
        ContactRole::Manager => 0,
        ContactRole::Leasing => 1,
        ContactRole::Owner => 2,
        ContactRole::Other => 3,
    }
}

/// `PropertyType` codes (kit `schemas/enums.json`) as readable names.
fn property_type(code: Option<u32>) -> Option<&'static str> {
    Some(match code? {
        1 => "Hospitality",
        2 => "Industrial",
        3 => "Land",
        5 => "Office",
        6 => "Retail",
        7 => "Flex",
        8 => "Sports & Entertainment",
        9 => "Mixed Use",
        10 => "Specialty",
        11 => "Multifamily",
        13 => "Healthcare",
        15 => "Residential",
        16 => "Homes",
        _ => return None,
    })
}

/// "3000 Robertson Blvd, Los Angeles, CA 90034". ZIP+4 is cut to the 5-digit ZIP.
fn address(p: &Property) -> Option<String> {
    let a = &p.primary_address;
    let street = join(&[a.street_number.as_deref(), a.street_name.as_deref(), a.street_type.as_deref()], " ");
    let zip = clean(a.postcode.as_deref()).map(|z| z.chars().take(5).collect::<String>());
    let state_zip = join(&[a.state.as_deref(), zip.as_deref()], " ");
    street.as_ref()?;
    join(&[street.as_deref(), a.city.as_deref(), state_zip.as_deref()], ", ")
}

fn full_name(first: Option<&str>, last: Option<&str>) -> Option<String> {
    join(&[first, last], " ")
}

/// Non-empty trimmed parts joined by `sep`; `None` if there are none.
fn join(parts: &[Option<&str>], sep: &str) -> Option<String> {
    let kept: Vec<&str> = parts.iter().filter_map(|p| p.map(str::trim)).filter(|p| !p.is_empty()).collect();
    (!kept.is_empty()).then(|| kept.join(sep))
}

fn clean(s: Option<&str>) -> Option<String> {
    s.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feet_become_metres() {
        assert_eq!(feet_to_m(150.0), 45.7); // 45.72
        assert_eq!(feet_to_m(0.0), 0.0);
        assert_eq!(feet_to_m(420.0), 128.0); // 128.016
    }

    #[test]
    fn join_skips_blanks() {
        assert_eq!(join(&[Some(" 12 "), None, Some(""), Some("Main")], " "), Some("12 Main".into()));
        assert_eq!(join(&[None, Some("  ")], " "), None);
    }

    #[test]
    fn only_access_roles_are_kept_from_the_building() {
        assert_eq!(building_role(4), Some(ContactRole::Manager));
        assert_eq!(building_role(6), Some(ContactRole::Leasing));
        assert_eq!(building_role(7), Some(ContactRole::Owner));
        assert_eq!(building_role(2), None); // Architect
        assert_eq!(building_role(1), None); // Developer
    }
}
