using System.Windows;
using System.Windows.Input;
using MarieLookApp.Services;

namespace MarieLookApp.ViewModels;

public class MainViewModel : ViewModelBase
{
    private readonly SupabaseService _supabase;
    private readonly UpdateService _updateService;

    public MainViewModel(SupabaseService supabase, UpdateService updateService)
    {
        _supabase = supabase;
        _updateService = updateService;

        SyncCommand = new AsyncRelayCommand(SyncAsync);
        CheckUpdateCommand = new AsyncRelayCommand(CheckUpdateAsync);
        LogoutCommand = new RelayCommand(Logout);
        ExitCommand = new RelayCommand(() => Application.Current.Shutdown());
    }

    public bool IsAuthenticated => _supabase.IsAuthenticated;

    public ICommand SyncCommand { get; }
    public ICommand CheckUpdateCommand { get; }
    public ICommand LogoutCommand { get; }
    public ICommand ExitCommand { get; }

    public event Action? ShowLoginRequested;

    public async Task SyncPhrasesAsync()
    {
        var result = await _supabase.FetchPhrasesAsync();
        if (!result.Success)
        {
            System.Diagnostics.Debug.WriteLine($"Sync failed: {result.Error}");
        }
    }

    public async Task<UpdateCheckResult> CheckForUpdateSilentAsync()
    {
        return await _updateService.CheckForUpdateAsync();
    }

    private async Task SyncAsync()
    {
        if (!_supabase.IsAuthenticated)
        {
            MessageBox.Show("Please log in first.", "Marie LookApp", MessageBoxButton.OK, MessageBoxImage.Information);
            return;
        }

        var result = await _supabase.FetchPhrasesAsync();
        if (result.Success)
        {
            MessageBox.Show($"Synced {result.Phrases?.Count ?? 0} phrases.", "Marie LookApp", MessageBoxButton.OK, MessageBoxImage.Information);
        }
        else
        {
            MessageBox.Show($"Sync failed: {result.Error}", "Marie LookApp", MessageBoxButton.OK, MessageBoxImage.Error);
        }
    }

    private async Task CheckUpdateAsync()
    {
        var result = await _updateService.CheckForUpdateAsync();

        if (result.Error != null)
        {
            MessageBox.Show($"Could not check for updates: {result.Error}", "Marie LookApp", MessageBoxButton.OK, MessageBoxImage.Warning);
        }
        else if (result.UpdateAvailable)
        {
            var response = MessageBox.Show(
                $"Version {result.NewVersion} is available (you have {_updateService.CurrentVersion}).\n\nWould you like to download it now?",
                "Update Available",
                MessageBoxButton.YesNo,
                MessageBoxImage.Information);

            if (response == MessageBoxResult.Yes)
            {
                UpdateService.OpenReleasesPage();
            }
        }
        else
        {
            MessageBox.Show($"You're up to date! (v{_updateService.CurrentVersion})", "Marie LookApp", MessageBoxButton.OK, MessageBoxImage.Information);
        }
    }

    private void Logout()
    {
        var result = MessageBox.Show("Are you sure you want to logout?", "Marie LookApp", MessageBoxButton.YesNo, MessageBoxImage.Question);
        if (result == MessageBoxResult.Yes)
        {
            _supabase.Logout();
            ShowLoginRequested?.Invoke();
        }
    }
}
