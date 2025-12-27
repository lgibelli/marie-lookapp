using System.Windows;
using MarieLookApp.Services;

namespace MarieLookApp;

public partial class App : Application
{
    public static StorageService Storage { get; } = new();
    public static SupabaseService Supabase { get; } = new(Storage);
    public static SearchService Search { get; } = new();
    public static TextInsertionService TextInsertion { get; } = new();

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);

        // Prevent multiple instances
        var mutex = new System.Threading.Mutex(true, "MarieLookApp_SingleInstance", out bool createdNew);
        if (!createdNew)
        {
            MessageBox.Show("Marie LookApp is already running.", "Marie LookApp", MessageBoxButton.OK, MessageBoxImage.Information);
            Shutdown();
            return;
        }

        // Keep mutex alive
        GC.KeepAlive(mutex);
    }
}
