import Foundation

func searchExample(cookie: String) async throws {
    let spotify = try Session()
    do {
        _ = try await spotify.login(cookie: cookie, options: SessionOptions(
            appId: "my-swift-app", deviceName: "My Music App", playback: false, dataDir: nil
        ), nativeKey: nil)
        let results = try await spotify.searchTracks(query: "Miles Davis", limit: 10, offset: 0)
        for track in results.items { print("\(track.name) — \(track.uri)") }
    } catch {
        try? await spotify.shutdown()
        throw error
    }
    try await spotify.shutdown()
}
