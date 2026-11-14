// Prints NOAA reference sunsets used in crates/core/src/sun.rs tests.
// Usage: curl -sSL https://gml.noaa.gov/grad/solcalc/main.js -o main.js
//        sed -n 60,445p main.js > calc.js   (the calculation functions only)
//        node docs/ray/noaa_ref.js calc.js
const fs = require('fs');
eval(fs.readFileSync(process.argv[2], 'utf8'));
const cases = [
  ['LA downtown, summer solstice', 34.0522, -118.2437, 2026, 6, 21],
  ['LA downtown, winter solstice', 34.0522, -118.2437, 2026, 12, 21],
  ['LA downtown, March equinox', 34.0522, -118.2437, 2026, 3, 20],
  ['Houston, July 4', 29.7604, -95.3698, 2026, 7, 4],
  ['Greenwich, summer solstice', 51.4769, -0.0005, 2026, 6, 21],
  ['Sydney, June', -33.8688, 151.2093, 2026, 6, 21],
  ['Tromso, midsummer', 69.6492, 18.9553, 2026, 6, 21],
  ['Tromso, midwinter', 69.6492, 18.9553, 2026, 12, 21],
];
for (const [name, lat, lon, y, m, d] of cases) {
  const JD = getJD(y, m, d);
  const t1 = calcSunriseSetUTC(0, JD, lat, lon);
  const t2 = calcSunriseSetUTC(0, JD + t1 / 1440, lat, lon);
  if (isNaN(t2)) { console.log(name, 'NO SUNSET'); continue; }
  const az = calcSunriseSet(0, JD, lat, lon, 0).azimuth;
  const ms = Date.UTC(y, m - 1, d) + Math.round(t2 * 60) * 1000;
  console.log(`${name} | ${lat}, ${lon}, ${y}-${m}-${d} | ${new Date(ms).toISOString()} | az ${az.toFixed(4)}`);
}
