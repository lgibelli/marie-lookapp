using System.Windows;
using MarieLookApp.Services;
using Drawing = System.Drawing;

namespace MarieLookApp;

public partial class MainWindow : Window
{
    private readonly HotkeyService _hotkeyService = new();
    private SearchWindow? _searchWindow;
    private LoginWindow? _loginWindow;

    public MainWindow()
    {
        InitializeComponent();

        // Create a simple tray icon at runtime
        TrayIcon.Icon = CreateTrayIcon();

        Loaded += OnLoaded;
        Closing += OnClosing;
    }

    private static Drawing.Icon CreateTrayIcon()
    {
        // Create a simple 16x16 icon with "M" letter
        using var bitmap = new Drawing.Bitmap(16, 16);
        using var g = Drawing.Graphics.FromImage(bitmap);

        // Blue background
        g.Clear(Drawing.Color.FromArgb(33, 150, 243));

        // White "M" letter
        using var font = new Drawing.Font("Arial", 10, Drawing.FontStyle.Bold);
        using var brush = new Drawing.SolidBrush(Drawing.Color.White);
        g.DrawString("M", font, brush, 1, 1);

        return Drawing.Icon.FromHandle(bitmap.GetHicon());
    }

    private async void OnLoaded(object sender, RoutedEventArgs e)
    {
        // Register global hotkey
        _hotkeyService.Register(this);
        _hotkeyService.HotkeyPressed += OnHotkeyPressed;

        // Check if user is authenticated
        if (!App.Supabase.IsAuthenticated)
        {
            ShowLoginWindow();
        }
        else
        {
            // Sync phrases on startup
            await SyncPhrasesAsync();
        }
    }

    private void OnClosing(object? sender, System.ComponentModel.CancelEventArgs e)
    {
        _hotkeyService.Dispose();
        TrayIcon.Dispose();
    }

    private void OnHotkeyPressed()
    {
        if (!App.Supabase.IsAuthenticated)
        {
            ShowLoginWindow();
            return;
        }

        ShowSearchWindow();
    }

    private void ShowSearchWindow()
    {
        if (_searchWindow != null && _searchWindow.IsVisible)
        {
            _searchWindow.Activate();
            return;
        }

        _searchWindow = new SearchWindow();
        _searchWindow.Closed += (s, e) => _searchWindow = null;
        _searchWindow.Show();
        _searchWindow.Activate();
    }

    private void ShowLoginWindow()
    {
        if (_loginWindow != null && _loginWindow.IsVisible)
        {
            _loginWindow.Activate();
            return;
        }

        _loginWindow = new LoginWindow();
        _loginWindow.LoginSuccessful += async () =>
        {
            await SyncPhrasesAsync();
        };
        _loginWindow.Closed += (s, e) => _loginWindow = null;
        _loginWindow.Show();
        _loginWindow.Activate();
    }

    private async Task SyncPhrasesAsync()
    {
        var result = await App.Supabase.FetchPhrasesAsync();
        if (!result.Success)
        {
            // Silent failure on auto-sync
            System.Diagnostics.Debug.WriteLine($"Sync failed: {result.Error}");
        }
    }

    private void OnSearchClick(object sender, RoutedEventArgs e)
    {
        if (!App.Supabase.IsAuthenticated)
        {
            ShowLoginWindow();
            return;
        }
        ShowSearchWindow();
    }

    private async void OnSyncClick(object sender, RoutedEventArgs e)
    {
        if (!App.Supabase.IsAuthenticated)
        {
            MessageBox.Show("Please log in first.", "Marie LookApp", MessageBoxButton.OK, MessageBoxImage.Information);
            return;
        }

        var result = await App.Supabase.FetchPhrasesAsync();
        if (result.Success)
        {
            MessageBox.Show($"Synced {result.Phrases?.Count ?? 0} phrases.", "Marie LookApp", MessageBoxButton.OK, MessageBoxImage.Information);
        }
        else
        {
            MessageBox.Show($"Sync failed: {result.Error}", "Marie LookApp", MessageBoxButton.OK, MessageBoxImage.Error);
        }
    }

    private void OnLogoutClick(object sender, RoutedEventArgs e)
    {
        var result = MessageBox.Show("Are you sure you want to logout?", "Marie LookApp", MessageBoxButton.YesNo, MessageBoxImage.Question);
        if (result == MessageBoxResult.Yes)
        {
            App.Supabase.Logout();
            ShowLoginWindow();
        }
    }

    private void OnExitClick(object sender, RoutedEventArgs e)
    {
        Application.Current.Shutdown();
    }
}
