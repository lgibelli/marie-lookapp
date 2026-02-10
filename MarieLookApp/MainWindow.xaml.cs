using System.Windows;
using MarieLookApp.Services;
using MarieLookApp.ViewModels;
using Drawing = System.Drawing;

namespace MarieLookApp;

public partial class MainWindow : Window
{
    private readonly HotkeyService _hotkeyService = new();
    private readonly MainViewModel _vm;
    private SearchWindow? _searchWindow;
    private LoginWindow? _loginWindow;

    public MainWindow()
    {
        _vm = new MainViewModel(App.Supabase, new UpdateService());
        _vm.ShowLoginRequested += ShowLoginWindow;

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
        var hotkeyRegistered = _hotkeyService.Register(this);
        _hotkeyService.HotkeyPressed += OnHotkeyPressed;

        if (!hotkeyRegistered)
        {
            TrayIcon.ShowBalloonTip(
                "Hotkey Failed",
                "Could not register Ctrl+Shift+L. Another app may be using it.",
                Hardcodet.Wpf.TaskbarNotification.BalloonIcon.Warning);

            MessageBox.Show(
                "Could not register the Ctrl+Shift+L hotkey.\n\n" +
                "Another application may already be using this shortcut.\n" +
                "You can still use Marie LookApp from the tray icon.",
                "Marie LookApp - Hotkey Registration Failed",
                MessageBoxButton.OK,
                MessageBoxImage.Warning);
        }

        // Subscribe to IPC events for context menu "open search" commands
        App.Ipc.OpenSearchRequested += OnHotkeyPressed;

        // Check if user is authenticated
        if (!_vm.IsAuthenticated)
        {
            ShowLoginWindow();
        }
        else
        {
            // Sync phrases on startup
            await _vm.SyncPhrasesAsync();

            // Open search window if started with --search flag
            if (App.OpenSearchOnStart)
            {
                ShowSearchWindow();
            }

            // Check for updates in the background
            _ = CheckForUpdatesSilentAsync();
        }
    }

    private async Task CheckForUpdatesSilentAsync()
    {
        var result = await _vm.CheckForUpdateSilentAsync();
        if (result.UpdateAvailable)
        {
            TrayIcon.ShowBalloonTip(
                "Update Available",
                $"Marie LookApp {result.NewVersion} is available. Click to download.",
                Hardcodet.Wpf.TaskbarNotification.BalloonIcon.Info);

            TrayIcon.TrayBalloonTipClicked += (s, e) => UpdateService.OpenReleasesPage();
        }
    }

    private void OnClosing(object? sender, System.ComponentModel.CancelEventArgs e)
    {
        _hotkeyService.Dispose();
        TrayIcon.Dispose();
    }

    private void OnHotkeyPressed()
    {
        if (!_vm.IsAuthenticated)
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

        // Capture target window BEFORE opening search so we know where to paste
        App.TextInsertion.CaptureTargetWindow();

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
            await _vm.SyncPhrasesAsync();
        };
        _loginWindow.Closed += (s, e) => _loginWindow = null;
        _loginWindow.Show();
        _loginWindow.Activate();
    }

    private void OnSearchClick(object sender, RoutedEventArgs e)
    {
        if (!_vm.IsAuthenticated)
        {
            ShowLoginWindow();
            return;
        }
        ShowSearchWindow();
    }

    private void OnSyncClick(object sender, RoutedEventArgs e)
    {
        _vm.SyncCommand.Execute(null);
    }

    private void OnCheckUpdateClick(object sender, RoutedEventArgs e)
    {
        _vm.CheckUpdateCommand.Execute(null);
    }

    private void OnLogoutClick(object sender, RoutedEventArgs e)
    {
        _vm.LogoutCommand.Execute(null);
    }

    private void OnExitClick(object sender, RoutedEventArgs e)
    {
        _vm.ExitCommand.Execute(null);
    }
}
