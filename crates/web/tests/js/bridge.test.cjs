const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const path = require('node:path');
const source = fs.readFileSync(path.join(__dirname, '../../resources/player-bridge.js'), 'utf8');
const messages = [], listeners = {}, calls = [];
const state = { item: {uri:'spotify:ad:id', title:'Ad', type:'ad'}, restrictions:{canSeek:false}, isPaused:false };
const media = {aac: true, drm: true};
const services = {
  ShuffleAPI: {
    getAvailableShuffleModes: async uri => uri === 'spotify:playlist:test' ? [0, 1, 2] : [0, 1],
    setShuffle: async (uri, mode) => { calls.push({uri, mode}); },
  },
  PlayerAPI: {
    getState: () => state,
    getQueue: () => ({current:state.item, queued:[], nextUp:[]}),
    getEvents: () => ({addListener:(name, fn) => { listeners[name] = fn; }}),
    pause: async () => {calls.push('pause');},
  },
  PlayerSDK: {harmony: {
    setName: async name => {calls.push('name:' + name); return true;},
    _streamer: {
    setMediaPlaybackQuality: async quality => {calls.push('quality:' + quality.audio); return true;},
    _playerPromise: Promise.resolve({
      _currentContent: {_bitrate: 256000, _mimeType: 'audio/aac'},
      _preferredMediaPlaybackQuality: {audio: 'very_high'},
    }),
  }}},
  PlaybackAPI: {getVolume:()=>0.5, setVolume:async n=>calls.push(n)},
  LibraryAPI: {contains:async (...uris)=>uris.map(()=>true)},
  TokenProvider: {getToken:async()=>({accessToken:'secret',accessTokenExpirationTimestampMs:123,clientId:'client'})},
};
const registry = new Map(Object.keys(services).map(name=>[Symbol(name), services[name]]));
const platform = {version:'test', getRegistry:()=>({_map:registry, resolve:key=>registry.get(key)}),
  getGraphQLLoader:()=>async (def, variables)=>({data:{name:def.name, variables,track:{_uri:'spotify:track:test',data:{__typename:'Track',name:'Saved'}},_private:'hidden'}})};
const host = {querySelectorAll:()=>[], '__reactContainer$test':{memoizedProps:{value:platform}}};
const context = vm.createContext({location:{origin:'https://open.spotify.com'},
  document:{getElementById:()=>host, createElement:()=>({canPlayType:()=>media.aac ? "probably" : ""})},
  navigator:{requestMediaKeySystemAccess:async()=>{if(!media.drm)throw new Error("CDM incompatible");return {}; }}, window:{__mspHostPost:json=>messages.push(JSON.parse(json))},
  setTimeout:()=>{throw Error('Unexpected service lookup retry');}, console});
vm.runInContext(source, context);
(async () => {
  const bridge = context.window.Myspoti;
  assert.equal(messages[0].type, 'ready');
  assert.equal(messages[1].state.track.kind, 'ad');
  assert.equal(messages[1].state.restrictions.canSeek, false);
  await bridge.call(null, 'player.pause', {});
  assert.equal(calls[0], 'pause');
  assert.equal(await bridge.call(null, 'player.setAudioQuality', {level:'very_high'}), true);
  assert.equal(calls[1], 'quality:very_high');
  const streamer = services.PlayerSDK.harmony._streamer;
  const setQuality = streamer.setMediaPlaybackQuality;
  streamer.setMediaPlaybackQuality = async () => false;
  await assert.rejects(bridge.call(null, 'player.setAudioQuality', {level:'high'}), /rejected/);
  streamer.setMediaPlaybackQuality = setQuality;
  const info = await bridge.call(null, 'player.audioInfo', {});
  assert.equal(info.bitrate, 256000);
  assert.equal(info.mime, 'audio/aac');
  assert.equal(info.level, 'very_high');
  assert.equal(await bridge.call(null, 'player.setDeviceName', {name:'SpotCore'}), true);
  assert.equal(calls.at(-1), 'name:SpotCore');
  state.context = {uri:'spotify:playlist:test'};
  await bridge.call(null, 'player.setShuffleMode', {mode:2});
  assert.deepEqual(calls.at(-1), {uri:'spotify:playlist:test', mode:2});
  await bridge.call(null, 'player.setShuffleMode', {mode:1});
  assert.equal(calls.at(-1).mode, 1);
  await bridge.call(null, 'player.setShuffleMode', {mode:0});
  assert.equal(calls.at(-1).mode, 0);
  state.context = {uri:'spotify:playlist:restricted'};
  const shuffleCallCount = calls.length;
  await assert.rejects(bridge.call(null, 'player.setShuffleMode', {mode:2}), /unavailable/);
  assert.equal(calls.length, shuffleCallCount);
  state.context = null;
  await assert.rejects(bridge.call(null, 'player.setShuffleMode', {mode:2}), /context/);
  let drm = await bridge.call(null, 'player.drmSupport', {});
  assert.equal(drm.widevine, true);
  assert.equal(drm.aac, true);
  media.drm = false;
  drm = await bridge.call(null, 'player.drmSupport', {});
  assert.equal(drm.widevine, false);
  assert.match(drm.error, /incompatible/);
  media.drm = true; media.aac = false;
  assert.equal((await bridge.call(null, 'player.drmSupport', {})).aac, false);
  media.aac = true;
  const result = await bridge.call(null, 'gql.query', {name:'getTrack',hash:'a',variables:{uri:'spotify:track:id'}});
  assert.equal(result.data.name, 'getTrack');
  assert.equal(result.data.track._uri, 'spotify:track:test');
  assert.equal(result.data.track.data.__typename, 'Track');
  assert.equal(result.data._private, undefined);
  assert.equal(result.data.variables.uri, 'spotify:track:id');
  const token = await bridge.call(null, 'token.get', {});
  assert.equal(token.accessToken, 'secret');
  assert.equal(token.appVersion, 'test');
  await assert.rejects(bridge.call(null, 'service.call', {service:'PlayerAPI',method:'_private'}));
  await assert.rejects(bridge.call(null, 'unknown', {}));
  vm.runInContext('Date.now = () => 5000', context);
  Object.assign(state, {positionAsOfTimestamp:1000, timestamp:1000, duration:10000, speed:1.5, isPaused:false, isBuffering:false});
  let live = await bridge.call(null, 'player.state', {});
  assert.equal(live.positionMs, 7000);
  assert.equal(live.timestamp, 5000);
  state.duration = 4000;
  assert.equal((await bridge.call(null, 'player.state', {})).positionMs, 4000);
  state.duration = 10000;
  state.isPaused = true;
  assert.equal((await bridge.call(null, 'player.state', {})).positionMs, 1000);
  state.isPaused = false; state.isBuffering = true;
  assert.equal((await bridge.call(null, 'player.state', {})).positionMs, 1000);
  state.isBuffering = false; state.timestamp = 6000;
  assert.equal((await bridge.call(null, 'player.state', {})).positionMs, 1000);
  delete state.timestamp;
  assert.equal((await bridge.call(null, 'player.state', {})).positionMs, 1000);
  listeners.update();
  assert.equal(messages.at(-1).type, 'state');
  const count = messages.length;
  vm.runInContext(source, context);
  assert.equal(messages.length, count + 3);
  assert.throws(()=>vm.runInNewContext(source, {location:{origin:'https://example.com'}}), /Spotify origin/);
  console.log('bridge runtime checks passed');
})().catch(error=>{console.error(error); process.exitCode=1;});
