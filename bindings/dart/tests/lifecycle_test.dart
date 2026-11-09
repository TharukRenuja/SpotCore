import 'package:spotcore_ffi/spotcore_ffi.dart';

Future<void> expectError(String code, Future<Object?> Function() action) async {
  try {
    await action();
  } on BindingError catch (error) {
    if (error.code != code) rethrow;
    return;
  }
  throw StateError('Expected $code');
}

Future<void> main() async {
  final session = Session();
  try {
    if (!session.operations().contains('library.add')) throw StateError('Missing operations');
    await expectError('not_authenticated', () => session.invoke('library.contains', '{"values":["spotify:track:test"]}'));
    await expectError('not_authenticated', session.account);
    await expectError('not_authenticated', () => session.searchTracks('Miles Davis', 5, 0));
    await expectError('not_authenticated', session.nextEvent);
    await expectError('invalid_input', () => session.login('', SessionOptions(
      appId: 'dart-test', deviceName: 'Binding test', playback: false, dataDir: null,
    ), null));
    await session.shutdown();
    await expectError('closed', session.account);
    print('Dart lifecycle checks passed');
  } finally {
    session.dispose$();
    shutdownBoltffi();
  }
}
