const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const path = require('node:path');
const messages = [], resolved = [];
const token = {getToken: async () => ({accessToken: 'test-token', accessTokenExpirationTimestampMs: 9999999999999, clientId: 'test'})};
const registry = new Map(['PlayerAPI', 'PlaybackAPI', 'LibraryAPI', 'TokenProvider'].map(name => [Symbol(name), name]));
const platform = {
  version: 'test',
  getRegistry: () => ({_map: registry, resolve: key => {
    const name = registry.get(key);
    resolved.push(name);
    if (name === 'PlayerAPI' || name === 'PlaybackAPI') throw Error('local player must remain lazy');
    return name === 'TokenProvider' ? token : {};
  }}),
  getGraphQLLoader: () => async (definition, variables) => ({data: {name: definition.name, variables}}),
};
const host = {querySelectorAll: () => [], '__reactContainer$test': {memoizedProps: {value: platform}}};
const context = vm.createContext({
  location: {origin: 'https://open.spotify.com'},
  document: {getElementById: () => host},
  window: {__spotcoreServiceOnly: true, __mspHostPost: value => messages.push(JSON.parse(value))},
  setTimeout: () => {throw Error('unexpected service lookup retry');},
  console,
});
vm.runInContext(fs.readFileSync(path.join(__dirname, '../../resources/player-bridge.js'), 'utf8'), context);
(async () => {
  assert.deepEqual(messages.map(message => message.type), ['ready']);
  const result = await context.window.Myspoti.call(null, 'gql.query', {name: 'profileAttributes', variables: {test: true}});
  assert.equal(result.data.name, 'profileAttributes');
  assert.equal(result.data.variables.test, true);
  assert(!resolved.includes('PlayerAPI'));
  assert(!resolved.includes('PlaybackAPI'));
  context.window.__spotcoreServiceOnly = false;
  assert.throws(() => vm.runInContext(fs.readFileSync(path.join(__dirname, '../../resources/player-bridge.js'), 'utf8'), context), /browser role changed/);
  console.log('service-only bridge checks passed');
})().catch(error => {console.error(error); process.exitCode = 1;});
