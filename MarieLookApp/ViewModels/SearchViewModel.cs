using System.Collections.ObjectModel;
using System.Windows;
using System.Windows.Input;
using System.Windows.Threading;
using MarieLookApp.Models;
using MarieLookApp.Services;

namespace MarieLookApp.ViewModels;

public class SearchViewModel : ViewModelBase
{
    private readonly StorageService _storage;
    private readonly SearchService _search;
    private readonly SupabaseService _supabase;
    private readonly TextInsertionService _textInsertion;
    private readonly DispatcherTimer _debounceTimer;

    private string _searchQuery = string.Empty;
    private int _selectedIndex = -1;
    private string _statusText = "Type to search...";

    public SearchViewModel(
        StorageService storage,
        SearchService search,
        SupabaseService supabase,
        TextInsertionService textInsertion)
    {
        _storage = storage;
        _search = search;
        _supabase = supabase;
        _textInsertion = textInsertion;

        _debounceTimer = new DispatcherTimer(DispatcherPriority.Normal, Application.Current.Dispatcher)
        {
            Interval = TimeSpan.FromMilliseconds(150)
        };
        _debounceTimer.Tick += OnDebounceTimerTick;

        MoveDownCommand = new RelayCommand(MoveDown);
        MoveUpCommand = new RelayCommand(MoveUp);
        SelectItemCommand = new RelayCommand(SelectItem);
    }

    public string SearchQuery
    {
        get => _searchQuery;
        set
        {
            if (SetProperty(ref _searchQuery, value))
            {
                _debounceTimer.Stop();
                _debounceTimer.Start();
            }
        }
    }

    public ObservableCollection<Phrase> Results { get; } = new();

    public int SelectedIndex
    {
        get => _selectedIndex;
        set => SetProperty(ref _selectedIndex, value);
    }

    public string StatusText
    {
        get => _statusText;
        set => SetProperty(ref _statusText, value);
    }

    public ICommand MoveDownCommand { get; }
    public ICommand MoveUpCommand { get; }
    public ICommand SelectItemCommand { get; }

    public event Action<string>? InsertTextRequested;

    public void Initialize()
    {
        PerformSearch();
    }

    public void PerformTextInsertion(string text)
    {
        _textInsertion.InsertText(text);
    }

    private void OnDebounceTimerTick(object? sender, EventArgs e)
    {
        _debounceTimer.Stop();
        PerformSearch();
    }

    private void PerformSearch()
    {
        var query = SearchQuery.Trim();
        var phrases = _storage.GetPhrases();

        List<Phrase> results;
        if (string.IsNullOrEmpty(query))
        {
            results = phrases.Take(20).ToList();
        }
        else
        {
            results = _search.Search(phrases, query, 20);
        }

        Results.Clear();
        foreach (var phrase in results)
        {
            Results.Add(phrase);
        }

        UpdateStatus();

        if (Results.Count > 0)
        {
            SelectedIndex = 0;
        }
    }

    private void UpdateStatus()
    {
        var phrases = _storage.GetPhrases();
        var email = _supabase.GetEmail() ?? "Not logged in";

        if (string.IsNullOrEmpty(SearchQuery))
        {
            StatusText = $"{phrases.Count} phrases | {email}";
        }
        else
        {
            StatusText = $"{Results.Count} matches | {email}";
        }
    }

    private void MoveDown()
    {
        if (Results.Count > 0)
        {
            if (SelectedIndex < 0)
                SelectedIndex = 0;
            else if (SelectedIndex < Results.Count - 1)
                SelectedIndex++;
        }
    }

    private void MoveUp()
    {
        if (Results.Count > 0 && SelectedIndex > 0)
        {
            SelectedIndex--;
        }
    }

    private void SelectItem()
    {
        if (SelectedIndex >= 0 && SelectedIndex < Results.Count)
        {
            var phrase = Results[SelectedIndex];
            InsertTextRequested?.Invoke(phrase.Text);
        }
    }
}
