'use strict';
const { NativeSession } = require('./generated/spotcore.node');

class SpotCoreError extends Error {
  constructor(code, message) {
    super(message);
    this.name = 'SpotCoreError';
    this.code = code;
  }
}

function camelCase(value) {
  if (Array.isArray(value)) return value.map(camelCase);
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [
      key.replace(/_([a-z])/g, (_, letter) => letter.toUpperCase()), camelCase(item),
    ]));
  }
  return value;
}

class Session {
  #native = new NativeSession();
  async #call(op, fields = {}, normalize = true) {
    const response = JSON.parse(await this.#native.request(JSON.stringify({ op, ...fields })));
    if (!response.ok) throw new SpotCoreError(response.error.code, response.error.message);
    return normalize ? camelCase(response.result) : response.result;
  }
  login(cookie, options, nativeKey = null) {
    return this.#call('login', { cookie, options: {
      app_id: options.appId, device_name: options.deviceName,
      playback: options.playback, data_dir: options.dataDir ?? null,
    }, native_key: nativeKey });
  }
  invoke(operation, args = {}) { return this.#call('invoke', { operation, arguments: args }, false); }
  operations() { return this.#call('operations', {}, false); }
  account() { return this.#call('account'); }
  searchTracks(query, limit, offset) { return this.#call('search_tracks', { query, limit, offset }); }
  play(uri) { return this.#call('play', { uri }); }
  pause() { return this.#call('pause'); }
  nextEvent() { return this.#call('next_event'); }
  pollEvent() { return this.#call('poll_event'); }
  shutdown() { return this.#call('shutdown'); }
}
module.exports = { Session, SpotCoreError };
