using System.Windows;
using MarieLookApp.Services;

namespace MarieLookApp;

public partial class App : Application
{
    public static StorageService Storage { get; } = new();
    public static SupabaseService Supabase { get; } = new(Storage);
    public static SearchService Search { get; } = new();
    public static TextInsertionService TextInsertion { get; } = new();
    public static IpcService Ipc { get; } = new();

    public static bool OpenSearchOnStart { get; private set; }

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);

        bool hasSearchArg = e.Args.Contains("--search");

        // Prevent multiple instances
        var mutex = new System.Threading.Mutex(true, "MarieLookApp_SingleInstance", out bool createdNew);
        if (!createdNew)
        {
            // App already running - send command via IPC if --search was requested
            if (hasSearchArg)
            {
                IpcService.SendCommand("open-search");
            }
            Shutdown();
            return;
        }

        // Keep mutex alive
        GC.KeepAlive(mutex);

        // Start IPC server to receive commands from other instances
        Ipc.StartServer();

        // Flag to open search window on start if --search was passed
        OpenSearchOnStart = hasSearchArg;
    }

    protected override void OnExit(ExitEventArgs e)
    {
        Ipc.Dispose();
        base.OnExit(e);
    }
}
