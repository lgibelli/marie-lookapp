namespace MarieLookApp.Models;

public class Phrase
{
    public string Id { get; set; } = string.Empty;
    public string Text { get; set; } = string.Empty;
    public string? Tag { get; set; }
    public DateTime CreatedAt { get; set; }
    public DateTime? UpdatedAt { get; set; }
}
