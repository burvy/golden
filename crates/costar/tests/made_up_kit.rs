//! Tests on a tiny MADE-UP kit written to a temp folder. Every id, name, phone and email below is
//! invented, shaped like the real kit's records. Never paste real CoStar rows here.

use std::fs;
use std::path::Path;

use costar::{ContactRole, Error, Kit};

/// Origin of the made-up kit: a point in downtown LA.
const LAT: f64 = 34.0490;
const LON: f64 = -118.2510;

/// Three buildings, ~0 m, ~111 m and ~1.1 km north of the origin, plus one with no coordinates.
fn write_kit(dir: &Path) {
    fs::create_dir_all(dir.join("data")).unwrap();

    let properties = [
        // Tower at the origin: height in feet, manager + owner (twice) + architect, one listing.
        r#"{"id":101,"primaryAddress":{"latitude":34.0490,"longitude":-118.2510,"streetNumber":"100","streetName":"Example","streetType":"St","city":"Los Angeles","state":"CA","postcode":"900131234"},
            "buildingName":"Made Up Tower","propertyType":5,"buildingHeightInFeet":150.0,"numStoriesAboveGrade":12,"isCondo":false,
            "contacts":[
              {"id":9001,"contactRole":2,"locationId":501,"contactPhoneNumber":"(000) 555-0001"},
              {"id":9002,"contactRole":7,"locationId":502,"contactPhoneNumber":null},
              {"id":9003,"contactRole":3,"locationId":502,"contactPhoneNumber":null},
              {"id":9004,"contactRole":4,"locationId":503,"contactPhoneNumber":"(000) 555-0004"}]}"#,
        // ~111 m north: no height, no contacts, no name.
        r#"{"id":102,"primaryAddress":{"latitude":34.0500,"longitude":-118.2510,"streetNumber":"200","streetName":"Sample","streetType":"Ave","city":"Los Angeles","state":"CA","postcode":"90013"},
            "propertyType":11,"numStoriesAboveGrade":4,"contacts":[]}"#,
        // ~1.1 km north.
        r#"{"id":103,"primaryAddress":{"latitude":34.0590,"longitude":-118.2510,"streetNumber":"300","streetName":"Far","streetType":"Rd","city":"Los Angeles","state":"CA","postcode":"90012"},
            "propertyType":6,"contacts":[]}"#,
        // No coordinates: skipped.
        r#"{"id":104,"primaryAddress":{"latitude":null,"longitude":null,"city":"Los Angeles","state":"CA"},"contacts":[]}"#,
    ];
    fs::write(dir.join("data/commercial-property.jsonl"), properties.map(one_line).join("\n")).unwrap();

    let listings = [
        // Active listing on 101: a broker, plus the person behind property contact 9004 (the manager).
        r#"{"id":7001,"listingStatus":1,"propertyId":101,
            "listingContacts":[{"firstName":"Pat","lastName":"Broker","locationName":"Pretend Realty","emailAddress":"pat@example.com","phoneNumber":"(000) 555-0100","contactRole":11,"displayInProduct":true},
                               {"firstName":"Hidden","lastName":"Person","locationName":"Pretend Realty","contactRole":11,"displayInProduct":false}],
            "propertyContacts":[{"id":9004,"firstName":"Sam","lastName":"Manager","locationName":"Imaginary Property Mgmt","emailAddress":"sam@example.com","phoneNumber":"(000) 555-0200"}]}"#,
        // Off-market listing on 102: its broker is stale and must not show up.
        r#"{"id":7002,"listingStatus":3,"propertyId":102,
            "listingContacts":[{"firstName":"Old","lastName":"Broker","locationName":"Gone Realty","contactRole":11,"displayInProduct":true}],
            "propertyContacts":[]}"#,
    ];
    fs::write(dir.join("data/commercial-listing-summary.jsonl"), listings.map(one_line).join("\n")).unwrap();

    let companies = [
        r#"{"id":501,"name":"Fictional Architects","locationPhone":"(000) 555-0501"}"#,
        r#"{"id":502,"name":"Invented Owner LLC","locationPhone":"(000) 555-0502"}"#,
        r#"{"id":503,"name":"Imaginary Property Mgmt","locationPhone":"(000) 555-0503"}"#,
    ];
    fs::write(dir.join("data/company-detail.jsonl"), companies.join("\n")).unwrap();

    fs::write(
        dir.join("commercial-properties.csv"),
        "PropertyId,MatterportModelId,Address,City,State\n101,FakeModel01,100 Example St,Los Angeles,CA\n101,FakeModel02,100 Example St,Los Angeles,CA\n102,,200 Sample Ave,Los Angeles,CA\n",
    )
    .unwrap();
}

fn one_line(json: &str) -> String {
    serde_json::to_string(&serde_json::from_str::<serde_json::Value>(json).unwrap()).unwrap()
}

fn kit() -> (tempfile::TempDir, Kit) {
    let dir = tempfile::tempdir().unwrap();
    write_kit(dir.path());
    let kit = Kit::load(dir.path()).unwrap();
    (dir, kit)
}

#[test]
fn loads_buildings_with_coordinates_only() {
    let (_dir, kit) = kit();
    assert_eq!(kit.len(), 3);
    assert!(kit.building("costar:104").is_none());
}

#[test]
fn nearest_first_within_radius() {
    let (_dir, kit) = kit();
    let near = kit.buildings_near(LAT, LON, 500.0);
    let ids: Vec<&str> = near.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(ids, ["costar:101", "costar:102"]); // 103 is ~1.1 km away
    assert_eq!(near[0].dist_m, Some(0.0));
    let d = near[1].dist_m.unwrap();
    assert!((d - 111.2).abs() < 0.5, "{d}");

    assert_eq!(kit.buildings_near(LAT, LON, 2000.0).len(), 3);
    assert!(kit.buildings_near(LAT, LON, 50.0).len() == 1);
    assert!(kit.buildings_near(0.0, 0.0, 1000.0).is_empty());
}

#[test]
fn building_fields_follow_the_contract() {
    let (_dir, kit) = kit();
    let b = kit.building("costar:101").unwrap();
    assert_eq!(b.dist_m, None);
    assert_eq!(b.name.as_deref(), Some("Made Up Tower"));
    assert_eq!(b.address.as_deref(), Some("100 Example St, Los Angeles, CA 90013"));
    assert_eq!(b.height_m, Some(45.7)); // 150 ft x 0.3048
    assert_eq!(b.floors, Some(12));
    assert_eq!(b.roof_m, None);
    assert_eq!(b.property_type.as_deref(), Some("Office"));

    let ids: Vec<&str> = b.matterport.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["FakeModel01", "FakeModel02"]);
    assert_eq!(b.matterport[0].url, "https://my.matterport.com/show/?m=FakeModel01");

    let plain = kit.building("costar:102").unwrap();
    assert_eq!((plain.name, plain.height_m, plain.floors), (None, None, Some(4)));
    assert!(plain.matterport.is_empty());
}

#[test]
fn contacts_put_the_manager_first_and_skip_the_rest() {
    let (_dir, kit) = kit();
    let c = kit.building("costar:101").unwrap().contacts;
    let roles: Vec<ContactRole> = c.iter().map(|c| c.role).collect();
    // Manager (from the building), the active listing's broker, then the owner once (TrueOwner and
    // RecordedOwner are the same company). Architect skipped, hidden broker skipped.
    assert_eq!(roles, [ContactRole::Manager, ContactRole::Leasing, ContactRole::Owner]);

    let manager = &c[0];
    assert_eq!(manager.name.as_deref(), Some("Sam Manager")); // named via the listing's propertyContacts
    assert_eq!(manager.company.as_deref(), Some("Imaginary Property Mgmt"));
    assert_eq!(manager.phone.as_deref(), Some("(000) 555-0004")); // the building record's phone wins
    assert_eq!(manager.email.as_deref(), Some("sam@example.com"));

    let broker = &c[1];
    assert_eq!(broker.name.as_deref(), Some("Pat Broker"));
    assert_eq!(broker.company.as_deref(), Some("Pretend Realty"));

    let owner = &c[2];
    assert_eq!(owner.name, None);
    assert_eq!(owner.company.as_deref(), Some("Invented Owner LLC"));
    assert_eq!(owner.phone.as_deref(), Some("(000) 555-0502")); // falls back to the company's phone

    assert!(kit.building("costar:102").unwrap().contacts.is_empty(), "off-market brokers are dropped");
}

#[test]
fn serialises_as_the_contract_shape() {
    let (_dir, kit) = kit();
    let list = costar::BuildingList { buildings: kit.buildings_near(LAT, LON, 500.0) };
    let json = serde_json::to_value(&list).unwrap();
    assert_eq!(json["buildings"][0]["id"], "costar:101");
    assert_eq!(json["buildings"][0]["contacts"][0]["role"], "manager");
    let back: costar::BuildingList = serde_json::from_value(json).unwrap();
    assert_eq!(back, list);
}

#[test]
fn works_without_the_optional_files() {
    let dir = tempfile::tempdir().unwrap();
    write_kit(dir.path());
    fs::remove_file(dir.path().join("data/commercial-listing-summary.jsonl")).unwrap();
    fs::remove_file(dir.path().join("data/company-detail.jsonl")).unwrap();
    fs::remove_file(dir.path().join("commercial-properties.csv")).unwrap();
    let kit = Kit::load(dir.path()).unwrap();
    let b = kit.building("costar:101").unwrap();
    assert!(b.matterport.is_empty());
    // Manager still listed, by phone only (no name, no company without company-detail).
    assert_eq!(b.contacts.len(), 1);
    assert_eq!(b.contacts[0].phone.as_deref(), Some("(000) 555-0004"));
}

#[test]
fn clear_errors() {
    let empty = tempfile::tempdir().unwrap();
    let err = Kit::load(empty.path()).unwrap_err();
    assert!(matches!(err, Error::Io { .. }), "{err}");
    assert!(err.to_string().contains("commercial-property.jsonl"));

    let dir = tempfile::tempdir().unwrap();
    write_kit(dir.path());
    fs::write(dir.path().join("data/commercial-property.jsonl"), "{\"id\":1}\nnot json\n").unwrap();
    let err = Kit::load(dir.path()).unwrap_err();
    assert!(err.to_string().contains("line 1"), "{err}");

    assert!(Error::NoKitDir.to_string().contains("COSTAR_KIT_DIR"));
}
