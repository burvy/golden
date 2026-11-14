# Kinesiology's devlog

## 11/14/26: CoStar buildings (branch `costar/buildings`)

**The contract was already there.** `contracts::Building`, the "Building (CoStar)" section in
`docs/contracts.md` and `fixtures/buildings.json` were already merged on main, so no contract PR was needed.
The task said `distance_m` / `matterport_id`, but the contract (which the agent and call sheet already use)
says `dist_m` and a `matterport` list of `{id, url}`, so the crate follows the contract.

**What exists now: `crates/costar`**
- `costar::buildings_near(lat, lon, radius_m) -> Result<Vec<Building>, Error>`: nearest first, `dist_m` set.
  Loads the kit from `COSTAR_KIT_DIR` on the first call and keeps it (the whole kit loads in ~0.3 s in release).
- `Kit::load(dir)`, `kit.buildings_near(..)`, `kit.building("costar:<id>")` (for `/api/buildings/{id}`).
- Try it: `cargo run --release -p costar --example near -- 34.0490 -118.2510 300`.
- Not yet wired into the server: `/api/buildings` is the next small PR (in `crates/server`).

**Where each field comes from in the kit**
| Field | Kit source |
|---|---|
| `id` | `costar:` + `commercial-property.jsonl` `id` |
| `name`, `address` | `buildingName`; `primaryAddress` (ZIP+4 cut to 5 digits) |
| `lat`, `lon` | `primaryAddress.latitude/longitude` |
| `height_m` | `buildingHeightInFeet × 0.3048`, to 0.1 m. `null` when missing (decided: no guessing from floors) |
| `floors` | `numStoriesAboveGrade` (the kit has no `numFloors` on buildings) |
| `property_type` | `propertyType` code turned into a name (codes from the kit's `schemas/enums.json`) |
| `roof_m` | always `null` here: it comes from the Vexcel height map, not CoStar |
| `matterport` | `commercial-properties.csv` (PropertyId → MatterportModelId) |
| `contacts` | see below |

**Contacts: who to call to get on the roof** (decided: use both sources, manager first)
1. The building's own contacts (`commercial-property` `contacts[]`): property manager / building superintendent
   → `manager`, leasing company → `leasing`, owner → `owner`. Architects, developers, lenders are skipped.
   These records have no person, so the company name and phone come from `company-detail.jsonl` (`locationId`),
   and the person's name/email from a listing's `propertyContacts[]` with the same contact `id`, when there is one.
2. Brokers on the building's **active** listings (`listingContacts[]`) → `leasing`. Off-market listings and
   contacts with `displayInProduct: false` are skipped.
Sorted manager → leasing → owner → other, duplicates removed, at most 8 per building.

**Coverage on the real kit (LA, 1,581 buildings)**
- 1,098 have at least one contact (970 with a manager). Listings alone would give ~331, mostly brokers.
- Only 128 have a CoStar height, but 1,572 have floors and 1,554 have Matterport tours.
- Downtown (1 km around Pershing Square): 67 buildings, 59 with contacts, 6 with a CoStar height.

**Things to know**
- The kit's phone numbers and emails are **fake** (its docs say so). Fine for the demo; say so if asked.
- CoStar heights can be wrong: one 7-floor building has `buildingHeightInFeet` ≈ 10 ft (3 m). Trust `roof_m`
  from the height map for anything that matters; `height_m` is for display.
- The kit stays out of git. Tests build a tiny made-up kit in a temp folder (`crates/costar/tests/made_up_kit.rs`),
  every name/id/phone in it is invented. The `near` example prints real rows to your terminal only.
