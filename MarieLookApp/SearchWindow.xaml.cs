using System.Windows;
using System.Windows.Input;
using System.Windows.Threading;
using MarieLookApp.ViewModels;

namespace MarieLookApp;

public partial class SearchWindow : Window
{
    private readonly SearchViewModel _vm;

    public SearchWindow()
    {
        _vm = new SearchViewModel(App.Storage, App.Search, App.Supabase, App.TextInsertion);
        DataContext = _vm;

        InitializeComponent();

        _vm.InsertTextRequested += OnInsertTextRequested;
        Loaded += OnLoaded;
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        SearchBox.Focus();
        _vm.Initialize();
    }

    private void OnDeactivated(object? sender, EventArgs e)
    {
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
                ResultsList.Focus();
                _vm.MoveDownCommand.Execute(null);
                e.Handled = true;
                break;

            case Key.Up:
                ResultsList.Focus();
                _vm.MoveUpCommand.Execute(null);
                e.Handled = true;
                break;

            case Key.Enter:
                _vm.SelectItemCommand.Execute(null);
                e.Handled = true;
                break;
        }
    }

    private void OnResultKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key == Key.Enter)
        {
            _vm.SelectItemCommand.Execute(null);
            e.Handled = true;
        }
    }

    private void OnResultDoubleClick(object sender, MouseButtonEventArgs e)
    {
        _vm.SelectItemCommand.Execute(null);
    }

    private void OnInsertTextRequested(string text)
    {
        Deactivated -= OnDeactivated;
        Close();

        var timer = new DispatcherTimer(DispatcherPriority.Normal, Application.Current.Dispatcher)
        {
            Interval = TimeSpan.FromMilliseconds(100)
        };
        timer.Tick += (s, e) =>
        {
            timer.Stop();
            try
            {
                _vm.PerformTextInsertion(text);
            }
            catch (Exception ex)
            {
                System.Diagnostics.Debug.WriteLine($"Text insertion failed: {ex}");
            }
        };
        timer.Start();
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
