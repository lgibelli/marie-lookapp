using System.Diagnostics;
using System.Net.Http;
using System.Reflection;
using System.Text.Json;

namespace MarieLookApp.Services;

public class UpdateService
{
    private const string GitHubOwner = "lgibelli";
    private const string GitHubRepo = "marie-lookapp-releases";

    private static readonly HttpClient HttpClient = new()
    {
        Timeout = TimeSpan.FromSeconds(10)
    };

    static UpdateService()
    {
        HttpClient.DefaultRequestHeaders.Add("User-Agent", "MarieLookApp");
    }

    public string CurrentVersion => Assembly.GetExecutingAssembly().GetName().Version?.ToString(3) ?? "1.0.0";

    public async Task<UpdateCheckResult> CheckForUpdateAsync()
    {
        try
        {
            var apiUrl = $"https://api.github.com/repos/{GitHubOwner}/{GitHubRepo}/releases/latest";
            var response = await HttpClient.GetStringAsync(apiUrl);

            using var doc = JsonDocument.Parse(response);
            var root = doc.RootElement;

            var tagName = root.GetProperty("tag_name").GetString() ?? "";
            var latestVersion = tagName.TrimStart('v');

            if (IsNewerVersion(latestVersion, CurrentVersion))
            {
                return new UpdateCheckResult
                {
                    UpdateAvailable = true,
                    NewVersion = latestVersion
                };
            }

            return new UpdateCheckResult { UpdateAvailable = false };
        }
        catch (Exception ex)
        {
            return new UpdateCheckResult
            {
                UpdateAvailable = false,
                Error = ex.Message
            };
        }
    }

    private static bool IsNewerVersion(string latest, string current)
    {
        if (Version.TryParse(latest, out var latestVer) && Version.TryParse(current, out var currentVer))
        {
            return latestVer > currentVer;
        }
        return false;
    }

    public static void OpenReleasesPage()
    {
        var url = $"https://github.com/{GitHubOwner}/{GitHubRepo}/releases/latest";
        Process.Start(new ProcessStartInfo(url) { UseShellExecute = true });
    }
}

public class UpdateCheckResult
{
    public bool UpdateAvailable { get; set; }
    public string? NewVersion { get; set; }
    public string? Error { get; set; }
}
