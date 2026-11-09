using SpotCore;

static async Task ExpectError(Func<Task> action, string code)
{
    try { await action(); }
    catch (BindingErrorException error) when (error.Error.Code == code) { return; }
    throw new Exception($"Expected {code}");
}

using var session = new Session();
if (!session.Operations().Contains("library.add")) throw new Exception("Missing operations");
await ExpectError(async () => { await session.Invoke("library.contains", "{\"values\":[\"spotify:track:test\"]}"); }, "not_authenticated");
await ExpectError(async () => { await session.Account(); }, "not_authenticated");
await ExpectError(async () => { await session.SearchTracks("Miles Davis", 5, 0); }, "not_authenticated");
await ExpectError(async () => { await session.NextEvent(); }, "not_authenticated");
await ExpectError(async () => {
    await session.Login("", new SessionOptions("dotnet-test", "Binding test", false, null), null);
}, "invalid_input");
await session.Shutdown();
await ExpectError(async () => { await session.Account(); }, "closed");
Console.WriteLine("C# lifecycle checks passed");
