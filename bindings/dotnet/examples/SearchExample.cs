using SpotCore;

public static class SearchExample
{
    public static async Task Run(string cookie)
    {
        using var spotify = new Session();
        try
        {
            await spotify.Login(cookie, new SessionOptions("my-dotnet-app", "My Music App", false, null), null);
            var results = await spotify.SearchTracks("Miles Davis", 10, 0);
            foreach (var track in results.Items)
                Console.WriteLine($"{track.Name} — {track.Uri}");
        }
        finally { await spotify.Shutdown(); }
    }
}
