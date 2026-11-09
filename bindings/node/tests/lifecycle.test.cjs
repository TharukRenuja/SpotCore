const { test } = require('node:test');
const assert = require('node:assert/strict');
const { Session, SpotCoreError } = require('..');
const { NativeSession } = require('../generated/spotcore.node');

test('async errors, configuration and shutdown', async () => {
  const session = new Session();
  const code = expected => error => error instanceof SpotCoreError && error.code === expected;
  try {
    await Promise.all([
      assert.rejects(session.account(), code('not_authenticated')),
      assert.rejects(session.searchTracks('Miles Davis', 5, 0), code('not_authenticated')),
      assert.rejects(session.nextEvent(), code('not_authenticated')),
    ]);
    await assert.rejects(session.login('', { appId: 'node-test', deviceName: 'Binding test', playback: false }), code('invalid_input'));
    await assert.rejects(session.searchTracks('Miles Davis', -1, 0), code('invalid_input'));
  } finally { await session.shutdown(); }
  await assert.rejects(session.account(), code('closed'));
});

test('native protocol rejects oversized and malformed input', async () => {
  const native = new NativeSession();
  for (const request of ['not json', '{"op":"unknown"}']) {
    assert.equal(JSON.parse(await native.request(request)).error.code, 'invalid_input');
  }
  assert.equal(JSON.parse(await native.request('x'.repeat(1024 * 1024 + 1))).error.code, 'input_too_large');
  assert.equal(JSON.parse(await native.request('{"op":"shutdown"}')).ok, true);
});

test('wrapper preserves typed result fields and request naming', async () => {
  const fs = require('node:fs');
  const vm = require('node:vm');
  const module = { exports: {} };
  class FakeNativeSession {
    async request(json) {
      const request = JSON.parse(json);
      if (request.op === 'login') {
        assert.deepEqual(request.options, { app_id: 'test-app', device_name: 'Test', playback: false, data_dir: null });
        return JSON.stringify({ ok: true, result: { account: { username: 'user', name: 'Test', tier: 'free', country: null }, engine: 'services', fallback_reason: null } });
      }
      if (request.op === 'invoke') {
        assert.equal(request.operation, 'home.feed_info');
        assert.equal(request.arguments.items_per_shelf, 10);
        return JSON.stringify({ ok: true, result: { raw_snake_key: { original_field: 1 } } });
      }
      assert.equal(request.op, 'search_tracks');
      return JSON.stringify({ ok: true, result: { items: [{ uri: 'spotify:track:t', name: 'Song', image_urls: ['https://example.com/art'], duration_ms: 1234 }], total: 2, next_offset: 1 } });
    }
  }
  vm.runInNewContext(fs.readFileSync(require.resolve('../index.cjs'), 'utf8'), {
    module, require: () => ({ NativeSession: FakeNativeSession }),
  });
  const session = new module.exports.Session();
  const info = await session.login('test-cookie', { appId: 'test-app', deviceName: 'Test', playback: false });
  assert.equal(info.engine, 'services');
  assert.equal(info.fallbackReason, null);
  const page = await session.searchTracks('Song', 1, 0);
  assert.equal(page.nextOffset, 1);
  assert.equal(page.items[0].durationMs, 1234);
  assert.equal(page.items[0].imageUrls[0], 'https://example.com/art');
  const raw = await session.invoke('home.feed_info', { items_per_shelf: 10 });
  assert.equal(raw.raw_snake_key.original_field, 1);
  assert.equal(raw.rawSnakeKey, undefined);
});

test('all domain operations share the SDK worker', async () => {
  const session = new Session();
  try {
    const inventory = await session.operations();
    assert.ok(inventory.length >= 170);
    for (const name of ['library.add', 'playlists.set_cover', 'users.friend_activity', 'jam.invite_link', 'playback.set_quality']) {
      assert.ok(inventory.some(entry => entry.operation === name));
      await assert.rejects(session.invoke(name, {}), { code: 'not_authenticated' });
    }
    await assert.rejects(session.invoke('unknown.operation'), { code: 'invalid_input' });
  } finally { await session.shutdown(); }
});
