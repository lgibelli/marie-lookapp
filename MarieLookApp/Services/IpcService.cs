using System.IO.Pipes;

namespace MarieLookApp.Services;

public class IpcService : IDisposable
{
    private const string PipeName = "MarieLookApp_IPC";
    private CancellationTokenSource? _cts;
    private Task? _serverTask;
    private bool _disposed;

    public event Action? OpenSearchRequested;

    public void StartServer()
    {
        _cts = new CancellationTokenSource();
        _serverTask = Task.Run(() => ServerLoop(_cts.Token));
    }

    private async Task ServerLoop(CancellationToken token)
    {
        while (!token.IsCancellationRequested)
        {
            try
            {
                using var server = new NamedPipeServerStream(PipeName, PipeDirection.In, 1, PipeTransmissionMode.Byte, PipeOptions.Asynchronous);
                await server.WaitForConnectionAsync(token);

                using var reader = new StreamReader(server);
                var command = await reader.ReadLineAsync(token);

                if (command == "open-search")
                {
                    System.Windows.Application.Current?.Dispatcher.Invoke(() => OpenSearchRequested?.Invoke());
                }
            }
            catch (OperationCanceledException)
            {
                break;
            }
            catch
            {
                // Ignore connection errors, continue listening
            }
        }
    }

    public static bool SendCommand(string command)
    {
        try
        {
            using var client = new NamedPipeClientStream(".", PipeName, PipeDirection.Out);
            client.Connect(1000); // 1 second timeout

            using var writer = new StreamWriter(client) { AutoFlush = true };
            writer.WriteLine(command);
            return true;
        }
        catch
        {
            return false;
        }
    }

    public void Dispose()
    {
        if (_disposed) return;
        _disposed = true;

        _cts?.Cancel();
        _cts?.Dispose();
        _serverTask?.Wait(1000);
    }
}
