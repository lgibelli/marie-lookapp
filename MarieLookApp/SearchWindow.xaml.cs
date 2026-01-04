using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Threading;
using MarieLookApp.Models;

namespace MarieLookApp;

public partial class SearchWindow : Window
{
    private readonly DispatcherTimer _debounceTimer;
    private List<Phrase> _currentResults = new();

    public SearchWindow()
    {
        InitializeComponent();

        _debounceTimer = new DispatcherTimer
        {
            Interval = TimeSpan.FromMilliseconds(150)
        };
        _debounceTimer.Tick += OnDebounceTimerTick;

        Loaded += OnLoaded;
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        SearchBox.Focus();
        UpdateStatus();
    }

    private void OnDeactivated(object sender, EventArgs e)
    {
        // Close when window loses focus
        Close();
    }

    private void OnKeyDown(object sender, KeyEventArgs e)
    {
        switch (e.Key)
        {
            case Key.Escape:
                Close();
                e.Handled = true;
                break;

            case Key.Down:
                if (ResultsList.Items.Count > 0)
                {
                    ResultsList.Focus();
                    if (ResultsList.SelectedIndex < 0)
                    {
                        ResultsList.SelectedIndex = 0;
                    }
                    else if (ResultsList.SelectedIndex < ResultsList.Items.Count - 1)
                    {
                        ResultsList.SelectedIndex++;
                    }
                }
                e.Handled = true;
                break;

            case Key.Up:
                if (ResultsList.Items.Count > 0 && ResultsList.SelectedIndex > 0)
                {
                    ResultsList.Focus();
                    ResultsList.SelectedIndex--;
                }
                e.Handled = true;
                break;

            case Key.Enter:
                SelectCurrentItem();
                e.Handled = true;
                break;
        }
    }

    private void OnResultKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key == Key.Enter)
        {
            SelectCurrentItem();
            e.Handled = true;
        }
    }

    private void OnSearchTextChanged(object sender, TextChangedEventArgs e)
    {
        _debounceTimer.Stop();
        _debounceTimer.Start();
    }

    private void OnDebounceTimerTick(object? sender, EventArgs e)
    {
        _debounceTimer.Stop();
        PerformSearch();
    }

    private void PerformSearch()
    {
        var query = SearchBox.Text.Trim();
        var phrases = App.Storage.GetPhrases();

        if (string.IsNullOrEmpty(query))
        {
            _currentResults = phrases.Take(20).ToList();
        }
        else
        {
            _currentResults = App.Search.Search(phrases, query, 20);
        }

        ResultsList.ItemsSource = _currentResults;
        UpdateStatus();

        if (_currentResults.Count > 0)
        {
            ResultsList.SelectedIndex = 0;
        }
    }

    private void UpdateStatus()
    {
        var phrases = App.Storage.GetPhrases();
        var email = App.Supabase.GetEmail() ?? "Not logged in";

        if (string.IsNullOrEmpty(SearchBox.Text))
        {
            StatusText.Text = $"{phrases.Count} phrases | {email}";
        }
        else
        {
            StatusText.Text = $"{_currentResults.Count} matches | {email}";
        }
    }

    private void OnResultDoubleClick(object sender, MouseButtonEventArgs e)
    {
        SelectCurrentItem();
    }

    private void SelectCurrentItem()
    {
        if (ResultsList.SelectedItem is Phrase phrase)
        {
            var textToInsert = phrase.Text;

            // Close window first so focus returns to previous app
            Close();

            // Longer delay to ensure focus fully returns to target app
            Task.Delay(200).ContinueWith(_ =>
            {
                Dispatcher.Invoke(() => App.TextInsertion.InsertText(textToInsert));
            });
        }
    }
}

// Converter for showing/hiding tag
public class NullToVisibilityConverter : System.Windows.Data.IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, System.Globalization.CultureInfo culture)
    {
        return value == null || string.IsNullOrEmpty(value.ToString())
            ? Visibility.Collapsed
            : Visibility.Visible;
    }

    public object ConvertBack(object value, Type targetType, object parameter, System.Globalization.CultureInfo culture)
    {
        throw new NotImplementedException();
    }
}
