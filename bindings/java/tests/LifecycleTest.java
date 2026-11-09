import dev.spotcore.BindingError;
import dev.spotcore.Session;
import dev.spotcore.SessionOptions;
import java.util.Optional;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.function.Supplier;

public class LifecycleTest {
    static void expectError(Supplier<CompletableFuture<?>> action, String code) {
        try { action.get().join(); }
        catch (CompletionException error) {
            if (error.getCause() instanceof BindingError binding && code.equals(binding.code)) return;
            throw error;
        }
        throw new AssertionError("Expected " + code);
    }
    public static void main(String[] args) {
        try (Session session = new Session()) {
            if (!session.operations().contains("library.add")) throw new AssertionError("Missing operations");
            expectError(() -> session.invoke("library.contains", "{\"values\":[\"spotify:track:test\"]}"), "not_authenticated");
            expectError(session::account, "not_authenticated");
            expectError(() -> session.searchTracks("Miles Davis", 5, 0), "not_authenticated");
            expectError(session::nextEvent, "not_authenticated");
            expectError(() -> session.login("", new SessionOptions("java-test", "Binding test", false, Optional.empty()), Optional.empty()), "invalid_input");
            session.shutdown().join();
            if (!session.operations().contains("library.add")) throw new AssertionError("Missing operations");
            expectError(() -> session.invoke("library.contains", "{\"values\":[\"spotify:track:test\"]}"), "closed");
            expectError(session::account, "closed");
        }
        System.out.println("Java lifecycle checks passed");
    }
}
