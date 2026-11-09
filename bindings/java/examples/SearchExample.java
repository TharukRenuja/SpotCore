import dev.spotcore.Session;
import dev.spotcore.SessionOptions;
import java.util.Optional;

// Blocking CLI example; run off the UI thread in a desktop app.
public class SearchExample {
    public static void run(String cookie) {
        try (Session spotify = new Session()) {
            try {
                spotify.login(cookie, new SessionOptions("my-java-app", "My Music App", false, Optional.empty()), Optional.empty()).join();
                var results = spotify.searchTracks("Miles Davis", 10, 0).join();
                for (var track : results.items())
                    System.out.println(track.name() + " — " + track.uri());
            } finally { spotify.shutdown().join(); }
        }
    }
}
