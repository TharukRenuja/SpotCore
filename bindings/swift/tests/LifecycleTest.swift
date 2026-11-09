import Foundation

func expectError(_ code: String, action: () async throws -> Void) async throws {
    do { try await action() }
    catch let error as BindingError {
        guard error.code == code else { throw error }
        return
    }
    fatalError("Expected \(code)")
}

@main struct LifecycleTest {
    static func main() async throws {
        let session = try Session()
        guard session.operations().contains("library.add") else { fatalError("Missing operations") }
        try await expectError("not_authenticated") { _ = try await session.invoke(operation: "library.contains", argumentsJson: #"{"values":["spotify:track:test"]}"#) }
        try await expectError("not_authenticated") { _ = try await session.account() }
        try await expectError("not_authenticated") { _ = try await session.searchTracks(query: "Miles Davis", limit: 5, offset: 0) }
        try await expectError("not_authenticated") { _ = try await session.nextEvent() }
        try await expectError("invalid_input") {
            _ = try await session.login(cookie: "", options: SessionOptions(appId: "swift-test", deviceName: "Binding test", playback: false, dataDir: nil), nativeKey: nil)
        }
        try await session.shutdown()
        try await expectError("closed") { _ = try await session.account() }
        print("Swift lifecycle checks passed")
    }
}
