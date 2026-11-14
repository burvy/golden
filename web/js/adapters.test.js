import test from 'node:test';
import assert from 'node:assert/strict';
import { briefFromForm, normalizeSun, normalizeHorizon, normalizeSearch, normalizeBuildings, classifyScene, decodeSceneRing } from './adapters.js';
import { createClient } from './api.js';

test('Brief matches strict official contract', () => {
 const b=briefFromForm({text:'sunset between two towers on rooftop',date:'2026-11-14',tz:'America/New_York',lat:40,lon:-74,radius_m:400});
 assert.deepEqual(Object.keys(b),['text','area','date','tz','event','standing','framing','max_spots']);
 assert.equal(b.framing,'between');
});
test('SunPath and HorizonProfile wrappers',()=>{
 assert.equal(normalizeSun({points:[{t:'2026-11-14T16:00:00-05:00',az_deg:250,elev_deg:4}]}).points.length,1);
 assert.equal(normalizeHorizon({points:[{az_deg:250,elev_deg:8}],sun:{hidden_at:'2026-11-14T16:00:00-05:00'}}).hidden_at,'2026-11-14T16:00:00-05:00');
});
test('SearchResult Spot sun and elevations',()=>{
 const r=normalizeSearch({spots:[{id:'a',lat:40,lon:-74,score:.9,ground_m:100,eye_m:101.7,sun:{hidden_at:'2026-11-14T16:00:00-05:00'},why:['tower gap']}],heat:{values:[.5]}});
 assert.equal(r.spots[0].height_m,100);assert.equal(r.spots[0].sun_visible_until,'2026-11-14T16:00:00-05:00');assert.equal(r.heat.values[0],.5);
});
test('BuildingList wrapper',()=>{
 const b=normalizeBuildings({buildings:[{name:'Example Tower',contacts:[{name:'Jane'}],matterport:[]}]});assert.equal(b[0].contact,'Jane');
});
test('SceneMeta u16le decimetre decode',()=>{
 const m=classifyScene({rings:[{width:2,height:1,cell_m:1,byte_offset:0}],data_url:'/api/scene/test.bin',base_m:10,nodata:65535});
 assert.equal(m.kind,'rings');const g=decodeSceneRing(new Uint8Array([20,0,255,255]),m.rings[0],m);assert.equal(g.heights[0],12);assert.ok(Number.isNaN(g.heights[1]));
});
test('client exposes agent independently of search',()=>{assert.equal(typeof createClient(()=> 'http://localhost:8787').agent,'function');});
