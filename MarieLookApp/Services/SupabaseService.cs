using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using MarieLookApp.Models;

namespace MarieLookApp.Services;

public class SupabaseService
{
    private const string SupabaseUrl = "https://lvqswbqrztizfudlsfee.supabase.co";
    private const string SupabaseAnonKey = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6Imx2cXN3YnFyenRpemZ1ZGxzZmVlIiwicm9sZSI6ImFub24iLCJpYXQiOjE3NjUxMDkwMjMsImV4cCI6MjA4MDY4NTAyM30.H-li1x9wlsNU8CUh7tOw-LBCk9GsNZp22EyDngssJfQ";

    private readonly HttpClient _httpClient;
    private readonly StorageService _storage;
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        PropertyNameCaseInsensitive = true
    };

    public SupabaseService(StorageService storage)
    {
        _storage = storage;
        _httpClient = new HttpClient();
        _httpClient.DefaultRequestHeaders.Add("apikey", SupabaseAnonKey);
    }

    /// <summary>
    /// Send OTP to email for authentication
    /// </summary>
    public async Task<(bool Success, string? Error)> SendOtpAsync(string email)
    {
        try
        {
            var payload = new { email = email.Trim().ToLowerInvariant(), create_user = true };
            var content = new StringContent(
                JsonSerializer.Serialize(payload),
                Encoding.UTF8,
                "application/json"
            );

            var response = await _httpClient.PostAsync($"{SupabaseUrl}/auth/v1/otp", content);

            if (response.IsSuccessStatusCode)
            {
                return (true, null);
            }

            var errorBody = await response.Content.ReadAsStringAsync();
            return (false, $"Failed to send OTP: {errorBody}");
        }
        catch (Exception ex)
        {
            return (false, ex.Message);
        }
    }

    /// <summary>
    /// Verify OTP and get session
    /// </summary>
    public async Task<(bool Success, Session? Session, string? Error)> VerifyOtpAsync(string email, string token)
    {
        try
        {
            var payload = new
            {
                email = email.Trim().ToLowerInvariant(),
                token = token.Trim(),
                type = "email"
            };
            var content = new StringContent(
                JsonSerializer.Serialize(payload),
                Encoding.UTF8,
                "application/json"
            );

            var response = await _httpClient.PostAsync($"{SupabaseUrl}/auth/v1/verify", content);
            var responseBody = await response.Content.ReadAsStringAsync();

            if (!response.IsSuccessStatusCode)
            {
                return (false, null, $"Verification failed: {responseBody}");
            }

            var result = JsonSerializer.Deserialize<VerifyResponse>(responseBody, JsonOptions);
            if (result?.AccessToken == null)
            {
                return (false, null, "No session returned");
            }

            var session = new Session
            {
                AccessToken = result.AccessToken,
                RefreshToken = result.RefreshToken ?? "",
                ExpiresAt = result.ExpiresAt,
                Email = email.Trim().ToLowerInvariant()
            };

            _storage.SetSession(session);
            return (true, session, null);
        }
        catch (Exception ex)
        {
            return (false, null, ex.Message);
        }
    }

    /// <summary>
    /// Refresh session if needed
    /// </summary>
    public async Task<bool> RefreshSessionIfNeededAsync()
    {
        var session = _storage.GetSession();
        if (session == null) return false;

        // Check if token expires within 5 minutes
        var expiresAt = DateTimeOffset.FromUnixTimeSeconds(session.ExpiresAt);
        if (DateTimeOffset.UtcNow.AddMinutes(5) < expiresAt)
        {
            return true; // Still valid
        }

        // Try to refresh
        try
        {
            var payload = new { refresh_token = session.RefreshToken };
            var content = new StringContent(
                JsonSerializer.Serialize(payload),
                Encoding.UTF8,
                "application/json"
            );

            var response = await _httpClient.PostAsync($"{SupabaseUrl}/auth/v1/token?grant_type=refresh_token", content);
            if (!response.IsSuccessStatusCode)
            {
                _storage.SetSession(null);
                return false;
            }

            var responseBody = await response.Content.ReadAsStringAsync();
            var result = JsonSerializer.Deserialize<VerifyResponse>(responseBody, JsonOptions);
            if (result?.AccessToken == null)
            {
                _storage.SetSession(null);
                return false;
            }

            session.AccessToken = result.AccessToken;
            session.RefreshToken = result.RefreshToken ?? session.RefreshToken;
            session.ExpiresAt = result.ExpiresAt;
            _storage.SetSession(session);
            return true;
        }
        catch
        {
            _storage.SetSession(null);
            return false;
        }
    }

    /// <summary>
    /// Fetch all phrases from cloud journal
    /// </summary>
    public async Task<(bool Success, List<Phrase>? Phrases, string? Error)> FetchPhrasesAsync()
    {
        var session = _storage.GetSession();
        if (session == null)
        {
            return (false, null, "Not authenticated");
        }

        if (!await RefreshSessionIfNeededAsync())
        {
            return (false, null, "Session expired");
        }

        session = _storage.GetSession()!;

        try
        {
            var request = new HttpRequestMessage(HttpMethod.Get,
                $"{SupabaseUrl}/rest/v1/profile_journals?email=eq.{Uri.EscapeDataString(session.Email)}&order=created_at.asc");
            request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", session.AccessToken);
            request.Headers.Add("apikey", SupabaseAnonKey);

            var response = await _httpClient.SendAsync(request);
            var responseBody = await response.Content.ReadAsStringAsync();

            if (!response.IsSuccessStatusCode)
            {
                return (false, null, $"Failed to fetch phrases: {responseBody}");
            }

            var operations = JsonSerializer.Deserialize<List<JournalEntry>>(responseBody, JsonOptions);
            var phrases = ReplayOperations(operations ?? new List<JournalEntry>());

            _storage.SetPhrases(phrases);
            _storage.SetLastSyncTimestamp(DateTime.UtcNow.ToString("o"));

            return (true, phrases, null);
        }
        catch (Exception ex)
        {
            return (false, null, ex.Message);
        }
    }

    /// <summary>
    /// Replay journal operations to build phrase list
    /// </summary>
    private static List<Phrase> ReplayOperations(List<JournalEntry> operations)
    {
        var phraseMap = new Dictionary<string, Phrase>();

        foreach (var op in operations)
        {
            switch (op.Operation)
            {
                case "ADD_PHRASE":
                case "UPDATE_PHRASE":
                    if (op.Payload != null)
                    {
                        phraseMap[op.PhraseId] = new Phrase
                        {
                            Id = op.PhraseId,
                            Text = op.Payload.Text ?? "",
                            Tag = op.Payload.Tag,
                            CreatedAt = op.ClientTimestamp ?? DateTime.UtcNow
                        };
                    }
                    break;

                case "DELETE_PHRASE":
                    phraseMap.Remove(op.PhraseId);
                    break;
            }
        }

        return phraseMap.Values.ToList();
    }

    public bool IsAuthenticated => _storage.GetSession() != null;

    public string? GetEmail() => _storage.GetSession()?.Email;

    public void Logout()
    {
        _storage.ClearAll();
    }

    private class VerifyResponse
    {
        [JsonPropertyName("access_token")]
        public string? AccessToken { get; set; }

        [JsonPropertyName("refresh_token")]
        public string? RefreshToken { get; set; }

        [JsonPropertyName("expires_at")]
        public long ExpiresAt { get; set; }
    }

    private class JournalEntry
    {
        public string Operation { get; set; } = "";

        [JsonPropertyName("phrase_id")]
        public string PhraseId { get; set; } = "";

        public JournalPayload? Payload { get; set; }

        [JsonPropertyName("client_timestamp")]
        public DateTime? ClientTimestamp { get; set; }
    }

    private class JournalPayload
    {
        public string? Text { get; set; }
        public string? Tag { get; set; }
    }
}
