/*
 * Derived from Myspoti's player-bridge.js, by kota-zo and contributors.
 * https://github.com/kota-zo/myspoti — GPL-3.0-only, see ../../../LICENSE.
 * Adapted for SpotCore with optional minimal UI startup.
 */
(() => {
  if (location.origin !== 'https://open.spotify.com') throw new Error('Spotify origin required');
  const serviceOnly = window.__spotcoreServiceOnly === true;
  if (window.Myspoti && window.Myspoti.version) {
    if (!!window.Myspoti.serviceOnly !== serviceOnly) throw new Error('browser role changed; reload the page before reconnecting');
    window.Myspoti.announce(); return;
  }
  const VERSION = 1;
  const READY_TIMEOUT_MS = 60000;

  const post = msg => {
    const json = JSON.stringify(msg);
    try {
      if (window.MyspotiHost && typeof window.MyspotiHost.postMessage === 'function') return window.MyspotiHost.postMessage(json);
      if (typeof window.__mspHostPost === 'function') return window.__mspHostPost(json);
    } catch (e) { /* host gone: nothing to report to */ }
  };

  const findPlatform = () => {
    if (window.__spotcorePlatform) return window.__spotcorePlatform;
    const host = document.getElementById('main') || document.body;
    const roots = [host, ...host.querySelectorAll('*')];
    for (const el of roots) {
      const key = Object.keys(el).find(k => k.startsWith('__reactContainer$'));
      if (!key) continue;
      const stack = [el[key]];
      while (stack.length) {
        const f = stack.pop();
        const v = f.memoizedProps && f.memoizedProps.value;
        if (v && typeof v === 'object' && typeof v.getRegistry === 'function') return v;
        if (f.sibling) stack.push(f.sibling);
        if (f.child) stack.push(f.child);
      }
    }
    return null;
  };

  let services = null;
  const resolveServices = () => {
    const platform = findPlatform();
    if (!platform) return null;
    const registry = platform.getRegistry();
    const byName = {};
    for (const key of registry._map.keys()) {
      if (key && key.description) byName[key.description] = key;
    }
    const get = name => {
      if (!byName[name]) throw new Error(`service ${name} not found`);
      return registry.resolve(byName[name]);
    };
    let player = serviceOnly ? null : get('PlayerAPI');
    if (!serviceOnly && (!player || typeof player.getState !== 'function')) return null;
    return {
      platform,
      get player() { return player || (player = get('PlayerAPI')); },
      get playback() { return get('PlaybackAPI'); },
      library: get('LibraryAPI'),
      token: get('TokenProvider'),
      graphql: platform.getGraphQLLoader(),
      byName,
      resolve: name => get(name),
    };
  };

  const image = images => {
    if (!images || !images.length) return null;
    return [...images].sort((a, b) => (b.width || 0) - (a.width || 0))[0].url;
  };

  // Ads come in the same queue as tracks, with `type: "ad"` and a title instead of a name.
  // Some markets give them a track-like uri: the metadata still flags them.
  const isAd = item => item.type === 'ad' || /^spotify:ad:/.test(item.uri || '') ||
    item.provider === 'ad' || !!item.advertiser ||
    (item.metadata && (item.metadata.is_advertisement === 'true' || item.metadata.is_advertisement === true || !!item.metadata.ad_id));
  const kind = item => isAd(item) ? 'ad'
    : /^spotify:episode:/.test(item.uri) ? 'episode' : 'track';

  const track = item => item && {
    kind: kind(item),
    uri: item.uri,
    uid: item.uid || null,
    name: item.name || item.title || '',
    advertiser: item.advertiser || null,
    durationMs: item.duration ? item.duration.milliseconds : null,
    artists: (item.artists || []).map(a => ({ uri: a.uri, name: a.name })),
    album: item.album ? { uri: item.album.uri, name: item.album.name } : null,
    image: image(item.images || (item.album && item.album.images)),
    explicit: !!item.isExplicit,
    mediaType: item.mediaType || 'audio',
    provider: item.provider || null,
  };

  const lastReal = items => (items || []).filter(i => kind(i) !== 'ad');
  const mapState = s => {
    if (!s) return null;
    const now = Date.now();
    const timestamp = Number(s.timestamp);
    const speed = Number.isFinite(s.speed) && s.speed > 0 ? s.speed : 1;
    const elapsed = !s.isPaused && !s.isBuffering && Number.isFinite(timestamp) && timestamp > 0
      ? Math.max(0, now - timestamp) * speed : 0;
    const duration = Number(s.duration);
    const position = Math.max(0, Number(s.positionAsOfTimestamp) || 0) + elapsed;
    return {
      track: track(s.item),
      contextUri: s.context ? s.context.uri : null,
      contextName: s.context && s.context.metadata ? s.context.metadata.context_description || null : null,
      previous: track(lastReal(s.previousItems).slice(-1)[0]),
      next: track(lastReal(s.nextItems)[0]),
      isPaused: !!s.isPaused,
      isBuffering: !!s.isBuffering,
      positionMs: Math.floor(Math.min(Number.MAX_SAFE_INTEGER, Number.isFinite(duration) && duration > 0 ? Math.min(position, duration) : position)),
      timestamp: now,
      durationMs: s.duration,
      shuffle: !!s.shuffle,
      smartShuffle: !!s.smartShuffle,
      repeat: s.repeat, // 0 off, 1 context, 2 track
      speed: s.speed,
      restrictions: s.restrictions || {},
      playbackId: s.playbackId || null,
    };
  };

  const mapQueue = q => q && {
    current: track(q.current),
    queued: (q.queued || []).map(track),
    nextUp: (q.nextUp || []).map(track),
  };

  const methodsOf = obj => {
    const names = new Set();
    for (let p = obj; p && p !== Object.prototype; p = Object.getPrototypeOf(p)) {
      for (const n of Object.getOwnPropertyNames(p)) {
        if (n === 'constructor' || n.startsWith('_')) continue;
        try { if (typeof p[n] === 'function') names.add(n); } catch (e) { /* getter threw */ }
      }
    }
    return [...names].sort();
  };

  // Plain JSON copy of whatever a service returns: drops functions, breaks cycles,
  // turns Maps/Sets into objects/arrays and BigInts into strings.
  const toJson = (value, depth = 0, seen = new WeakSet()) => {
    if (value === null || value === undefined) return null;
    const t = typeof value;
    if (t === 'bigint') return value.toString();
    if (t === 'function' || t === 'symbol') return undefined;
    if (t !== 'object') return value;
    if (depth > 12 || seen.has(value)) return null;
    seen.add(value);
    if (value instanceof Date) return value.toISOString();
    if (value instanceof Map) value = Object.fromEntries(value);
    if (value instanceof Set) value = [...value];
    if (ArrayBuffer.isView(value)) return Array.from(value);
    if (Array.isArray(value)) return value.map(v => { const j = toJson(v, depth + 1, seen); return j === undefined ? null : j; });
    const out = {};
    for (const k of Object.keys(value)) {
      if (k.startsWith('_') && k !== '_uri' && k !== '__typename') continue;
      const j = toJson(value[k], depth + 1, seen);
      if (j !== undefined) out[k] = j;
    }
    return out;
  };

  const need = () => {
    if (!services) throw new Error('not ready');
    return services;
  };

  const playerMedia = async () => {
    let player = null, timer;
    try {
      const promise = need().resolve('PlayerSDK').harmony?._streamer?._playerPromise;
      if (promise) player = await Promise.race([promise, new Promise(resolve => { timer = setTimeout(() => resolve(null), 2000); })]);
    } catch (_) { /* diagnostics still include document media when the player fails */ }
    finally { clearTimeout(timer); }
    const media = new Set(document.querySelectorAll('audio,video'));
    if (player?._player && typeof player._player.volume === 'number') media.add(player._player);
    return [...media];
  };

  const commands = {
    'bridge.version': () => VERSION,
    'media.state': async () => (await playerMedia()).map(el => ({paused:el.paused,muted:el.muted,volume:el.volume,currentTime:el.currentTime,readyState:el.readyState,networkState:el.networkState,ended:el.ended,hasKeys:!!el.mediaKeys,errorCode:el.error?.code || null})),
    'player.runtime': () => ({minimalUi: !!window.__spotcorePlatform}),
    'player.setDeviceName': a => {
      const sdk = need().resolve('PlayerSDK');
      if (!sdk || !sdk.harmony || typeof sdk.harmony.setName !== 'function') throw new Error('device naming unavailable');
      return sdk.harmony.setName(String(a.name));
    },
    'player.state': () => mapState(need().player.getState()),
    'player.queue': () => mapQueue(need().player.getQueue()),
    // {contextUri, trackUri?, trackUid?, index?, positionMs?}
    'player.play': a => {
      const skipTo = {};
      if (a.trackUri) skipTo.uri = a.trackUri;
      if (a.trackUid) skipTo.uid = a.trackUid;
      if (Number.isInteger(a.index)) skipTo.index = a.index;
      const extra = {};
      if (Object.keys(skipTo).length) extra.skipTo = skipTo;
      if (a.positionMs) extra.seekTo = a.positionMs;
      // A given order (sorted playlist): the context plays as one page of those songs.
      const context = Array.isArray(a.order) && a.order.length
        ? { uri: a.contextUri, url: 'context://' + a.contextUri + '?order=' + Date.now(), pages: [{ items: a.order.map(uri => ({ uri })) }] }
        : { uri: a.contextUri };
      return need().player.play(context, {}, extra);
    },
    'player.pause': () => need().player.pause(),
    'player.resume': () => need().player.resume(),
    'player.seek': a => need().player.seekTo(a.positionMs),
    'player.next': () => need().player.skipToNext(),
    'player.previous': () => need().player.skipToPrevious(),
    'player.setShuffle': a => need().player.setShuffle(!!a.on),
    'player.drmSupport': async () => {
      const aac = !!document.createElement('audio').canPlayType('audio/mp4; codecs="mp4a.40.2"');
      try {
        if (typeof navigator.requestMediaKeySystemAccess !== 'function') throw new Error('Encrypted Media Extensions unavailable');
        await navigator.requestMediaKeySystemAccess('com.widevine.alpha', [{
          initDataTypes: ['cenc'], audioCapabilities: [{contentType: 'audio/mp4; codecs="mp4a.40.2"'}],
        }]);
        return {widevine: true, aac, error: null};
      } catch (error) {
        return {widevine: false, aac, error: String(error.message || error)};
      }
    },
    'player.setShuffleMode': async a => {
      if (![0, 1, 2].includes(a.mode)) throw new Error('invalid shuffle mode');
      const uri = need().player.getState()?.context?.uri;
      if (!uri) throw new Error('shuffle requires an active context');
      const shuffle = need().resolve('ShuffleAPI');
      const available = await shuffle.getAvailableShuffleModes(uri);
      if (!available.includes(a.mode)) throw new Error('shuffle mode unavailable for this context');
      return shuffle.setShuffle(uri, a.mode);
    },
    'player.setRepeat': a => need().player.setRepeat(a.mode),
    'player.setSpeed': a => need().player.setSpeed(a.speed),
    // Inspect the local streamer; Connect state does not expose its codec or bitrate.
    'player.audioInfo': async () => {
      const sdk = need().resolve('PlayerSDK');
      const streamer = sdk && sdk.harmony && sdk.harmony._streamer;
      const player = streamer && streamer._playerPromise ? await streamer._playerPromise : null;
      const content = player && player._currentContent;
      return {
        bitrate: content && content._bitrate || 0,
        mime: content && content._mimeType || '',
        level: player && player._preferredMediaPlaybackQuality && player._preferredMediaPlaybackQuality.audio || '',
      };
    },
    'player.setAudioQuality': async a => {
      const sdk = need().resolve('PlayerSDK');
      const harmony = sdk && sdk.harmony;
      // The page's own streamer first: the public entry refuses while Spotify Connect
      // calls another device the active one, yet this page still plays.
      const target = [harmony && harmony._streamer, harmony]
        .find(t => t && typeof t.setMediaPlaybackQuality === 'function');
      if (!target) throw new Error('no audio quality setting');
      const applied = await target.setMediaPlaybackQuality({ audio: a.level });
      if (!applied) throw new Error('audio quality setting was rejected');
      return true;
    },
    'queue.add': a => need().player.addToQueue(a.uris.map(uri => ({ uri }))),
    'queue.remove': a => need().player.removeFromQueue(a.items),
    'queue.clear': () => need().player.clearQueue(),
    // Jumps to a queued or upcoming item.
    'queue.skipTo': a => need().player.skipToNext({ uri: a.uri, uid: a.uid }),
    // {uris, before: {uri, uid}} : "play next" puts the tracks before the first queued item.
    'queue.insert': a => need().player.insertIntoQueue(a.uris.map(uri => ({ uri })), { before: a.before }),
    // {items: [{uri, uid}], before | after: {uri, uid}}
    'queue.reorder': a => need().player.reorderQueue(a.items, a.after ? { after: a.after } : { before: a.before }),

    'volume.get': () => {
      const pb = need().playback;
      return typeof pb._volume === 'number' ? pb._volume : (typeof pb.__volume === 'number' ? pb.__volume : 1.0);
    },
    'volume.set': a => {
      const pb = need().playback;
      try { pb.setVolume(a.level); } catch (e) { /* ignore */ }
      return typeof pb._volume === 'number' ? pb._volume : a.level;
    },

    'library.contains': a => need().library.contains(...a.uris),
    'library.add': a => need().library.add({ uris: a.uris }),
    'library.remove': a => need().library.remove({ uris: a.uris }),

    // Generic access to the page's services (PlaylistAPI, RootlistAPI, ShowAPI, ConnectAPI...).
    'service.list': () => {
      const out = {};
      for (const name of Object.keys(need().byName)) {
        let v;
        try { v = need().resolve(name); } catch (e) { continue; }
        if (v && typeof v === 'object') out[name] = methodsOf(v);
      }
      return out;
    },
    // {service, method, args[]}
    'service.call': async a => {
      const svc = need().resolve(a.service);
      if (!svc || typeof svc[a.method] !== 'function' || a.method.startsWith('_')) {
        throw new Error(`no method ${a.service}.${a.method}`);
      }
      return toJson(await svc[a.method](...(a.args || [])));
    },

    // A playlist's cover, as the page's "Edit details" sets it: the JPEG goes to the image
    // upload service, then its token to the playlist. No jpeg (null): the cover is removed.
    // {uri, jpeg: base64 | null}
    'playlist.setCover': async a => {
      const playlists = need().resolve('PlaylistAPI');
      let token = null;
      if (a.jpeg) {
        const bytes = Uint8Array.from(atob(a.jpeg), c => c.charCodeAt(0));
        token = await playlists.uploadImage(new Blob([bytes], { type: 'image/jpeg' }));
      }
      await playlists.updateDetails(a.uri, { imageUploadToken: token });
      return true;
    },

    // Any persisted GraphQL operation of the Web Player, sent by the page's own loader
    // (its auth, client token and retries). {name, hash, variables}
    'gql.query': async a => {
      const def = { name: a.name, operation: a.operation || 'query', sha256Hash: a.hash, value: null };
      return need().graphql(def, a.variables || {});
    },

    // Discovers live GraphQL operation hashes by scanning the Web Player's script chunks.
    'gql.discover': async () => {
      const scripts = Array.from(document.querySelectorAll('script[src]'))
        .map(s => s.src)
        .filter(src => src.includes('web-player.'));
      const mainSrc = scripts[0];
      if (!mainSrc) return {};

      const ops = {};
      const opRegex = /"([A-Za-z0-9_]+)"\s*,\s*"(query|mutation)"\s*,\s*"([a-f0-9]{64})"/g;
      const scanText = text => {
        let m;
        while ((m = opRegex.exec(text)) !== null) {
          ops[m[1]] = { kind: m[2], hash: m[3] };
        }
      };

      try {
        const mainText = await (await fetch(mainSrc)).text();
        scanText(mainText);

        const chunkMatch = mainText.match(/u\.u=e=>""\+\(\((\{[^}]+\})\)\[e\]\|\|e\)\+"\."\+\((\{[^}]+\})\)\[e\]\+"\.[j]s"/);
        if (chunkMatch) {
          const names = {};
          for (const m of chunkMatch[1].matchAll(/(\d+):"([^"]+)"/g)) names[m[1]] = m[2];
          const hashes = {};
          for (const m of chunkMatch[2].matchAll(/(\d+):"([^"]+)"/g)) hashes[m[1]] = m[2];

          const baseUrl = mainSrc.substring(0, mainSrc.lastIndexOf('/') + 1);
          const chunkUrls = [];
          for (const [id, h] of Object.entries(hashes)) {
            const n = names[id] || id;
            if (/^(xpui-routes|browse|home|playlist|artist|album|search)/.test(n)) {
              chunkUrls.push(`${baseUrl}${n}.${h}.js`);
            }
          }

          const results = await Promise.allSettled(chunkUrls.map(u => fetch(u).then(r => r.text())));
          for (const res of results) {
            if (res.status === 'fulfilled') scanText(res.value);
          }
        }
      } catch (e) { /* partial discovery is safe */ }

      return ops;
    },

    // Web token for the HTTP API, refreshed by the page itself.
    'token.get': async () => {
      const t = await need().token.getToken();
      // clientId and appVersion let the host mint its own client token for direct HTTP calls.
      return t && {
        accessToken: t.accessToken,
        expiresAtMs: t.accessTokenExpirationTimestampMs,
        clientId: t.clientId,
        appVersion: need().platform.version,
        isAnonymous: !!t.isAnonymous,
      };
    },
  };

  const call = async (id, method, args) => {
    const fn = commands[method];
    if (!fn) throw new Error('unknown bridge method ' + method);
    return toJson(await fn(args || {}));
  };
  const pushState = () => post({type: 'state', state: mapState(services.player.getState())});
  const pushQueue = () => post({type: 'queue', queue: mapQueue(services.player.getQueue())});
  const announce = () => { if (services) { post({type: 'ready', version: VERSION}); if (!serviceOnly) { pushState(); pushQueue(); } } };
  window.Myspoti = {version: VERSION, serviceOnly, call, announce, methods: Object.keys(commands)};
  const started = Date.now();
  const boot = () => {
    try { services = resolveServices(); } catch (_) { services = null; }
    if (services) {
      if (!serviceOnly) {
        const events = services.player.getEvents();
        events.addListener('update', pushState);
        events.addListener('queue_update', pushQueue);
      }
      announce();
    } else if (Date.now() - started > READY_TIMEOUT_MS) {
      post({type: 'error', error: 'player services not found; log in to Spotify Web Player'});
    } else { setTimeout(boot, 250); }
  };
  boot();
})();
