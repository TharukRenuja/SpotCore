import dev.spotcore.BindingError
import dev.spotcore.Session
import dev.spotcore.SessionOptions
import kotlinx.coroutines.runBlocking

suspend fun expectError(code: String, action: suspend () -> Unit) {
    try { action() }
    catch (error: BindingError) {
        check(error.code == code) { "Expected $code, got ${error.code}" }
        return
    }
    error("Expected $code")
}

fun main() = runBlocking {
    Session().use { session ->
        check(session.operations().contains("library.add"))
        expectError("not_authenticated") { session.invoke("library.contains", """{"values":["spotify:track:test"]}""") }
        expectError("not_authenticated") { session.account() }
        expectError("not_authenticated") { session.searchTracks("Miles Davis", 5u, 0u) }
        expectError("not_authenticated") { session.nextEvent() }
        expectError("invalid_input") {
            session.login("", SessionOptions("kotlin-test", "Binding test", false, null), null)
        }
        session.shutdown()
        expectError("closed") { session.account() }
    }
    println("Kotlin lifecycle checks passed")
}
