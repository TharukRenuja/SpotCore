import 'dart:io';
import 'package:spotcore_ffi/spotcore_ffi.dart';

Future<void> main() async {
  final cookie = File(Platform.environment['SPOTCORE_COOKIE_FILE']!).readAsStringSync().trim();
  final spotify = Session();
  try {
    await spotify.login(cookie, SessionOptions(
      appId: 'my-dart-app', deviceName: 'My Music App', playback: false, dataDir: null,
    ), null);
    final results = await spotify.searchTracks('Miles Davis', 10, 0);
    for (final track in results.items) { print('${track.name} — ${track.uri}'); }
  } finally {
    try {
      await spotify.shutdown();
    } finally {
      spotify.dispose$();
      shutdownBoltffi();
    }
  }
}
