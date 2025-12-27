using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;
using MarieLookApp.Models;

namespace MarieLookApp.Services;

public class SearchService
{
    /// <summary>
    /// Normalize text for searching (lowercase, remove accents)
    /// </summary>
    private static string Normalize(string text)
    {
        var normalized = text.ToLowerInvariant().Normalize(NormalizationForm.FormD);
        var sb = new StringBuilder();
        foreach (var c in normalized)
        {
            var category = CharUnicodeInfo.GetUnicodeCategory(c);
            if (category != UnicodeCategory.NonSpacingMark)
            {
                sb.Append(c);
            }
        }
        return sb.ToString().Normalize(NormalizationForm.FormC);
    }

    /// <summary>
    /// Calculate relevance score (higher = more relevant)
    /// </summary>
    private static int GetScore(Phrase phrase, string query)
    {
        var normalizedQuery = Normalize(query);
        var normalizedText = Normalize(phrase.Text);
        var normalizedTag = phrase.Tag != null ? Normalize(phrase.Tag) : "";

        int score = 0;

        // Exact tag match (highest priority)
        if (normalizedTag == normalizedQuery)
        {
            score += 100;
        }
        // Tag starts with query
        else if (normalizedTag.StartsWith(normalizedQuery))
        {
            score += 80;
        }
        // Tag contains query
        else if (normalizedTag.Contains(normalizedQuery))
        {
            score += 60;
        }

        // Text starts with query
        if (normalizedText.StartsWith(normalizedQuery))
        {
            score += 50;
        }
        // Text contains query as whole word
        else if (Regex.IsMatch(normalizedText, $@"\b{Regex.Escape(normalizedQuery)}\b"))
        {
            score += 40;
        }
        // Text contains query
        else if (normalizedText.Contains(normalizedQuery))
        {
            score += 20;
        }

        // Boost shorter phrases (more likely to be what user wants)
        if (score > 0 && phrase.Text.Length < 100)
        {
            score += 10;
        }

        return score;
    }

    /// <summary>
    /// Search phrases and return matches sorted by relevance
    /// </summary>
    public List<Phrase> Search(List<Phrase> phrases, string query, int limit = 20)
    {
        if (string.IsNullOrWhiteSpace(query))
        {
            return new List<Phrase>();
        }

        var results = new List<(Phrase phrase, int score)>();

        foreach (var phrase in phrases)
        {
            var score = GetScore(phrase, query);
            if (score > 0)
            {
                results.Add((phrase, score));
            }
        }

        return results
            .OrderByDescending(x => x.score)
            .Take(limit)
            .Select(x => x.phrase)
            .ToList();
    }
}
