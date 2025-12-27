using System.IO;
using System.Text.Json;
using MarieLookApp.Models;

namespace MarieLookApp.Services;

public class StorageService
{
    private static readonly string AppDataPath = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        "MarieLookApp"
    );
    private static readonly string DataFilePath = Path.Combine(AppDataPath, "data.json");
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        WriteIndented = true,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase
    };

    private AppData _data = new();

    public StorageService()
    {
        EnsureDirectoryExists();
        Load();
    }

    private void EnsureDirectoryExists()
    {
        if (!Directory.Exists(AppDataPath))
        {
            Directory.CreateDirectory(AppDataPath);
        }
    }

    public void Load()
    {
        try
        {
            if (File.Exists(DataFilePath))
            {
                var json = File.ReadAllText(DataFilePath);
                _data = JsonSerializer.Deserialize<AppData>(json, JsonOptions) ?? new AppData();
            }
        }
        catch
        {
            _data = new AppData();
        }
    }

    public void Save()
    {
        try
        {
            var json = JsonSerializer.Serialize(_data, JsonOptions);
            File.WriteAllText(DataFilePath, json);
        }
        catch (Exception ex)
        {
            System.Diagnostics.Debug.WriteLine($"Failed to save data: {ex.Message}");
        }
    }

    public Session? GetSession() => _data.Session;

    public void SetSession(Session? session)
    {
        _data.Session = session;
        Save();
    }

    public List<Phrase> GetPhrases() => _data.Phrases;

    public void SetPhrases(List<Phrase> phrases)
    {
        _data.Phrases = phrases;
        Save();
    }

    public string? GetLastSyncTimestamp() => _data.LastSyncTimestamp;

    public void SetLastSyncTimestamp(string? timestamp)
    {
        _data.LastSyncTimestamp = timestamp;
        Save();
    }

    public void ClearAll()
    {
        _data = new AppData();
        Save();
    }
}
