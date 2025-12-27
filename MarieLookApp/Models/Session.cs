namespace MarieLookApp.Models;

public class Session
{
    public string AccessToken { get; set; } = string.Empty;
    public string RefreshToken { get; set; } = string.Empty;
    public long ExpiresAt { get; set; }
    public string Email { get; set; } = string.Empty;
}

public class AppData
{
    public Session? Session { get; set; }
    public List<Phrase> Phrases { get; set; } = new();
    public string? LastSyncTimestamp { get; set; }
}
