import dev.spotcore.Session
import dev.spotcore.SessionOptions

suspend fun searchExample(cookie: String) {
    Session().use { spotify ->
        try {
            spotify.login(cookie, SessionOptions("my-kotlin-app", "My Music App", false, null), null)
            val results = spotify.searchTracks("Miles Davis", 10u, 0u)
            results.items.forEach { println("${it.name} — ${it.uri}") }
        } finally { spotify.shutdown() }
    }
}
