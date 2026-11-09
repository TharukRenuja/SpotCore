const { readFileSync } = require('node:fs');
const { Session } = require('..');
async function main() {
  const spotify = new Session();
  try {
    const cookie = readFileSync(process.env.SPOTCORE_COOKIE_FILE, 'utf8').trim();
    await spotify.login(cookie, { appId: 'my-node-app', deviceName: 'My Music App', playback: false });
    const results = await spotify.searchTracks('Miles Davis', 10, 0);
    for (const track of results.items) console.log(`${track.name} — ${track.uri}`);
  } finally { await spotify.shutdown(); }
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
